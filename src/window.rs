use crate::vk_context;
use crate::vk_shared::{self, ImportedVkImage, SkiaGetProc, VulkanDeviceInfo, COLOR_SUBRESOURCE_RANGE};
use ash::vk::Handle;
use skia_safe::gpu::vk::GetProcOf;
use skia_safe::gpu::{self, DirectContext};
use skia_safe::{ColorType, Surface};
use std::ffi::CStr;
use std::sync::Arc;
use vulkano::command_buffer::pool::{
    CommandBufferAllocateInfo, CommandPool, CommandPoolAlloc, CommandPoolCreateFlags,
    CommandPoolCreateInfo,
};
use vulkano::device::physical::PhysicalDevice;
use vulkano::device::{Device, Queue};
use vulkano::format::Format;
use vulkano::image::{Image, ImageUsage};
use vulkano::instance::Instance;
use vulkano::memory::allocator::MemoryAllocator;
use vulkano::swapchain::{
    AcquireNextImageInfo, ColorSpace, CompositeAlpha, FullScreenExclusive, PresentMode,
    Swapchain, SwapchainCreateInfo,
};
use vulkano::sync::semaphore::{Semaphore, SemaphoreCreateInfo};
use vulkano::sync::Sharing;
use vulkano::VulkanObject;
use winit::dpi::LogicalSize;
use winit::window::{Window, WindowAttributes};
use winit_core::event_loop::ActiveEventLoop;
use winit_core::icon::{Icon, RgbaIcon};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Main environment holding the window, the Skia surface + context, and Vulkan state.
pub(crate) struct Env {
    pub(crate) surface: Surface,
    pub(crate) gr_context: DirectContext,
    pub(crate) window: Arc<Box<dyn Window>>,
    pub(crate) vk: VkState,
}

impl Env {
    /// Mark the swapchain as needing recreation on the next frame.
    pub fn invalidate_swapchain(&mut self) {
        self.vk.swapchain_valid = false;
    }

    /// Recreate the Skia surface after a window resize.
    pub fn recreate_surface(&mut self) -> Result<(), String> {
        self.vk.swapchain_valid = false;
        vk_prepare_swapchain(&mut self.vk, self.window.clone(), &mut self.gr_context)?;
        if !self.vk.skia_surfaces.is_empty() {
            self.surface = self.vk.skia_surfaces[0].clone();
        }
        Ok(())
    }

    /// Acquire the next swapchain image and point the Skia surface at it.
    /// Returns `false` if the swapchain was out-of-date and the frame should be
    /// skipped.
    pub fn acquire_frame(&mut self) -> Result<bool, String> {
        vk_acquire(&mut self.vk, self.window.clone(), &mut self.surface, &mut self.gr_context)
    }

    /// Blit an imported tab VkImage directly to the page region of the current
    /// swapchain image using `vkCmdBlitImage`, **after** Skia has already
    /// flushed the chrome.
    ///
    /// Call order per frame:
    ///   1. `acquire_frame()`          — acquires swapchain image, redirects Skia surface
    ///   2. Skia draws chrome UI       — via `surface.canvas()` + `painter.render()`
    ///   3. `gr_context.flush_and_submit()` — Skia submits GPU work
    ///   4. `blit_tab_then_present()`  — blits tab, waits image_available, signals render_finished, presents
    pub fn blit_tab_then_present(
        &mut self,
        tab_frame: Option<(&Arc<ImportedVkImage>, u32, u32, i64)>,
        chrome_px: i32,
    ) -> Result<(), String> {
        vk_blit_tab_then_present(&mut self.vk, self.window.clone(), tab_frame, chrome_px)
    }
}

/// Vulkan-specific state.
pub(crate) struct VkState {
    pub(crate) entry: ash::Entry,
    pub(crate) ash_instance: ash::Instance,
    pub(crate) ash_physical_device: ash::vk::PhysicalDevice,
    pub(crate) ash_device: ash::Device,
    pub(crate) ash_queue: ash::vk::Queue,
    pub(crate) queue_family_index: u32,

    // Keep the vulkano objects alive while ash wrappers are used by the renderer.
    pub(crate) instance: Arc<Instance>,
    pub(crate) device: Arc<Device>,
    pub(crate) physical_device: Arc<PhysicalDevice>,
    pub(crate) queue: Arc<Queue>,
    pub(crate) allocator: Arc<dyn MemoryAllocator>,
    pub(crate) surface: Arc<vulkano::swapchain::Surface>,

    // Swapchain
    vk_swapchain: Arc<Swapchain>,
    swapchain_images: Vec<Arc<Image>>,
    swapchain_format: Format,
    swapchain_extent: ash::vk::Extent2D,

    /// One pre-built Skia `Surface` per swapchain image.
    pub(crate) skia_surfaces: Vec<Surface>,
    pub(crate) swapchain_valid: bool,

    /// Index of the swapchain image acquired for the current frame.
    pub(crate) current_image_index: u32,

    /// Skia color type matching the swapchain format.
    pub(crate) color_type: ColorType,
    /// Skia/Vulkan format tag for `ImageInfo`.
    pub(crate) vk_format: skia_safe::gpu::vk::Format,

    // Synchronisation (single frame-in-flight)
    image_available_semaphore: Arc<Semaphore>,
    render_finished_semaphore: Arc<Semaphore>,

    // Persistent command pool + buffer for blit operations (reused each frame)
    blit_cmd_pool: Arc<CommandPool>,
    blit_cmd_buf: CommandPoolAlloc,

    /// Keep the submitted tab image alive until `in_flight_fence` signals.
    in_flight_tab_image: Option<Arc<ImportedVkImage>>,

    /// Imported external semaphores waited by the previous submit.
    /// Destroy only after `in_flight_fence` signals.
    deferred_wait_semaphores: Vec<ash::vk::Semaphore>,
}

impl VkState {
    /// Build a `VulkanDeviceInfo` that tab processes can use to attach to the
    /// same physical device and share VkImages with the parent.
    pub(crate) fn device_info(&self) -> VulkanDeviceInfo {
        let device_uuid = unsafe {
            crate::vk_shared::physical_device_uuid(&self.ash_instance, self.ash_physical_device)
        };
        VulkanDeviceInfo {
            device_uuid,
            queue_family_index: self.queue_family_index,
            image_format: self.swapchain_format as i32,
            parent_pid: std::process::id(),
        }
    }
}

impl Drop for VkState {
    fn drop(&mut self) {
        unsafe {
            self.ash_device.device_wait_idle().ok();
            for sem in self.deferred_wait_semaphores.drain(..) {
                self.ash_device.destroy_semaphore(sem, None);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Vulkan helpers
// ---------------------------------------------------------------------------

/// Choose the best swapchain format, preferring UNORM over SRGB so Skia's maths stays linear.
fn vk_pick_format(
    _instance: &Arc<Instance>,
    physical_device: Arc<PhysicalDevice>,
    surface: Arc<vulkano::swapchain::Surface>,
) -> (Format, skia_safe::gpu::vk::Format, ColorType) {
    let formats = unsafe {
        physical_device.surface_formats(&surface, Default::default())
            .expect("Failed to query surface formats")
    };

    let preferred = [
        Format::B8G8R8A8_UNORM,
        Format::R8G8B8A8_UNORM,
        Format::B8G8R8A8_SRGB,
        Format::R8G8B8A8_SRGB,
    ];
    for want in preferred {
        if formats.iter().any(|sf| sf.0 == want) {
            if let Some((vf, ct)) = vk_shared::vk_format_to_skia(want) {
                return (want, vf, ct);
            }
        }
    }
    for sf in &formats {
        if let Some((vf, ct)) = vk_shared::vk_format_to_skia(sf.0) {
            return (sf.0, vf, ct);
        }
    }
    panic!("No Skia-compatible Vulkan swapchain format found. Available: {formats:?}");
}

fn vk_error_is_out_of_date_message(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    lower.contains("outofdate") || lower.contains("out_of_date")
}

fn vk_error_is_surface_lost_message(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    lower.contains("surface_lost")
        || lower.contains("surface lost")
        || lower.contains("surface is no longer available")
        || lower.contains("a surface is no longer available")
}

fn vk_error_is_recoverable_message(msg: &str) -> bool {
    vk_error_is_out_of_date_message(msg) || vk_error_is_surface_lost_message(msg)
}

fn vk_error_is_not_ready_message(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    lower.contains("notready") || lower.contains("not_ready") || lower.contains("not yet ready")
}

/// Recreate the swapchain after a resize (sets `swapchain_valid = true`).
fn vk_prepare_swapchain(
    vk: &mut VkState,
    window: Arc<Box<dyn Window>>,
    gr_context: &mut DirectContext,
) -> Result<(), String> {
    let sz = window.surface_size();
    if sz.width == 0 || sz.height == 0 || vk.swapchain_valid {
        return Ok(());
    }

    unsafe {
        vk.ash_device.device_wait_idle().ok();

        let caps = match vk
            .physical_device
            .surface_capabilities(&vk.surface, Default::default())
        {
            Ok(caps) => caps,
            Err(e) => {
                let msg = format!("surface_capabilities: {e:?}");
                vk.swapchain_valid = false;
                if vk_error_is_surface_lost_message(&msg) {
                    vk_recreate_surface(vk, window.clone())?;
                }
                if vk_error_is_recoverable_message(&msg) {
                    return Ok(());
                }
                return Err(msg);
            }
        };

        let extent = if let Some(current_extent) = caps.current_extent {
            ash::vk::Extent2D {
                width: current_extent[0],
                height: current_extent[1],
            }
        } else {
            ash::vk::Extent2D {
                width: sz.width.max(caps.min_image_extent[0]).min(caps.max_image_extent[0]),
                height: sz.height.max(caps.min_image_extent[1]).min(caps.max_image_extent[1]),
            }
        };

        let min_image_count = caps
            .min_image_count
            .max(2)
            .min(caps.max_image_count.unwrap_or(u32::MAX));

        let swapchain_ci = SwapchainCreateInfo {
            flags: Default::default(),
            min_image_count,
            image_format: vk.swapchain_format,
            image_view_formats: Default::default(),
            image_color_space: ColorSpace::SrgbNonLinear,
            image_extent: [extent.width, extent.height],
            image_array_layers: 1,
            image_usage: ImageUsage::COLOR_ATTACHMENT | ImageUsage::TRANSFER_DST,
            image_sharing: Sharing::Exclusive,
            pre_transform: caps.current_transform,
            composite_alpha: CompositeAlpha::Opaque,
            present_mode: PresentMode::Fifo,
            present_modes: Default::default(),
            clipped: true,
            scaling_behavior: None,
            present_gravity: None,
            full_screen_exclusive: FullScreenExclusive::Default,
            win32_monitor: None,
            ..Default::default()
        };

        let (new_swapchain, new_images) = match vk.vk_swapchain.recreate(swapchain_ci.clone()) {
            Ok(sc) => sc,
            Err(e) => {
                let recreate_msg = format!("swapchain_recreate: {e:?}");
                vk.swapchain_valid = false;

                if vk_error_is_surface_lost_message(&recreate_msg) {
                    vk_recreate_surface(vk, window.clone())?;
                }

                match Swapchain::new(vk.device.clone(), vk.surface.clone(), swapchain_ci) {
                    Ok(sc) => sc,
                    Err(e2) => {
                        let new_msg = format!("swapchain_new: {e2:?}");
                        if vk_error_is_recoverable_message(&new_msg) {
                            return Ok(());
                        }
                        return Err(format!("{recreate_msg}; {new_msg}"));
                    }
                }
            }
        };

        vk.vk_swapchain = new_swapchain;
        vk.swapchain_extent = extent;
        vk.swapchain_images = new_images;

        vk.skia_surfaces.clear();
        vk.skia_surfaces = vk_build_surfaces(
            gr_context,
            &vk.swapchain_images,
            extent,
            vk.vk_format,
            vk.color_type,
        );
        vk.swapchain_valid = true;
    }

    Ok(())
}

/// Build a Skia `Surface` that renders into a specific Vulkan swapchain image.
fn vk_surface_for_image(
    gr_context: &mut DirectContext,
    image: ash::vk::Image,
    extent: ash::vk::Extent2D,
    vk_format: skia_safe::gpu::vk::Format,
    color_type: ColorType,
) -> Surface {
    let alloc = skia_safe::gpu::vk::Alloc::default();
    let image_info = unsafe {
        skia_safe::gpu::vk::ImageInfo::new(
            image.as_raw() as _,
            alloc,
            skia_safe::gpu::vk::ImageTiling::OPTIMAL,
            skia_safe::gpu::vk::ImageLayout::UNDEFINED,
            vk_format,
            1,
            None, None, None, None,
        )
    };

    let render_target = gpu::backend_render_targets::make_vk(
        (extent.width as i32, extent.height as i32),
        &image_info,
    );

    gpu::surfaces::wrap_backend_render_target(
        gr_context,
        &render_target,
        gpu::SurfaceOrigin::TopLeft,
        color_type,
        None,
        None,
    )
    .expect("Failed to wrap Vulkan backend render target")
}

/// Pre-allocate one Skia `Surface` for every swapchain image.
fn vk_build_surfaces(
    gr_context: &mut DirectContext,
    images: &[Arc<Image>],
    extent: ash::vk::Extent2D,
    vk_format: skia_safe::gpu::vk::Format,
    color_type: ColorType,
) -> Vec<Surface> {
    images
        .iter()
        .map(|img| vk_surface_for_image(gr_context, img.handle(), extent, vk_format, color_type))
        .collect()
}

/// Acquire the next swapchain image and redirect the Skia surface to it.
/// Returns `Ok(false)` if the frame should be skipped (out-of-date swapchain).
fn vk_acquire(
    vk: &mut VkState,
    window: Arc<Box<dyn Window>>,
    current_surface: &mut Surface,
    gr_context: &mut DirectContext,
) -> Result<bool, String> {
    if !vk.swapchain_valid {
        vk_prepare_swapchain(vk, window.clone(), gr_context)?;
    }
    if !vk.swapchain_valid || vk.skia_surfaces.is_empty() {
        return Ok(false);
    }

    // Let vulkano own queue synchronization and avoid manual fence lifecycle issues.
    vk.queue
        .with(|mut q| q.wait_idle())
        .map_err(|e| format!("queue_wait_idle: {:?}", e))?;

    unsafe {
        // Release previously submitted shared tab image after GPU completion.
        vk.in_flight_tab_image = None;

        // Previous submit is complete, so it is now safe to destroy per-frame
        // imported wait semaphores created for external tab sync.
        for sem in vk.deferred_wait_semaphores.drain(..) {
            vk.ash_device.destroy_semaphore(sem, None);
        }

        let acquired = match vk.vk_swapchain.acquire_next_image(&AcquireNextImageInfo {
            timeout: None,
            semaphore: Some(vk.image_available_semaphore.clone()),
            fence: None,
            ..Default::default()
        }) {
            Ok(result) => result,
            Err(err) => {
                let msg = format!("acquire_next_image: {err:?}");
                if vk_error_is_not_ready_message(&msg) {
                    return Ok(false);
                }
                if vk_error_is_out_of_date_message(&msg) {
                    vk.swapchain_valid = false;
                    return Ok(false);
                }
                if vk_error_is_surface_lost_message(&msg) {
                    vk.swapchain_valid = false;
                    vk_recreate_surface(vk, window.clone())?;
                    return Ok(false);
                }
                return Err(msg);
            }
        };

        if acquired.is_suboptimal {
            vk.swapchain_valid = false;
        }

        vk.current_image_index = acquired.image_index;
        *current_surface = vk.skia_surfaces[acquired.image_index as usize].clone();
    }

    Ok(true)
}

#[cfg(not(windows))]
unsafe fn import_wait_semaphore_for_frame(
    instance: &ash::Instance,
    device: &ash::Device,
    sem_handle: i64,
) -> Result<Option<ash::vk::Semaphore>, String> {
    if sem_handle <= 0 {
        return Ok(None);
    }

    let mut export_ci = ash::vk::ExportSemaphoreCreateInfo::default()
        .handle_types(ash::vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD);
    let sem_ci = ash::vk::SemaphoreCreateInfo::default().push_next(&mut export_ci);
    let sem = device
        .create_semaphore(&sem_ci, None)
        .map_err(|e| format!("vkCreateSemaphore (import wait): {:?}", e))?;

    let ext_sem_fd = ash::khr::external_semaphore_fd::Device::new(instance, device);
    let import_info = ash::vk::ImportSemaphoreFdInfoKHR::default()
        .semaphore(sem)
        .flags(ash::vk::SemaphoreImportFlags::TEMPORARY)
        .handle_type(ash::vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD)
        .fd(sem_handle as libc::c_int);

    if let Err(e) = ext_sem_fd.import_semaphore_fd(&import_info) {
        device.destroy_semaphore(sem, None);
        if e == ash::vk::Result::ERROR_INVALID_EXTERNAL_HANDLE {
            // Cross-process sync fd handoff can fail transiently; fall back to presenting without this wait.
            return Ok(None);
        }
        return Err(format!("vkImportSemaphoreFdKHR failed: {:?}", e));
    }

    Ok(Some(sem))
}

#[cfg(windows)]
unsafe fn import_wait_semaphore_for_frame(
    instance: &ash::Instance,
    device: &ash::Device,
    sem_handle: i64,
) -> Result<Option<ash::vk::Semaphore>, String> {
    if sem_handle == 0 {
        return Ok(None);
    }

    let mut export_ci = ash::vk::ExportSemaphoreCreateInfo::default()
        .handle_types(ash::vk::ExternalSemaphoreHandleTypeFlags::OPAQUE_WIN32);
    let sem_ci = ash::vk::SemaphoreCreateInfo::default().push_next(&mut export_ci);
    let sem = device
        .create_semaphore(&sem_ci, None)
        .map_err(|e| format!("vkCreateSemaphore (import wait): {:?}", e))?;

    let ext_sem_win32 = ash::khr::external_semaphore_win32::Device::new(instance, device);
    let mut import_info = ash::vk::ImportSemaphoreWin32HandleInfoKHR::default()
        .semaphore(sem)
        .flags(ash::vk::SemaphoreImportFlags::TEMPORARY)
        .handle_type(ash::vk::ExternalSemaphoreHandleTypeFlags::OPAQUE_WIN32)
        .handle(sem_handle as ash::vk::HANDLE);

    if let Err(e) = ext_sem_win32.import_semaphore_win32_handle(&mut import_info) {
        device.destroy_semaphore(sem, None);
        return Err(format!("vkImportSemaphoreWin32HandleKHR failed: {:?}", e));
    }

    Ok(Some(sem))
}

/// Flush Skia GPU work, optionally blit a tab frame into the page region of the
/// swapchain image, then present.
///
/// Uses the persistent blit command pool/buffer on VkState (no per-frame
/// allocation or blocking fence wait for cleanup).
fn vk_blit_tab_then_present(
    vk: &mut VkState,
    window: Arc<Box<dyn Window>>,
    tab_frame: Option<(&Arc<ImportedVkImage>, u32, u32, i64)>,
    chrome_px: i32,
) -> Result<(), String> {
    unsafe {
        let ash_device = &vk.ash_device;
        let swapchain_image = vk.swapchain_images[vk.current_image_index as usize].handle();
        let mut submitted_tab_image: Option<Arc<ImportedVkImage>> = None;

        // Reset the persistent command buffer for this frame.
        ash_device.reset_command_buffer(vk.blit_cmd_buf.handle(), ash::vk::CommandBufferResetFlags::empty())
            .map_err(|e| format!("vkResetCommandBuffer (blit-present): {:?}", e))?;

        let begin_info = ash::vk::CommandBufferBeginInfo::default()
            .flags(ash::vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        ash_device.begin_command_buffer(vk.blit_cmd_buf.handle(), &begin_info)
            .map_err(|e| format!("vkBeginCommandBuffer (blit-present): {:?}", e))?;

        let mut external_wait_semaphore = ash::vk::Semaphore::null();

        let sublayers = ash::vk::ImageSubresourceLayers {
            aspect_mask: ash::vk::ImageAspectFlags::COLOR,
            mip_level: 0,
            base_array_layer: 0,
            layer_count: 1,
        };

        if let Some((tab, tab_w, tab_h, sem_handle)) = tab_frame {
            submitted_tab_image = Some(tab.clone());

            if let Some(imported_wait) = import_wait_semaphore_for_frame(&vk.ash_instance, ash_device, sem_handle)? {
                external_wait_semaphore = imported_wait;
            }

            let sw = vk.swapchain_extent.width as i32;
            let sh = vk.swapchain_extent.height as i32;
            let dst_h = (sh - chrome_px).max(0);

            // Transition swapchain image: COLOR_ATTACHMENT_OPTIMAL -> TRANSFER_DST_OPTIMAL
            // Transition tab image:       GENERAL                 -> TRANSFER_SRC_OPTIMAL
            let barriers_pre = [
                ash::vk::ImageMemoryBarrier::default()
                    .src_access_mask(ash::vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
                    .dst_access_mask(ash::vk::AccessFlags::TRANSFER_WRITE)
                    .old_layout(ash::vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                    .new_layout(ash::vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                    .src_queue_family_index(ash::vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(ash::vk::QUEUE_FAMILY_IGNORED)
                    .image(swapchain_image)
                    .subresource_range(COLOR_SUBRESOURCE_RANGE),
                ash::vk::ImageMemoryBarrier::default()
                    .src_access_mask(ash::vk::AccessFlags::MEMORY_WRITE)
                    .dst_access_mask(ash::vk::AccessFlags::TRANSFER_READ)
                    .old_layout(ash::vk::ImageLayout::GENERAL)
                    .new_layout(ash::vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                    .src_queue_family_index(ash::vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(ash::vk::QUEUE_FAMILY_IGNORED)
                    .image(tab.image().handle())
                    .subresource_range(COLOR_SUBRESOURCE_RANGE),
            ];
            ash_device.cmd_pipeline_barrier(
                vk.blit_cmd_buf.handle(),
                ash::vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT | ash::vk::PipelineStageFlags::ALL_COMMANDS,
                ash::vk::PipelineStageFlags::TRANSFER,
                ash::vk::DependencyFlags::empty(),
                &[], &[], &barriers_pre,
            );

            // Blit tab → swapchain page region
            let blit = ash::vk::ImageBlit::default()
                .src_subresource(sublayers)
                .src_offsets([
                    ash::vk::Offset3D { x: 0, y: 0, z: 0 },
                    ash::vk::Offset3D { x: tab_w as i32, y: tab_h as i32, z: 1 },
                ])
                .dst_subresource(sublayers)
                .dst_offsets([
                    ash::vk::Offset3D { x: 0, y: chrome_px, z: 0 },
                    ash::vk::Offset3D { x: sw, y: chrome_px + dst_h, z: 1 },
                ]);
            ash_device.cmd_blit_image(
                vk.blit_cmd_buf.handle(),
                tab.image().handle(), ash::vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                swapchain_image, ash::vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &[blit],
                ash::vk::Filter::LINEAR,
            );

            // Transition back:
            //   swapchain: TRANSFER_DST → PRESENT_SRC_KHR
            //   tab image: TRANSFER_SRC → GENERAL (ready for next frame)
            let barriers_post = [
                ash::vk::ImageMemoryBarrier::default()
                    .src_access_mask(ash::vk::AccessFlags::TRANSFER_WRITE)
                    .dst_access_mask(ash::vk::AccessFlags::empty())
                    .old_layout(ash::vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                    .new_layout(ash::vk::ImageLayout::PRESENT_SRC_KHR)
                    .src_queue_family_index(ash::vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(ash::vk::QUEUE_FAMILY_IGNORED)
                    .image(swapchain_image)
                    .subresource_range(COLOR_SUBRESOURCE_RANGE),
                ash::vk::ImageMemoryBarrier::default()
                    .src_access_mask(ash::vk::AccessFlags::TRANSFER_READ)
                    .dst_access_mask(ash::vk::AccessFlags::empty())
                    .old_layout(ash::vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                    .new_layout(ash::vk::ImageLayout::GENERAL)
                    .src_queue_family_index(ash::vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(ash::vk::QUEUE_FAMILY_IGNORED)
                    .image(tab.image().handle())
                    .subresource_range(COLOR_SUBRESOURCE_RANGE),
            ];
            ash_device.cmd_pipeline_barrier(
                vk.blit_cmd_buf.handle(),
                ash::vk::PipelineStageFlags::TRANSFER,
                ash::vk::PipelineStageFlags::ALL_COMMANDS,
                ash::vk::DependencyFlags::empty(),
                &[], &[], &barriers_post,
            );
        } else {
            // Skia wrote the swapchain image as a color attachment this frame.
            // When no tab frame is blitted, we still must transition it for present.
            let to_present_barrier = [ash::vk::ImageMemoryBarrier::default()
                .src_access_mask(ash::vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
                .dst_access_mask(ash::vk::AccessFlags::empty())
                .old_layout(ash::vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .new_layout(ash::vk::ImageLayout::PRESENT_SRC_KHR)
                .src_queue_family_index(ash::vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(ash::vk::QUEUE_FAMILY_IGNORED)
                .image(swapchain_image)
                .subresource_range(COLOR_SUBRESOURCE_RANGE)];
            ash_device.cmd_pipeline_barrier(
                vk.blit_cmd_buf.handle(),
                ash::vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                ash::vk::PipelineStageFlags::ALL_COMMANDS,
                ash::vk::DependencyFlags::empty(),
                &[],
                &[],
                &to_present_barrier,
            );
        }

        ash_device.end_command_buffer(vk.blit_cmd_buf.handle())
            .map_err(|e| format!("vkEndCommandBuffer (blit-present): {:?}", e))?;

        // Submit: wait image_available (+ optional tab frame semaphore), signal render_finished.
        let mut wait_sems = vec![vk.image_available_semaphore.handle()];
        let mut wait_stages = vec![ash::vk::PipelineStageFlags::ALL_COMMANDS];
        if external_wait_semaphore != ash::vk::Semaphore::null() {
            wait_sems.push(external_wait_semaphore);
            wait_stages.push(ash::vk::PipelineStageFlags::ALL_COMMANDS);
        }
        let signal_sems = [vk.render_finished_semaphore.handle()];
        let cmd_bufs = [vk.blit_cmd_buf.handle()];
        let submit_info = ash::vk::SubmitInfo::default()
            .wait_semaphores(&wait_sems)
            .wait_dst_stage_mask(&wait_stages)
            .command_buffers(&cmd_bufs)
            .signal_semaphores(&signal_sems);

        if let Err(e) = ash_device.queue_submit(vk.ash_queue, &[submit_info], ash::vk::Fence::null()) {
            if external_wait_semaphore != ash::vk::Semaphore::null() {
                ash_device.destroy_semaphore(external_wait_semaphore, None);
            }
            return Err(format!("vkQueueSubmit (blit-present): {:?}", e));
        }

        if external_wait_semaphore != ash::vk::Semaphore::null() {
            vk.deferred_wait_semaphores.push(external_wait_semaphore);
        }
        vk.in_flight_tab_image = submitted_tab_image;

        // Present
        let swapchains = [vk.vk_swapchain.handle()];
        let image_indices = [vk.current_image_index];
        let present_info = ash::vk::PresentInfoKHR::default()
            .wait_semaphores(&signal_sems)
            .swapchains(&swapchains)
            .image_indices(&image_indices);

        let swapchain_fn = ash::khr::swapchain::Device::new(&vk.ash_instance, ash_device);
        match swapchain_fn.queue_present(vk.ash_queue, &present_info) {
            Ok(false) => {}
            Ok(true) | Err(ash::vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                vk.swapchain_valid = false;
            }
            Err(ash::vk::Result::ERROR_SURFACE_LOST_KHR) => {
                vk.swapchain_valid = false;
                vk_recreate_surface(vk, window.clone())?;
            }
            Err(e) => return Err(format!("queue_present: {:?}", e)),
        }
    }
    Ok(())
}

/// Recreate the Vulkan surface from the current native window handles.
fn vk_recreate_surface(vk: &mut VkState, window: Arc<Box<dyn Window>>) -> Result<(), String> {
    unsafe {
        vk.ash_device.device_wait_idle().ok();

        vk.swapchain_images.clear();
        vk.skia_surfaces.clear();

        let new_surface = vulkano::swapchain::Surface::from_window_ref(
            vk.instance.clone(),
            &*window,
        )
        .map_err(|e| format!("Surface::from_window_ref (recreate): {e:?}"))?;

        vk.surface = new_surface;
        vk.current_image_index = 0;
        vk.swapchain_valid = false;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Public window constructor
// ---------------------------------------------------------------------------

/// Create the main window using the Vulkan backend.
pub(crate) fn create_window_vk(el: &Box<&dyn ActiveEventLoop>) -> Env {
    let icon: Option<Icon> = {
        let icon_bytes = include_bytes!("../assets/com.ethanstokes.stokes-browser.png");
        image::load_from_memory(icon_bytes).ok().and_then(|img| {
            let rgba = img.to_rgba8();
            let (w, h) = rgba.dimensions();
            let rgba_icon = RgbaIcon::new(rgba.into_raw(), w, h);
            match rgba_icon {
                Ok(rgba_icon) => Some(Icon::from(rgba_icon)),
                Err(_) => None,
            }
        })
    };

    let win_attrs = WindowAttributes::default()
        .with_title("Stokes Browser")
        .with_min_surface_size(LogicalSize::new(500, crate::ui::BrowserUI::CHROME_HEIGHT as i32 + 1))
        .with_window_icon(icon);

    let window: Arc<Box<dyn winit_core::window::Window>> = Arc::new(
        el.create_window(win_attrs).expect("Failed to create window"),
    );

    let bootstrap = vk_context::create_parent_context(window.clone(), el)
        .unwrap_or_else(|e| panic!("Failed to initialize vulkano parent context: {e}"));

    let entry = bootstrap.ash_entry;
    let instance = bootstrap.ash_instance;
    let physical_device = bootstrap.physical_device;
    let ash_physical_device = bootstrap.ash_physical_device;
    let device = bootstrap.ash_device;
    let queue = bootstrap.queue;
    let queue_family_index = bootstrap.queue_family_index;
    let negotiated_api_version = bootstrap.negotiated_api_version;
    let vk_instance = bootstrap.instance_owner;
    let vk_device = bootstrap.device_owner;
    let vk_queue_owner = bootstrap.queue_owner;
    let vk_surface_owner = bootstrap
        .surface_owner
        .expect("Parent context must provide a window surface owner");

    let vk_allocator = Arc::new(vulkano::memory::allocator::StandardMemoryAllocator::new_default(vk_device.clone()));

    let (swapchain_format, skia_vk_format, color_type) =
        vk_pick_format(vk_surface_owner.as_ref().instance(), physical_device.clone(), vk_surface_owner.clone());

    let caps = physical_device
        .surface_capabilities(&vk_surface_owner, Default::default())
        .expect("Failed to query surface capabilities");

    let window_size = window.surface_size();
    let extent = if let Some(current_extent) = caps.current_extent {
        ash::vk::Extent2D {
            width: current_extent[0],
            height: current_extent[1],
        }
    } else {
        ash::vk::Extent2D {
            width: window_size.width.max(caps.min_image_extent[0]).min(caps.max_image_extent[0]),
            height: window_size.height.max(caps.min_image_extent[1]).min(caps.max_image_extent[1]),
        }
    };

    let min_image_count = caps.min_image_count.max(2).min(caps.max_image_count.unwrap_or(u32::MAX));

    let swapchain_ci = SwapchainCreateInfo {
        flags: Default::default(),
        min_image_count,
        image_format: swapchain_format,
        image_view_formats: Default::default(),
        image_color_space: ColorSpace::SrgbNonLinear,
        image_extent: [extent.width, extent.height],
        image_array_layers: 1,
        image_usage: ImageUsage::COLOR_ATTACHMENT | ImageUsage::TRANSFER_DST,
        image_sharing: Sharing::Exclusive,
        pre_transform: caps.current_transform,
        composite_alpha: CompositeAlpha::Opaque,
        present_mode: PresentMode::Fifo,
        present_modes: Default::default(),
        clipped: true,
        scaling_behavior: None,
        present_gravity: None,
        full_screen_exclusive: FullScreenExclusive::Default,
        win32_monitor: None,
        ..Default::default()
    };

    let (swapchain, swapchain_images) = Swapchain::new(
        vk_device.clone(),
        vk_surface_owner.clone(),
        swapchain_ci,
    ).expect("Failed to create swapchain");

    let image_available_semaphore = Arc::new(
        Semaphore::new(vk_device.clone(), SemaphoreCreateInfo::default())
            .expect("Failed to create image_available semaphore"),
    );
    let render_finished_semaphore = Arc::new(
        Semaphore::new(vk_device.clone(), SemaphoreCreateInfo::default())
            .expect("Failed to create render_finished semaphore"),
    );

    let blit_cmd_pool = Arc::new(
        CommandPool::new(
            vk_device.clone(),
            CommandPoolCreateInfo {
                flags: CommandPoolCreateFlags::RESET_COMMAND_BUFFER,
                queue_family_index,
                ..Default::default()
            },
        )
        .expect("Failed to create blit command pool"),
    );
    let blit_cmd_buf = blit_cmd_pool
        .allocate_command_buffers(CommandBufferAllocateInfo {
            level: vulkano::command_buffer::CommandBufferLevel::Primary,
            command_buffer_count: 1,
            ..Default::default()
        })
        .expect("Failed to allocate blit command buffer")
        .next()
        .expect("Expected one blit command buffer");

    let mut gr_context = {
        let get_proc = SkiaGetProc::new(&entry, &instance);
        let get_proc_fn = |of: GetProcOf| get_proc.resolve(of);

        let mut backend_context = unsafe {
            skia_safe::gpu::vk::BackendContext::new(
                instance.handle().as_raw() as _,
                ash_physical_device.as_raw() as _,
                device.handle().as_raw() as _,
                (queue.as_raw() as _, queue_family_index as usize),
                &get_proc_fn,
            )
        };
        backend_context.set_max_api_version(negotiated_api_version);

        skia_safe::gpu::direct_contexts::make_vulkan(&backend_context, None)
            .unwrap_or_else(|| {
                let props = unsafe { instance.get_physical_device_properties(ash_physical_device) };
                let driver_name = unsafe { CStr::from_ptr(props.device_name.as_ptr()) }
                    .to_string_lossy()
                    .into_owned();
                panic!(
                    "Failed to create Skia Vulkan DirectContext. GPU: {} (type {:?}), \
                     queue_family_index: {}, negotiated_api_version: 0x{:X}",
                    driver_name, props.device_type,
                    queue_family_index, negotiated_api_version,
                )
            })
    };

    let skia_surfaces = vk_build_surfaces(
        &mut gr_context,
        &swapchain_images,
        extent,
        skia_vk_format,
        color_type,
    );
    let initial_surface = skia_surfaces[0].clone();

    let vk = VkState {
        entry,
        ash_instance: instance,
        ash_physical_device,
        ash_device: device,
        ash_queue: queue,
        queue_family_index,
        instance: vk_instance,
        device: vk_device,
        physical_device,
        queue: vk_queue_owner,
        allocator: vk_allocator,
        surface: vk_surface_owner,
        vk_swapchain: swapchain,
        swapchain_images,
        swapchain_format,
        swapchain_extent: extent,
        skia_surfaces,
        swapchain_valid: true,
        current_image_index: 0,
        color_type,
        vk_format: skia_vk_format,
        image_available_semaphore,
        render_finished_semaphore,
        blit_cmd_pool,
        blit_cmd_buf,
        in_flight_tab_image: None,
        deferred_wait_semaphores: Vec::new(),
    };

    Env {
        surface: initial_surface,
        gr_context,
        window,
        vk,
    }
}
