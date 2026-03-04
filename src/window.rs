use crate::vk_shared::{self, ImportedVkImage, VulkanDeviceInfo, SkiaGetProc, COLOR_SUBRESOURCE_RANGE};
use ash::vk::{self, Handle};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use skia_safe::gpu::{self, DirectContext};
use skia_safe::{ColorType, Surface};
use std::collections::HashSet;
use std::ffi::CStr;
use std::sync::Arc;
use skia_safe::gpu::vk::GetProcOf;
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
        vk_prepare_swapchain(&mut self.vk, &**self.window, &mut self.gr_context);
        if !self.vk.skia_surfaces.is_empty() {
            self.surface = self.vk.skia_surfaces[0].clone();
        }
    }

    /// Acquire the next swapchain image and point the Skia surface at it.
    /// Returns `false` if the swapchain was out-of-date and the frame should be
    /// skipped.
    pub fn acquire_frame(&mut self) -> Result<bool, String> {
        vk_acquire(&mut self.vk, &**self.window, &mut self.surface, &mut self.gr_context)
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
        tab_frame: Option<(&ImportedVkImage, u32, u32, i64)>,
        chrome_px: i32,
    ) -> Result<(), String> {
        vk_blit_tab_then_present(&mut self.vk, &**self.window, tab_frame, chrome_px)
    }
}

/// Vulkan-specific state (pure ash, no vulkano).
pub(crate) struct VkState {
    pub(crate) entry: ash::Entry,
    pub(crate) instance: ash::Instance,
    pub(crate) physical_device: vk::PhysicalDevice,
    pub(crate) device: ash::Device,
    pub(crate) queue: vk::Queue,
    pub(crate) queue_family_index: u32,

    // Surface
    surface_khr: vk::SurfaceKHR,
    surface_fn: ash::khr::surface::Instance,

    // Swapchain
    swapchain_fn: ash::khr::swapchain::Device,
    swapchain: vk::SwapchainKHR,
    swapchain_images: Vec<vk::Image>,
    swapchain_format: vk::Format,
    swapchain_extent: vk::Extent2D,

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
    image_available_semaphore: vk::Semaphore,
    render_finished_semaphore: vk::Semaphore,
    in_flight_fence: vk::Fence,

    // Persistent command pool + buffer for blit operations (reused each frame)
    blit_cmd_pool: vk::CommandPool,
    blit_cmd_buf: vk::CommandBuffer,

    /// Imported external semaphores waited by the previous submit.
    /// Destroy only after `in_flight_fence` signals.
    deferred_wait_semaphores: Vec<vk::Semaphore>,
}

impl VkState {
    /// Build a `VulkanDeviceInfo` that tab processes can use to attach to the
    /// same physical device and share VkImages with the parent.
    pub(crate) fn device_info(&self) -> VulkanDeviceInfo {
        let device_uuid = unsafe {
            crate::vk_shared::physical_device_uuid(&self.instance, self.physical_device)
        };
        VulkanDeviceInfo {
            device_uuid,
            queue_family_index: self.queue_family_index,
            image_format: self.swapchain_format.as_raw(),
            parent_pid: std::process::id(),
        }
    }
}

impl Drop for VkState {
    fn drop(&mut self) {
        unsafe {
            self.device.device_wait_idle().ok();
            for sem in self.deferred_wait_semaphores.drain(..) {
                self.device.destroy_semaphore(sem, None);
            }
            self.device.destroy_command_pool(self.blit_cmd_pool, None);
            self.device.destroy_semaphore(self.image_available_semaphore, None);
            self.device.destroy_semaphore(self.render_finished_semaphore, None);
            self.device.destroy_fence(self.in_flight_fence, None);
            self.swapchain_fn.destroy_swapchain(self.swapchain, None);
            self.surface_fn.destroy_surface(self.surface_khr, None);
            self.device.destroy_device(None);
            self.instance.destroy_instance(None);
        }
    }
}

// ---------------------------------------------------------------------------
// Vulkan helpers
// ---------------------------------------------------------------------------

/// Choose the best swapchain format, preferring UNORM over SRGB so Skia's maths stays linear.
fn vk_pick_format(
    surface_fn: &ash::khr::surface::Instance,
    physical_device: vk::PhysicalDevice,
    surface: vk::SurfaceKHR,
) -> (vk::Format, skia_safe::gpu::vk::Format, ColorType) {
    let formats = unsafe {
        surface_fn.get_physical_device_surface_formats(physical_device, surface)
            .expect("Failed to query surface formats")
    };

    let preferred = [
        vk::Format::B8G8R8A8_UNORM,
        vk::Format::R8G8B8A8_UNORM,
        vk::Format::B8G8R8A8_SRGB,
        vk::Format::R8G8B8A8_SRGB,
    ];
    for want in preferred {
        if formats.iter().any(|sf| sf.format == want) {
            if let Some((vf, ct)) = vk_shared::vk_format_to_skia(want) {
                return (want, vf, ct);
            }
        }
    }
    for sf in &formats {
        if let Some((vf, ct)) = vk_shared::vk_format_to_skia(sf.format) {
            return (sf.format, vf, ct);
        }
    }
    panic!("No Skia-compatible Vulkan swapchain format found. Available: {formats:?}");
}

/// Recreate the swapchain after a resize (sets `swapchain_valid = true`).
fn vk_prepare_swapchain(vk: &mut VkState, window: &dyn Window, gr_context: &mut DirectContext) {
    let sz = window.surface_size();
    if sz.width == 0 || sz.height == 0 || vk.swapchain_valid {
        return;
    }

    unsafe {
        vk.device.device_wait_idle().ok();

        let caps = match vk
            .surface_fn
            .get_physical_device_surface_capabilities(vk.physical_device, vk.surface_khr)
        {
            Ok(caps) => caps,
            Err(vk::Result::ERROR_SURFACE_LOST_KHR) => {
                let _ = vk_recreate_surface(vk, window);
                return;
            }
            Err(e) => panic!("Failed to query surface capabilities: {e:?}"),
        };

        let extent = if caps.current_extent.width != u32::MAX {
            caps.current_extent
        } else {
            vk::Extent2D {
                width: sz.width.max(caps.min_image_extent.width).min(caps.max_image_extent.width),
                height: sz.height.max(caps.min_image_extent.height).min(caps.max_image_extent.height),
            }
        };

        let min_image_count = caps.min_image_count.max(2).min(
            if caps.max_image_count > 0 { caps.max_image_count } else { u32::MAX }
        );

        let old_swapchain = vk.swapchain;

        let swapchain_ci = vk::SwapchainCreateInfoKHR::default()
            .surface(vk.surface_khr)
            .min_image_count(min_image_count)
            .image_format(vk.swapchain_format)
            .image_color_space(vk::ColorSpaceKHR::SRGB_NONLINEAR)
            .image_extent(extent)
            .image_array_layers(1)
            .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_DST)
            .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
            .pre_transform(caps.current_transform)
            .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
            .present_mode(vk::PresentModeKHR::FIFO)
            .clipped(true)
            .old_swapchain(old_swapchain);

        let new_swapchain = match vk.swapchain_fn.create_swapchain(&swapchain_ci, None) {
            Ok(sc) => sc,
            Err(vk::Result::ERROR_SURFACE_LOST_KHR) => {
                let _ = vk_recreate_surface(vk, window);
                return;
            }
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                vk.swapchain_valid = false;
                return;
            }
            Err(e) => panic!("Failed to recreate swapchain: {e:?}"),
        };

        if old_swapchain != vk::SwapchainKHR::null() {
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
    image: vk::Image,
    extent: vk::Extent2D,
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
    images: &[vk::Image],
    extent: vk::Extent2D,
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
    window: &dyn Window,
    current_surface: &mut Surface,
    gr_context: &mut DirectContext,
) -> Result<bool, String> {
    if !vk.swapchain_valid {
        vk_prepare_swapchain(vk, window, gr_context);
    }
    if !vk.swapchain_valid || vk.skia_surfaces.is_empty() {
        return Ok(false);
    }

    unsafe {
        vk.device
            .wait_for_fences(&[vk.in_flight_fence], true, u64::MAX)
            .map_err(|e| format!("wait_for_fences: {:?}", e))?;

        // Previous submit is complete, so it is now safe to destroy per-frame
        // imported wait semaphores created for external tab sync.
        for sem in vk.deferred_wait_semaphores.drain(..) {
            vk.device.destroy_semaphore(sem, None);
        }

        let (image_index, suboptimal) = match vk.swapchain_fn.acquire_next_image(
            vk.swapchain,
            u64::MAX,
            vk.image_available_semaphore,
            vk::Fence::null(),
        ) {
            Ok(result) => result,
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                vk.swapchain_valid = false;
                return Ok(false);
            }
            Err(vk::Result::ERROR_SURFACE_LOST_KHR) => {
                vk_recreate_surface(vk, window)?;
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
) -> Result<Option<vk::Semaphore>, String> {
    if sem_handle < 0 {
        return Ok(None);
    }

    let sem_ci = vk::SemaphoreCreateInfo::default();
    let sem = device
        .create_semaphore(&sem_ci, None)
        .map_err(|e| format!("vkCreateSemaphore (import wait): {:?}", e))?;

    let ext_sem_fd = ash::khr::external_semaphore_fd::Device::new(instance, device);
    let import_info = vk::ImportSemaphoreFdInfoKHR::default()
        .semaphore(sem)
        .flags(vk::SemaphoreImportFlags::TEMPORARY)
        .handle_type(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD)
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
) -> Result<Option<vk::Semaphore>, String> {
    if sem_handle == 0 {
        return Ok(None);
    }

    let sem_ci = vk::SemaphoreCreateInfo::default();
    let sem = device
        .create_semaphore(&sem_ci, None)
        .map_err(|e| format!("vkCreateSemaphore (import wait): {:?}", e))?;

    let ext_sem_win32 = ash::khr::external_semaphore_win32::Device::new(instance, device);
    let mut import_info = vk::ImportSemaphoreWin32HandleInfoKHR::default()
        .semaphore(sem)
        .flags(vk::SemaphoreImportFlags::TEMPORARY)
        .handle_type(vk::ExternalSemaphoreHandleTypeFlags::OPAQUE_WIN32)
        .handle(sem_handle as vk::HANDLE);

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
    tab_frame: Option<(&ImportedVkImage, u32, u32, i64)>,
    chrome_px: i32,
) -> Result<(), String> {
    unsafe {
        let device = &vk.device;
        let swapchain_image = vk.swapchain_images[vk.current_image_index as usize];

        // Reset the persistent command buffer for this frame.
        device.reset_command_buffer(vk.blit_cmd_buf, vk::CommandBufferResetFlags::empty())
            .map_err(|e| format!("vkResetCommandBuffer (blit-present): {:?}", e))?;

        let begin_info = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        device.begin_command_buffer(vk.blit_cmd_buf, &begin_info)
            .map_err(|e| format!("vkBeginCommandBuffer (blit-present): {:?}", e))?;

        let mut external_wait_semaphore = vk::Semaphore::null();

        let sublayers = vk::ImageSubresourceLayers {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            mip_level: 0,
            base_array_layer: 0,
            layer_count: 1,
        };

        if let Some((tab, tab_w, tab_h, sem_handle)) = tab_frame {
            if let Some(imported_wait) = import_wait_semaphore_for_frame(&vk.instance, device, sem_handle)? {
                external_wait_semaphore = imported_wait;
            }

            let sw = vk.swapchain_extent.width as i32;
            let sh = vk.swapchain_extent.height as i32;
            let dst_h = (sh - chrome_px).max(0);

            // Transition swapchain image: PRESENT_SRC_KHR → TRANSFER_DST_OPTIMAL
            // Transition tab image:       GENERAL          → TRANSFER_SRC_OPTIMAL
            let barriers_pre = [
                vk::ImageMemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
                    .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                    .old_layout(vk::ImageLayout::PRESENT_SRC_KHR)
                    .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .image(swapchain_image)
                    .subresource_range(COLOR_SUBRESOURCE_RANGE),
                vk::ImageMemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE | vk::AccessFlags::TRANSFER_WRITE)
                    .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
                    .old_layout(vk::ImageLayout::GENERAL)
                    .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .image(tab.image())
                    .subresource_range(COLOR_SUBRESOURCE_RANGE),
            ];
            device.cmd_pipeline_barrier(
                vk.blit_cmd_buf,
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT | vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::TRANSFER,
                vk::DependencyFlags::empty(),
                &[], &[], &barriers_pre,
            );

            // Blit tab → swapchain page region
            let blit = vk::ImageBlit::default()
                .src_subresource(sublayers)
                .src_offsets([
                    vk::Offset3D { x: 0, y: 0, z: 0 },
                    vk::Offset3D { x: tab_w as i32, y: tab_h as i32, z: 1 },
                ])
                .dst_subresource(sublayers)
                .dst_offsets([
                    vk::Offset3D { x: 0, y: chrome_px, z: 0 },
                    vk::Offset3D { x: sw, y: chrome_px + dst_h, z: 1 },
                ]);
            device.cmd_blit_image(
                vk.blit_cmd_buf,
                tab.image(), vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                swapchain_image, vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &[blit],
                vk::Filter::LINEAR,
            );

            // Transition back:
            //   swapchain: TRANSFER_DST → PRESENT_SRC_KHR
            //   tab image: TRANSFER_SRC → GENERAL (ready for next frame)
            let barriers_post = [
                vk::ImageMemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                    .dst_access_mask(vk::AccessFlags::empty())
                    .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                    .new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .image(swapchain_image)
                    .subresource_range(COLOR_SUBRESOURCE_RANGE),
                vk::ImageMemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::TRANSFER_READ)
                    .dst_access_mask(vk::AccessFlags::empty())
                    .old_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                    .new_layout(vk::ImageLayout::GENERAL)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .image(tab.image())
                    .subresource_range(COLOR_SUBRESOURCE_RANGE),
            ];
            device.cmd_pipeline_barrier(
                vk.blit_cmd_buf,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                vk::DependencyFlags::empty(),
                &[], &[], &barriers_post,
            );
        }

        device.end_command_buffer(vk.blit_cmd_buf)
            .map_err(|e| format!("vkEndCommandBuffer (blit-present): {:?}", e))?;

        // Submit: wait image_available (+ optional tab frame semaphore), signal render_finished.
        let mut wait_sems = vec![vk.image_available_semaphore];
        let mut wait_stages = vec![vk::PipelineStageFlags::TRANSFER];
        if external_wait_semaphore != vk::Semaphore::null() {
            wait_sems.push(external_wait_semaphore);
            wait_stages.push(vk::PipelineStageFlags::TRANSFER);
        }
        let signal_sems = [vk.render_finished_semaphore];
        let cmd_bufs = [vk.blit_cmd_buf];
        let submit_info = vk::SubmitInfo::default()
            .wait_semaphores(&wait_sems)
            .wait_dst_stage_mask(&wait_stages)
            .command_buffers(&cmd_bufs)
            .signal_semaphores(&signal_sems);

        // Reset only immediately before the submit that will signal this fence.
        device
            .reset_fences(&[vk.in_flight_fence])
            .map_err(|e| format!("reset_fences: {:?}", e))?;

        if let Err(e) = device.queue_submit(vk.queue, &[submit_info], vk.in_flight_fence) {
            if external_wait_semaphore != vk::Semaphore::null() {
                device.destroy_semaphore(external_wait_semaphore, None);
            }
            return Err(format!("vkQueueSubmit (blit-present): {:?}", e));
        }

        if external_wait_semaphore != vk::Semaphore::null() {
            vk.deferred_wait_semaphores.push(external_wait_semaphore);
        }

        // Present
        let swapchains = [vk.swapchain];
        let image_indices = [vk.current_image_index];
        let present_info = vk::PresentInfoKHR::default()
            .wait_semaphores(&signal_sems)
            .swapchains(&swapchains)
            .image_indices(&image_indices);

        match vk.swapchain_fn.queue_present(vk.queue, &present_info) {
            Ok(false) => {}
            Ok(true)
            | Err(vk::Result::ERROR_OUT_OF_DATE_KHR)
            | Err(vk::Result::ERROR_SURFACE_LOST_KHR) => {
                vk.swapchain_valid = false;
            }
            Err(e) => return Err(format!("queue_present: {:?}", e)),
        }
    }
    Ok(())
}

/// Recreate the Vulkan surface from the current native window handles.
fn vk_recreate_surface(vk: &mut VkState, window: &dyn Window) -> Result<(), String> {
    unsafe {
        vk.device.device_wait_idle().ok();

        if vk.swapchain != vk::SwapchainKHR::null() {
            vk.swapchain_fn.destroy_swapchain(vk.swapchain, None);
            vk.swapchain = vk::SwapchainKHR::null();
        }
        vk.swapchain_images.clear();
        vk.skia_surfaces.clear();

        if vk.surface_khr != vk::SurfaceKHR::null() {
            vk.surface_fn.destroy_surface(vk.surface_khr, None);
            vk.surface_khr = vk::SurfaceKHR::null();
        }

        let display_handle = window
            .display_handle()
            .map_err(|e| format!("display_handle: {e}"))?;
        let window_handle = window
            .window_handle()
            .map_err(|e| format!("window_handle: {e}"))?;

        vk.surface_khr = ash_window::create_surface(
            &vk.entry,
            &vk.instance,
            display_handle.as_raw(),
            window_handle.as_raw(),
            None,
        )
        .map_err(|e| format!("create_surface (recreate): {e:?}"))?;

        vk.current_image_index = 0;
        vk.swapchain_valid = false;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Public window constructor
// ---------------------------------------------------------------------------

/// Create the main window using the Vulkan backend (pure ash, no vulkano).
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

    // ── 2. ash Entry + Instance ──────────────────────────────────────────────
    let entry = unsafe { ash::Entry::load().expect("Failed to load Vulkan library") };

    let instance_api_version = unsafe {
        entry
            .try_enumerate_instance_version()
            .ok()
            .flatten()
            .unwrap_or(vk::API_VERSION_1_0)
    };

    let display_handle = el.as_ref().display_handle().expect("Failed to get display handle");
    let required_extensions = ash_window::enumerate_required_extensions(display_handle.as_raw())
        .expect("Failed to enumerate required Vulkan extensions")
        .to_vec();

    let app_info = vk::ApplicationInfo::default()
        .application_name(c"stokes-browser")
        .api_version(instance_api_version);

    let instance_ci = vk::InstanceCreateInfo::default()
        .application_info(&app_info)
        .enabled_extension_names(&required_extensions);

    let instance = unsafe {
        entry.create_instance(&instance_ci, None)
            .expect("Failed to create Vulkan instance")
    };

    // ── 3. Surface from the winit window ─────────────────────────────────────
    let surface_fn = ash::khr::surface::Instance::new(&entry, &instance);

    let window_handle = window.window_handle().expect("Failed to get window handle");
    let surface_khr = unsafe {
        ash_window::create_surface(
            &entry,
            &instance,
            display_handle.as_raw(),
            window_handle.as_raw(),
            None,
        )
        .expect("Failed to create Vulkan surface")
    };

    // ── 4. Physical device + queue family ────────────────────────────────────
    let physical_devices = unsafe {
        instance.enumerate_physical_devices()
            .expect("Failed to enumerate physical devices")
    };

    let (physical_device, queue_family_index) = physical_devices
        .iter()
        .filter_map(|&pd| {
            let queue_families = unsafe { instance.get_physical_device_queue_family_properties(pd) };
            queue_families.iter().enumerate()
                .position(|(i, q)| {
                    q.queue_flags.contains(vk::QueueFlags::GRAPHICS)
                        && unsafe {
                            surface_fn.get_physical_device_surface_support(pd, i as u32, surface_khr)
                                .unwrap_or(false)
                        }
                })
                .map(|i| (pd, i as u32))
        })
        .min_by_key(|(pd, _)| {
            let props = unsafe { instance.get_physical_device_properties(*pd) };
            match props.device_type {
                vk::PhysicalDeviceType::DISCRETE_GPU   => 0u32,
                vk::PhysicalDeviceType::INTEGRATED_GPU => 1,
                vk::PhysicalDeviceType::VIRTUAL_GPU    => 2,
                vk::PhysicalDeviceType::CPU            => 3,
                _                                      => 4,
            }
        })
        .expect("No suitable Vulkan physical device found");

    let device_api_version = unsafe {
        instance
            .get_physical_device_properties(physical_device)
            .api_version
    };
    let negotiated_api_version = instance_api_version.min(device_api_version);

    // ── 5. Logical device + queue ────────────────────────────────────────────
    let queue_priority = [1.0f32];
    let queue_ci = vk::DeviceQueueCreateInfo::default()
        .queue_family_index(queue_family_index)
        .queue_priorities(&queue_priority);

    let device_extensions = crate::vk_shared::parent_device_extension_names();

    // Validate required extensions up-front.
    let available_exts = unsafe {
        instance
            .enumerate_device_extension_properties(physical_device)
            .expect("Failed to enumerate Vulkan device extensions")
    };
    let available_ext_names: HashSet<String> = available_exts
        .iter()
        .map(|p| unsafe { CStr::from_ptr(p.extension_name.as_ptr()) }.to_string_lossy().into_owned())
        .collect();
    let missing_exts: Vec<String> = device_extensions
        .iter()
        .map(|&name| unsafe { CStr::from_ptr(name) }.to_string_lossy().into_owned())
        .filter(|name| !available_ext_names.contains(name))
        .collect();
    if !missing_exts.is_empty() {
        let driver_name = unsafe { CStr::from_ptr(instance.get_physical_device_properties(physical_device).device_name.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        panic!(
            "Selected GPU is missing required Vulkan device extensions: {:?}. GPU: {}",
            missing_exts,
            driver_name,
        );
    }

    let device_ci = vk::DeviceCreateInfo::default()
        .queue_create_infos(std::slice::from_ref(&queue_ci))
        .enabled_extension_names(&device_extensions);

    let device = unsafe {
        instance.create_device(physical_device, &device_ci, None)
            .expect("Failed to create Vulkan logical device")
    };

    let queue = unsafe { device.get_device_queue(queue_family_index, 0) };

    // ── 6. Swapchain ─────────────────────────────────────────────────────────
    let swapchain_fn = ash::khr::swapchain::Device::new(&instance, &device);

    let (swapchain_format, skia_vk_format, color_type) =
        vk_pick_format(&surface_fn, physical_device, surface_khr);

    let caps = unsafe {
        surface_fn.get_physical_device_surface_capabilities(physical_device, surface_khr)
            .expect("Failed to query surface capabilities")
    };

    let window_size = window.surface_size();
    let extent = if caps.current_extent.width != u32::MAX {
        caps.current_extent
    } else {
        vk::Extent2D {
            width: window_size.width.max(caps.min_image_extent.width).min(caps.max_image_extent.width),
            height: window_size.height.max(caps.min_image_extent.height).min(caps.max_image_extent.height),
        }
    };

    let min_image_count = caps.min_image_count.max(2).min(
        if caps.max_image_count > 0 { caps.max_image_count } else { u32::MAX }
    );

    let swapchain_ci = vk::SwapchainCreateInfoKHR::default()
        .surface(surface_khr)
        .min_image_count(min_image_count)
        .image_format(swapchain_format)
        .image_color_space(vk::ColorSpaceKHR::SRGB_NONLINEAR)
        .image_extent(extent)
        .image_array_layers(1)
        .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_DST)
        .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
        .pre_transform(caps.current_transform)
        .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
        .present_mode(vk::PresentModeKHR::FIFO)
        .clipped(true);

    let swapchain = unsafe {
        swapchain_fn.create_swapchain(&swapchain_ci, None)
            .expect("Failed to create swapchain")
    };

    let swapchain_images = unsafe {
        swapchain_fn.get_swapchain_images(swapchain)
            .expect("Failed to get swapchain images")
    };

    // ── 7. Synchronisation primitives ────────────────────────────────────────
    let semaphore_ci = vk::SemaphoreCreateInfo::default();
    let fence_ci = vk::FenceCreateInfo::default()
        .flags(vk::FenceCreateFlags::SIGNALED);

    let image_available_semaphore = unsafe {
        device.create_semaphore(&semaphore_ci, None)
            .expect("Failed to create image_available semaphore")
    };
    let render_finished_semaphore = unsafe {
        device.create_semaphore(&semaphore_ci, None)
            .expect("Failed to create render_finished semaphore")
    };
    let in_flight_fence = unsafe {
        device.create_fence(&fence_ci, None)
            .expect("Failed to create in_flight fence")
    };

    // ── 7b. Persistent blit command pool + buffer ────────────────────────────
    let blit_cmd_pool = unsafe {
        let pool_ci = vk::CommandPoolCreateInfo::default()
            .queue_family_index(queue_family_index)
            .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
        device.create_command_pool(&pool_ci, None)
            .expect("Failed to create blit command pool")
    };
    let blit_cmd_buf = unsafe {
        let ai = vk::CommandBufferAllocateInfo::default()
            .command_pool(blit_cmd_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
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
                physical_device.as_raw() as _,
                device.handle().as_raw() as _,
                (queue.as_raw() as _, queue_family_index as usize),
                &get_proc_fn,
            )
        };
        backend_context.set_max_api_version(negotiated_api_version);

        skia_safe::gpu::direct_contexts::make_vulkan(&backend_context, None)
            .unwrap_or_else(|| {
                let props = unsafe { instance.get_physical_device_properties(physical_device) };
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
        instance,
        physical_device,
        device,
        queue,
        queue_family_index,
        surface_khr,
        surface_fn,
        swapchain_fn,
        swapchain,
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
        in_flight_fence,
        blit_cmd_pool,
        blit_cmd_buf,
        deferred_wait_semaphores: Vec::new(),
    };

    Env {
        surface: initial_surface,
        gr_context,
        window,
        vk,
    }
}



