use crate::vk_context;
use crate::vk_shared::{self, ImportedVkImage, SkiaGetProc, VulkanDeviceInfo, COLOR_SUBRESOURCE_RANGE};
use ash::vk::{Extent2D, Handle};
use skia_safe::gpu::vk::GetProcOf;
use skia_safe::gpu::{self, DirectContext};
use skia_safe::{ColorType, Surface};
use std::ffi::CStr;
use std::sync::Arc;
use vulkano::command_buffer::PrimaryCommandBufferAbstract;
use vulkano::device::{Device, Queue};
use vulkano::device::physical::PhysicalDevice;
use vulkano::format::Format;
use vulkano::image::ImageUsage;
use vulkano::instance::Instance;
use vulkano::memory::allocator::MemoryAllocator;
use vulkano::swapchain::{ColorSpace, CompositeAlpha, FullScreenExclusive, PresentMode, SurfaceTransform, Swapchain, SwapchainCreateInfo};
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
    /// Recreate the Skia surface after a window resize.
    pub fn recreate_surface(&mut self) {
        self.vk.swapchain_valid = false;
        vk_prepare_swapchain(&mut self.vk, self.window.clone(), &mut self.gr_context);
        if !self.vk.skia_surfaces.is_empty() {
            self.surface = self.vk.skia_surfaces[0].clone();
        }
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
        vk_blit_tab_then_present(&mut self.vk, &**self.window, tab_frame, chrome_px)
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

    // Surface
    surface_khr: ash::vk::SurfaceKHR,
    surface_fn: ash::khr::surface::Instance,

    // Swapchain
    swapchain_fn: ash::khr::swapchain::Device,
    swapchain: ash::vk::SwapchainKHR,
    vk_swapchain: Arc<Swapchain>,
    swapchain_images: Vec<ash::vk::Image>,
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
    image_available_semaphore: ash::vk::Semaphore,
    render_finished_semaphore: ash::vk::Semaphore,

    // Persistent command pool + buffer for blit operations (reused each frame)
    blit_cmd_pool: ash::vk::CommandPool,
    blit_cmd_buf: ash::vk::CommandBuffer,

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
            self.ash_device.destroy_command_pool(self.blit_cmd_pool, None);
            self.ash_device.destroy_semaphore(self.image_available_semaphore, None);
            self.ash_device.destroy_semaphore(self.render_finished_semaphore, None);
            self.swapchain_fn.destroy_swapchain(self.swapchain, None);
        }
    }
}

// ---------------------------------------------------------------------------
// Vulkan helpers
// ---------------------------------------------------------------------------

/// Choose the best swapchain format, preferring UNORM over SRGB so Skia's maths stays linear.
fn vk_pick_format(
    surface_fn: &Arc<Instance>,
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

/// Recreate the swapchain after a resize (sets `swapchain_valid = true`).
fn vk_prepare_swapchain(vk: &mut VkState, window: Arc<Box<dyn Window>>, gr_context: &mut DirectContext) {
    let sz = window.surface_size();
    if sz.width == 0 || sz.height == 0 || vk.swapchain_valid {
        return;
    }

    unsafe {
        vk.ash_device.device_wait_idle().ok();

        let caps = match vk
            .surface_fn
            .get_physical_device_surface_capabilities(vk.ash_physical_device, vk.surface_khr)
        {
            Ok(caps) => caps,
            Err(ash::vk::Result::ERROR_SURFACE_LOST_KHR) => {
                let _ = vk_recreate_surface(vk, window);
                return;
            }
            Err(e) => panic!("Failed to query surface capabilities: {e:?}"),
        };

        let extent = if caps.current_extent.width != u32::MAX {
            caps.current_extent
        } else {
            ash::vk::Extent2D {
                width: sz.width.max(caps.min_image_extent.width).min(caps.max_image_extent.width),
                height: sz.height.max(caps.min_image_extent.height).min(caps.max_image_extent.height),
            }
        };

        let min_image_count = caps.min_image_count.max(2).min(
            if caps.max_image_count > 0 { caps.max_image_count } else { u32::MAX }
        );

        let old_swapchain = vk.swapchain;

        let swapchain_ci = ash::vk::SwapchainCreateInfoKHR::default()
            .surface(vk.surface_khr)
            .min_image_count(min_image_count)
            .image_format(ash::vk::Format::from_raw(vk.swapchain_format as i32))
            .image_color_space(ash::vk::ColorSpaceKHR::SRGB_NONLINEAR)
            .image_extent(extent)
            .image_array_layers(1)
            .image_usage(ash::vk::ImageUsageFlags::COLOR_ATTACHMENT | ash::vk::ImageUsageFlags::TRANSFER_DST)
            .image_sharing_mode(ash::vk::SharingMode::EXCLUSIVE)
            .pre_transform(caps.current_transform)
            .composite_alpha(ash::vk::CompositeAlphaFlagsKHR::OPAQUE)
            .present_mode(ash::vk::PresentModeKHR::FIFO)
            .clipped(true)
            .old_swapchain(old_swapchain);

        let new_swapchain = match vk.swapchain_fn.create_swapchain(&swapchain_ci, None) {
            Ok(sc) => sc,
            Err(ash::vk::Result::ERROR_SURFACE_LOST_KHR) => {
                let _ = vk_recreate_surface(vk, window);
                return;
            }
            Err(ash::vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                vk.swapchain_valid = false;
                return;
            }
            Err(e) => panic!("Failed to recreate swapchain: {e:?}"),
        };

        if old_swapchain != ash::vk::SwapchainKHR::null() {
            vk.swapchain_fn.destroy_swapchain(old_swapchain, None);
        }

        vk.swapchain = new_swapchain;
        vk.swapchain_extent = extent;
        vk.swapchain_images = vk.swapchain_fn
            .get_swapchain_images(new_swapchain)
            .expect("Failed to get swapchain images");

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
    images: &[ash::vk::Image],
    extent: ash::vk::Extent2D,
    vk_format: skia_safe::gpu::vk::Format,
    color_type: ColorType,
) -> Vec<Surface> {
    images
        .iter()
        .map(|&img| vk_surface_for_image(gr_context, img, extent, vk_format, color_type))
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
        vk_prepare_swapchain(vk, window.clone(), gr_context);
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

        let (image_index, suboptimal) = match vk.swapchain_fn.acquire_next_image(
            vk.swapchain,
            u64::MAX,
            vk.image_available_semaphore,
            ash::vk::Fence::null(),
        ) {
            Ok(result) => result,
            Err(ash::vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                vk.swapchain_valid = false;
                return Ok(false);
            }
            Err(ash::vk::Result::ERROR_SURFACE_LOST_KHR) => {
                vk_recreate_surface(vk, window.clone())?;
                return Ok(false);
            }
            Err(e) => return Err(format!("acquire_next_image: {:?}", e)),
        };

        if suboptimal {
            vk.swapchain_valid = false;
        }

        vk.current_image_index = image_index;
        *current_surface = vk.skia_surfaces[image_index as usize].clone();
    }

    Ok(true)
}

#[cfg(not(windows))]
unsafe fn import_wait_semaphore_for_frame(
    instance: &ash::Instance,
    device: &ash::Device,
    sem_handle: i64,
) -> Result<Option<ash::vk::Semaphore>, String> {
    if sem_handle < 0 {
        return Ok(None);
    }

    let sem_ci = ash::vk::SemaphoreCreateInfo::default();
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

    let sem_ci = ash::vk::SemaphoreCreateInfo::default();
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
    _window: &dyn Window,
    tab_frame: Option<(&Arc<ImportedVkImage>, u32, u32, i64)>,
    chrome_px: i32,
) -> Result<(), String> {
    unsafe {
        let device = &vk.device;
        let swapchain_image = vk.swapchain_images[vk.current_image_index as usize];
        let mut submitted_tab_image: Option<Arc<ImportedVkImage>> = None;

        // Reset the persistent command buffer for this frame.
        device.reset_command_buffer(vk.blit_cmd_buf, ash::vk::CommandBufferResetFlags::empty())
            .map_err(|e| format!("vkResetCommandBuffer (blit-present): {:?}", e))?;

        let begin_info = ash::vk::CommandBufferBeginInfo::default()
            .flags(ash::vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        device.begin_command_buffer(vk.blit_cmd_buf, &begin_info)
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

            if let Some(imported_wait) = import_wait_semaphore_for_frame(&vk.ash_instance, device, sem_handle)? {
                external_wait_semaphore = imported_wait;
            }

            let sw = vk.swapchain_extent.width as i32;
            let sh = vk.swapchain_extent.height as i32;
            let dst_h = (sh - chrome_px).max(0);

            // Transition swapchain image: PRESENT_SRC_KHR → TRANSFER_DST_OPTIMAL
            // Transition tab image:       GENERAL          → TRANSFER_SRC_OPTIMAL
            let barriers_pre = [
                ash::vk::ImageMemoryBarrier::default()
                    .src_access_mask(ash::vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
                    .dst_access_mask(ash::vk::AccessFlags::TRANSFER_WRITE)
                    .old_layout(ash::vk::ImageLayout::PRESENT_SRC_KHR)
                    .new_layout(ash::vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                    .src_queue_family_index(ash::vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(ash::vk::QUEUE_FAMILY_IGNORED)
                    .image(swapchain_image)
                    .subresource_range(COLOR_SUBRESOURCE_RANGE),
                ash::vk::ImageMemoryBarrier::default()
                    .src_access_mask(ash::vk::AccessFlags::empty())
                    .dst_access_mask(ash::vk::AccessFlags::TRANSFER_READ)
                    .old_layout(ash::vk::ImageLayout::GENERAL)
                    .new_layout(ash::vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                    .src_queue_family_index(ash::vk::QUEUE_FAMILY_EXTERNAL)
                    .dst_queue_family_index(vk.queue_family_index)
                    .image(tab.image())
                    .subresource_range(COLOR_SUBRESOURCE_RANGE),
            ];
            device.cmd_pipeline_barrier(
                vk.blit_cmd_buf,
                ash::vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT | ash::vk::PipelineStageFlags::TRANSFER,
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
            device.cmd_blit_image(
                vk.blit_cmd_buf,
                tab.image(), ash::vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
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
                    .src_queue_family_index(vk.queue_family_index)
                    .dst_queue_family_index(ash::vk::QUEUE_FAMILY_EXTERNAL)
                    .image(tab.image())
                    .subresource_range(COLOR_SUBRESOURCE_RANGE),
            ];
            device.cmd_pipeline_barrier(
                vk.blit_cmd_buf,
                ash::vk::PipelineStageFlags::TRANSFER,
                ash::vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                ash::vk::DependencyFlags::empty(),
                &[], &[], &barriers_post,
            );
        }

        device.end_command_buffer(vk.blit_cmd_buf)
            .map_err(|e| format!("vkEndCommandBuffer (blit-present): {:?}", e))?;

        // Submit: wait image_available (+ optional tab frame semaphore), signal render_finished.
        let mut wait_sems = vec![vk.image_available_semaphore];
        let mut wait_stages = vec![ash::vk::PipelineStageFlags::TRANSFER];
        if external_wait_semaphore != ash::vk::Semaphore::null() {
            wait_sems.push(external_wait_semaphore);
            wait_stages.push(ash::vk::PipelineStageFlags::TRANSFER);
        }
        let signal_sems = [vk.render_finished_semaphore];
        let cmd_bufs = [vk.blit_cmd_buf];
        let submit_info = ash::vk::SubmitInfo::default()
            .wait_semaphores(&wait_sems)
            .wait_dst_stage_mask(&wait_stages)
            .command_buffers(&cmd_bufs)
            .signal_semaphores(&signal_sems);

        if let Err(e) = device.queue_submit(vk.ash_queue, &[submit_info], ash::vk::Fence::null()) {
            if external_wait_semaphore != ash::vk::Semaphore::null() {
                device.destroy_semaphore(external_wait_semaphore, None);
            }
            return Err(format!("vkQueueSubmit (blit-present): {:?}", e));
        }

        if external_wait_semaphore != ash::vk::Semaphore::null() {
            vk.deferred_wait_semaphores.push(external_wait_semaphore);
        }
        vk.in_flight_tab_image = submitted_tab_image;

        // Present
        let swapchains = [vk.swapchain];
        let image_indices = [vk.current_image_index];
        let present_info = ash::vk::PresentInfoKHR::default()
            .wait_semaphores(&signal_sems)
            .swapchains(&swapchains)
            .image_indices(&image_indices);

        match vk.swapchain_fn.queue_present(vk.ash_queue, &present_info) {
            Ok(false) => {}
            Ok(true)
            | Err(ash::vk::Result::ERROR_OUT_OF_DATE_KHR)
            | Err(ash::vk::Result::ERROR_SURFACE_LOST_KHR) => {
                vk.swapchain_valid = false;
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

        if vk.swapchain != ash::vk::SwapchainKHR::null() {
            vk.swapchain_fn.destroy_swapchain(vk.swapchain, None);
            vk.swapchain = ash::vk::SwapchainKHR::null();
        }
        vk.swapchain_images.clear();
        vk.skia_surfaces.clear();

        let new_surface = vulkano::swapchain::Surface::from_window_ref(
            vk.instance.clone(),
            &*window,
        )
        .map_err(|e| format!("Surface::from_window_ref (recreate): {e:?}"))?;

        vk.surface_khr = new_surface.handle();
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
    // ── 1. Create winit window ───────────────────────────────────────────────
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

    // ── 2. Vulkan instance/device/queue via vulkano, then bridge into ash ──
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

    // ── 3. Surface from vulkano-owned surface handle ────────────────────────
    let surface_fn = ash::khr::surface::Instance::new(&entry, &instance);
    let surface_khr = vk_surface_owner.handle();

    // ── 4. Swapchain ─────────────────────────────────────────────────────────
    let swapchain_fn = ash::khr::swapchain::Device::new(&instance, &device);

    let (swapchain_format, skia_vk_format, color_type) =
        vk_pick_format(vk_surface_owner.as_ref().instance(), physical_device.clone(), vk_surface_owner.clone());

    let caps = unsafe {
        surface_fn.get_physical_device_surface_capabilities(ash_physical_device, surface_khr)
            .expect("Failed to query surface capabilities")
    };

    let window_size = window.surface_size();
    let extent = if caps.current_extent.width != u32::MAX {
        caps.current_extent
    } else {
        ash::vk::Extent2D {
            width: window_size.width.max(caps.min_image_extent.width).min(caps.max_image_extent.width),
            height: window_size.height.max(caps.min_image_extent.height).min(caps.max_image_extent.height),
        }
    };

    let min_image_count = caps.min_image_count.max(2).min(
        if caps.max_image_count > 0 { caps.max_image_count } else { u32::MAX }
    );

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
        pre_transform: SurfaceTransform::try_from(caps.current_transform).unwrap(),
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

    // ── 7. Synchronisation primitives ────────────────────────────────────────
    let semaphore_ci = ash::vk::SemaphoreCreateInfo::default();
    let fence_ci = ash::vk::FenceCreateInfo::default()
        .flags(ash::vk::FenceCreateFlags::SIGNALED);

    let image_available_semaphore = unsafe {
        device.create_semaphore(&semaphore_ci, None)
            .expect("Failed to create image_available semaphore")
    };
    let render_finished_semaphore = unsafe {
        device.create_semaphore(&semaphore_ci, None)
            .expect("Failed to create render_finished semaphore")
    };

    // ── 7b. Persistent blit command pool + buffer ────────────────────────────
    let blit_cmd_pool = unsafe {
        let pool_ci = ash::vk::CommandPoolCreateInfo::default()
            .queue_family_index(queue_family_index)
            .flags(ash::vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
        device.create_command_pool(&pool_ci, None)
            .expect("Failed to create blit command pool")
    };
    let blit_cmd_buf = unsafe {
        let ai = ash::vk::CommandBufferAllocateInfo::default()
            .command_pool(blit_cmd_pool)
            .level(ash::vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        device.allocate_command_buffers(&ai)
            .expect("Failed to allocate blit command buffer")[0]
    };

    // ── 8. Skia DirectContext via ash raw handles ────────────────────────────
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

    // TODO replace
    let swapchain_images = unsafe {
        swapchain_fn.get_swapchain_images(swapchain.handle())
            .expect("Failed to get swapchain images")
    };

    // ── 9. Skia surfaces (one per swapchain image) ───────────────────────────
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
        ash_physical_device: ash_physical_device,
        ash_device: device,
        ash_queue: queue,
        queue_family_index,
        instance: vk_instance,
        device: vk_device,
        physical_device: physical_device,
        queue: vk_queue_owner,
        allocator: vk_allocator,
        surface: vk_surface_owner,
        surface_khr,
        surface_fn,
        swapchain_fn,
        swapchain: swapchain.handle(),
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
