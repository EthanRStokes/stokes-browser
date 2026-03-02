use crate::vk_shared::ImportedVkImage;
use crate::vk_shared::VulkanDeviceInfo;
use ash::vk::{self, Handle};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use skia_safe::gpu::{self, DirectContext};
use skia_safe::{ColorType, Surface};
use std::collections::HashSet;
use std::ffi::CStr;
use std::ptr;
use std::sync::Arc;
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
        // Point env.surface at index 0 as a safe placeholder until the next present.
        if !self.vk.skia_surfaces.is_empty() {
            self.surface = self.vk.skia_surfaces[0].clone();
        }
    }

    /// Acquire the next swapchain image and point the Skia surface at it.
    /// Must be called at the start of each frame, before any Skia drawing.
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
    ///   3. `gr_context.flush_and_submit()` — Skia submits GPU work; image → PRESENT_SRC_KHR
    ///   4. `blit_tab_then_present()`  — blits tab, waits image_available, signals render_finished, presents
    ///
    /// If `tab_frame` is `None` the function skips the blit and just does the
    /// semaphore hand-off + present.
    pub fn blit_tab_then_present(
        &mut self,
        tab_frame: Option<(&ImportedVkImage, u32, u32, Option<i64>)>,
        chrome_px: i32,
    ) -> Result<(), String> {
        vk_blit_tab_then_present(&mut self.vk, &**self.window, tab_frame, chrome_px)
    }

    /// Flush Skia GPU work and present the current swapchain image.
    /// Must be called after all Skia drawing for the frame is done.
    pub fn flush_and_present(&mut self) -> Result<(), String> {
        vk_flush_and_present(&mut self.vk, &**self.window, &mut self.gr_context)
    }

    /// Present the rendered frame: acquires the next swapchain image, re-targets the
    /// Skia surface to it, flushes, and presents.
    pub fn present(&mut self) -> Result<(), String> {
        vk_present(&mut self.vk, &**self.window, &mut self.surface, &mut self.gr_context)
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

    /// One pre-built Skia `Surface` per swapchain image — created once, reused every frame.
    pub(crate) skia_surfaces: Vec<Surface>,
    pub(crate) swapchain_valid: bool,

    /// Index of the swapchain image acquired for the current frame.
    /// Set by `vk_acquire`, consumed by `vk_flush_and_present`.
    pub(crate) current_image_index: u32,

    /// Skia color type matching the swapchain format.
    pub(crate) color_type: ColorType,
    /// Skia/Vulkan format tag for `ImageInfo`.
    pub(crate) vk_format: skia_safe::gpu::vk::Format,

    // Synchronisation (single frame-in-flight)
    image_available_semaphore: vk::Semaphore,
    render_finished_semaphore: vk::Semaphore,
    in_flight_fence: vk::Fence,
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

/// Map an `ash::vk::Format` to the Skia equivalents needed by `ImageInfo`.
/// Returns `None` for formats Skia cannot handle directly.
fn vk_map_format(fmt: vk::Format) -> Option<(skia_safe::gpu::vk::Format, ColorType)> {
    match fmt {
        vk::Format::B8G8R8A8_UNORM => Some((skia_safe::gpu::vk::Format::B8G8R8A8_UNORM, ColorType::BGRA8888)),
        vk::Format::R8G8B8A8_UNORM => Some((skia_safe::gpu::vk::Format::R8G8B8A8_UNORM, ColorType::RGBA8888)),
        vk::Format::B8G8R8A8_SRGB  => Some((skia_safe::gpu::vk::Format::B8G8R8A8_SRGB,  ColorType::BGRA8888)),
        vk::Format::R8G8B8A8_SRGB  => Some((skia_safe::gpu::vk::Format::R8G8B8A8_SRGB,  ColorType::RGBA8888)),
        _ => None,
    }
}

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
            if let Some((vf, ct)) = vk_map_format(want) {
                return (want, vf, ct);
            }
        }
    }
    for sf in &formats {
        if let Some((vf, ct)) = vk_map_format(sf.format) {
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
        // Wait for any in-flight work to finish before destroying the old swapchain.
        vk.device.device_wait_idle().ok();

        let caps = vk.surface_fn
            .get_physical_device_surface_capabilities(vk.physical_device, vk.surface_khr)
            .expect("Failed to query surface capabilities");

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

        let new_swapchain = vk.swapchain_fn
            .create_swapchain(&swapchain_ci, None)
            .expect("Failed to recreate swapchain");

        // Destroy the old swapchain after creating the new one.
        if old_swapchain != vk::SwapchainKHR::null() {
            vk.swapchain_fn.destroy_swapchain(old_swapchain, None);
        }

        vk.swapchain = new_swapchain;
        vk.swapchain_extent = extent;
        vk.swapchain_images = vk.swapchain_fn
            .get_swapchain_images(new_swapchain)
            .expect("Failed to get swapchain images");

        // Drop old Skia surfaces before rebuilding.
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
            1,  // sample count
            None,
            None,
            None,
            None,
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

    unsafe {
        vk.device
            .wait_for_fences(&[vk.in_flight_fence], true, u64::MAX)
            .map_err(|e| format!("wait_for_fences: {:?}", e))?;
        vk.device
            .reset_fences(&[vk.in_flight_fence])
            .map_err(|e| format!("reset_fences: {:?}", e))?;

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

/// Flush Skia GPU work, optionally blit a tab frame into the page region of the
/// swapchain image, then present.
///
/// This is the zero-copy present path.  Ordering:
///   1. `gr_context.flush_and_submit()` — Skia finishes; swapchain image is now
///      in `PRESENT_SRC_KHR` layout.
///   2. A single command buffer is submitted that:
///        a. Waits on `image_available_semaphore` (WSI acquire guarantee).
///        b. If a tab frame is supplied: transitions the swapchain image
///           `PRESENT_SRC_KHR → TRANSFER_DST_OPTIMAL`, blits only the page
///           region (below chrome), then transitions back to `PRESENT_SRC_KHR`.
///        c. Signals `render_finished_semaphore`.
///   3. Present waits on `render_finished_semaphore`.
fn vk_blit_tab_then_present(
    vk: &mut VkState,
    window: &dyn Window,
    tab_frame: Option<(&ImportedVkImage, u32, u32, Option<i64>)>,
    chrome_px: i32,
) -> Result<(), String> {
    unsafe {
        let device = &vk.device;
        let swapchain_image = vk.swapchain_images[vk.current_image_index as usize];

        let mut imported_sem: Option<vk::Semaphore> = None;
        let mut wait_on_tab_sem = false;
        if let Some((_, _, _, Some(sem_handle))) = tab_frame {
            let sem = import_tab_semaphore(vk, sem_handle)?;
            imported_sem = Some(sem);
            wait_on_tab_sem = true;
        }

        // ── Command pool + buffer ────────────────────────────────────────────
        let cmd_pool = {
            let ci = vk::CommandPoolCreateInfo::default()
                .queue_family_index(vk.queue_family_index)
                .flags(vk::CommandPoolCreateFlags::TRANSIENT);
            device.create_command_pool(&ci, None)
                .map_err(|e| format!("vkCreateCommandPool (blit-present): {:?}", e))?
        };
        let cmd_buf = {
            let ai = vk::CommandBufferAllocateInfo::default()
                .command_pool(cmd_pool)
                .level(vk::CommandBufferLevel::PRIMARY)
                .command_buffer_count(1);
            let bufs = device.allocate_command_buffers(&ai).map_err(|e| {
                device.destroy_command_pool(cmd_pool, None);
                format!("vkAllocateCommandBuffers (blit-present): {:?}", e)
            })?;
            bufs[0]
        };

        let begin_info = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        device.begin_command_buffer(cmd_buf, &begin_info).map_err(|e| {
            device.destroy_command_pool(cmd_pool, None);
            format!("vkBeginCommandBuffer (blit-present): {:?}", e)
        })?;

        let subresource = vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        };
        let sublayers = vk::ImageSubresourceLayers {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            mip_level: 0,
            base_array_layer: 0,
            layer_count: 1,
        };

        if let Some((tab, tab_w, tab_h, _)) = tab_frame {
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
                    .subresource_range(subresource),
                vk::ImageMemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE | vk::AccessFlags::TRANSFER_WRITE)
                    .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
                    .old_layout(vk::ImageLayout::GENERAL)
                    .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .image(tab.image())
                    .subresource_range(subresource),
            ];
            device.cmd_pipeline_barrier(
                cmd_buf,
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
                cmd_buf,
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
                    .subresource_range(subresource),
                vk::ImageMemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::TRANSFER_READ)
                    .dst_access_mask(vk::AccessFlags::empty())
                    .old_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                    .new_layout(vk::ImageLayout::GENERAL)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .image(tab.image())
                    .subresource_range(subresource),
            ];
            device.cmd_pipeline_barrier(
                cmd_buf,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                vk::DependencyFlags::empty(),
                &[], &[], &barriers_post,
            );
        }

        device.end_command_buffer(cmd_buf)
            .map_err(|e| format!("vkEndCommandBuffer (blit-present): {:?}", e))?;

        // Submit: wait image_available (+ tab semaphore if available), execute blit, signal render_finished
        let wait_sems = if wait_on_tab_sem {
            [vk.image_available_semaphore, imported_sem.unwrap_or(vk::Semaphore::null())]
        } else {
            [vk.image_available_semaphore, vk::Semaphore::null()]
        };
        let wait_stages = [vk::PipelineStageFlags::TRANSFER, vk::PipelineStageFlags::TRANSFER];
        let wait_count = if wait_on_tab_sem { 2 } else { 1 };

        let signal_sems = [vk.render_finished_semaphore];
        let cmd_bufs = [cmd_buf];
        let submit_info = vk::SubmitInfo::default()
            .wait_semaphores(&wait_sems[..wait_count])
            .wait_dst_stage_mask(&wait_stages[..wait_count])
            .command_buffers(&cmd_bufs)
            .signal_semaphores(&signal_sems);

        device.queue_submit(vk.queue, &[submit_info], vk.in_flight_fence)
            .map_err(|e| format!("vkQueueSubmit (blit-present): {:?}", e))?;

        // Present
        let swapchains = [vk.swapchain];
        let image_indices = [vk.current_image_index];
        let present_info = vk::PresentInfoKHR::default()
            .wait_semaphores(&signal_sems)
            .swapchains(&swapchains)
            .image_indices(&image_indices);

        match vk.swapchain_fn.queue_present(vk.queue, &present_info) {
            Ok(false) => {}
            Ok(true) | Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                vk.swapchain_valid = false;
            }
            Err(e) => return Err(format!("queue_present: {:?}", e)),
        }

        // Free the transient command pool.
        // We wait on in_flight_fence so we know the GPU is done with cmd_pool
        // before destroying it. The fence stays signaled — vk_acquire resets it
        // at the top of the next frame (after its own wait_for_fences).
        device.wait_for_fences(&[vk.in_flight_fence], true, 5_000_000_000)
            .map_err(|e| format!("wait_for_fences (blit-present cleanup): {:?}", e))?;
        if let Some(sem) = imported_sem {
            device.destroy_semaphore(sem, None);
        }
        device.destroy_command_pool(cmd_pool, None);
        // Leave in_flight_fence SIGNALED — vk_acquire will reset it.
    }
    Ok(())
}

fn vk_flush_and_present(
    vk: &mut VkState,
    window: &dyn Window,
    gr_context: &mut DirectContext,
) -> Result<(), String> {
    unsafe {
        gr_context.flush_and_submit();

        let wait_semaphores = [vk.image_available_semaphore];
        let signal_semaphores = [vk.render_finished_semaphore];
        let wait_stages = [vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];

        let submit_info = vk::SubmitInfo::default()
            .wait_semaphores(&wait_semaphores)
            .wait_dst_stage_mask(&wait_stages)
            .signal_semaphores(&signal_semaphores);

        vk.device
            .queue_submit(vk.queue, &[submit_info], vk.in_flight_fence)
            .map_err(|e| format!("queue_submit: {:?}", e))?;

        let swapchains = [vk.swapchain];
        let image_indices = [vk.current_image_index];
        let present_info = vk::PresentInfoKHR::default()
            .wait_semaphores(&signal_semaphores)
            .swapchains(&swapchains)
            .image_indices(&image_indices);

        match vk.swapchain_fn.queue_present(vk.queue, &present_info) {
            Ok(false) => {}
            Ok(true) | Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                vk.swapchain_valid = false;
            }
            Err(e) => return Err(format!("queue_present: {:?}", e)),
        }
    }

    let _ = window;
    Ok(())
}

/// Acquire the next swapchain image, redirect the Skia surface to it, flush, and present.
fn vk_present(
    vk: &mut VkState,
    window: &dyn Window,
    current_surface: &mut Surface,
    gr_context: &mut DirectContext,
) -> Result<(), String> {
    if !vk.swapchain_valid {
        vk_prepare_swapchain(vk, window, gr_context);
    }

    unsafe {
        // Wait for the previous frame to finish.
        vk.device
            .wait_for_fences(&[vk.in_flight_fence], true, u64::MAX)
            .map_err(|e| format!("wait_for_fences: {:?}", e))?;
        vk.device
            .reset_fences(&[vk.in_flight_fence])
            .map_err(|e| format!("reset_fences: {:?}", e))?;

        // Acquire the next swapchain image.
        let (image_index, suboptimal) = match vk.swapchain_fn.acquire_next_image(
            vk.swapchain,
            u64::MAX,
            vk.image_available_semaphore,
            vk::Fence::null(),
        ) {
            Ok(result) => result,
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                vk.swapchain_valid = false;
                return Ok(());
            }
            Err(e) => return Err(format!("acquire_next_image: {:?}", e)),
        };

        if suboptimal {
            vk.swapchain_valid = false;
        }

        // Redirect the Skia surface for this frame.
        *current_surface = vk.skia_surfaces[image_index as usize].clone();

        // Flush Skia GPU work.
        gr_context.flush_and_submit();

        // Submit a dummy command buffer that waits on image_available and signals render_finished.
        let wait_semaphores = [vk.image_available_semaphore];
        let signal_semaphores = [vk.render_finished_semaphore];
        let wait_stages = [vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];

        let submit_info = vk::SubmitInfo::default()
            .wait_semaphores(&wait_semaphores)
            .wait_dst_stage_mask(&wait_stages)
            .signal_semaphores(&signal_semaphores);

        vk.device
            .queue_submit(vk.queue, &[submit_info], vk.in_flight_fence)
            .map_err(|e| format!("queue_submit: {:?}", e))?;

        // Present.
        let swapchains = [vk.swapchain];
        let image_indices = [image_index];
        let present_info = vk::PresentInfoKHR::default()
            .wait_semaphores(&signal_semaphores)
            .swapchains(&swapchains)
            .image_indices(&image_indices);

        match vk.swapchain_fn.queue_present(vk.queue, &present_info) {
            Ok(false) => {}
            Ok(true) | Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                vk.swapchain_valid = false;
            }
            Err(e) => return Err(format!("queue_present: {:?}", e)),
        }
    }

    Ok(())
}

#[cfg(windows)]
fn import_tab_semaphore(vk: &VkState, sem_handle: i64) -> Result<vk::Semaphore, String> {
    use ash::khr::external_semaphore_win32;

    let device = &vk.device;
    let semaphore = unsafe { device.create_semaphore(&vk::SemaphoreCreateInfo::default(), None) }
        .map_err(|e| format!("vkCreateSemaphore (tab import): {:?}", e))?;

    let ext = external_semaphore_win32::Device::new(&vk.instance, device);
    let import_info = vk::ImportSemaphoreWin32HandleInfoKHR::default()
        .semaphore(semaphore)
        .handle_type(vk::ExternalSemaphoreHandleTypeFlags::OPAQUE_WIN32)
        .flags(vk::SemaphoreImportFlags::TEMPORARY)
        .handle(sem_handle as vk::HANDLE);

    unsafe { ext.import_semaphore_win32_handle(&import_info) }
        .map_err(|e| {
            unsafe { device.destroy_semaphore(semaphore, None); }
            format!("vkImportSemaphoreWin32HandleKHR failed: {:?}", e)
        })?;

    Ok(semaphore)
}

#[cfg(not(windows))]
fn import_tab_semaphore(vk: &VkState, sem_handle: i64) -> Result<vk::Semaphore, String> {
    use ash::khr::external_semaphore_fd;

    let device = &vk.device;
    let semaphore = unsafe { device.create_semaphore(&vk::SemaphoreCreateInfo::default(), None) }
        .map_err(|e| format!("vkCreateSemaphore (tab import): {:?}", e))?;

    let ext = external_semaphore_fd::Device::new(&vk.instance, device);
    let import_info = vk::ImportSemaphoreFdInfoKHR::default()
        .semaphore(semaphore)
        .handle_type(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD)
        .flags(vk::SemaphoreImportFlags::TEMPORARY)
        .fd(sem_handle as i32);

    unsafe { ext.import_semaphore_fd(&import_info) }
        .map_err(|e| {
            unsafe { device.destroy_semaphore(semaphore, None); }
            format!("vkImportSemaphoreFdKHR failed: {:?}", e)
        })?;

    Ok(semaphore)
}

unsafe fn instance_supports_extension(entry: &ash::Entry, name: &CStr) -> bool {
    entry
        .enumerate_instance_extension_properties(None)
        .map(|exts| {
            exts.iter().any(|ext| unsafe { CStr::from_ptr(ext.extension_name.as_ptr()) } == name)
        })
        .unwrap_or(false)
}

unsafe fn select_instance_api_version(entry: &ash::Entry) -> u32 {
    let version = entry
        .try_enumerate_instance_version()
        .ok()
        .flatten()
        .unwrap_or(vk::API_VERSION_1_0);
    if version >= vk::API_VERSION_1_1 {
        vk::API_VERSION_1_1
    } else {
        vk::API_VERSION_1_0
    }
}

/// Create the main window using the Vulkan backend (pure ash, no vulkano).
pub(crate) unsafe fn create_window_vk(el: &Box<&dyn ActiveEventLoop>) -> Env {
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
    let api_version = select_instance_api_version(&entry);

    // Collect required instance extensions for surface creation.
    let display_handle = el.as_ref().display_handle().expect("Failed to get display handle");
    let mut required_extensions = ash_window::enumerate_required_extensions(display_handle.as_raw())
        .expect("Failed to enumerate required Vulkan extensions")
        .to_vec();

    if api_version < vk::API_VERSION_1_1 {
        let props2 = ash::khr::get_physical_device_properties2::NAME;
        if instance_supports_extension(&entry, props2) && !required_extensions.contains(&props2.as_ptr()) {
            required_extensions.push(props2.as_ptr());
        }
    }

    let app_info = vk::ApplicationInfo::default()
        .application_name(c"stokes-browser")
        .api_version(api_version);

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

    // ── 5. Logical device + queue ────────────────────────────────────────────
    let queue_priority = [1.0f32];
    let queue_ci = vk::DeviceQueueCreateInfo::default()
        .queue_family_index(queue_family_index)
        .queue_priorities(&queue_priority);

    let mut device_extensions: Vec<*const std::ffi::c_char> = vec![
        ash::khr::swapchain::NAME.as_ptr(),
        ash::khr::external_memory::NAME.as_ptr(),
        ash::khr::external_semaphore::NAME.as_ptr(),
        #[cfg(windows)]
        ash::khr::external_memory_win32::NAME.as_ptr(),
        #[cfg(not(windows))]
        ash::khr::external_memory_fd::NAME.as_ptr(),
        #[cfg(not(windows))]
        ash::vk::EXT_EXTERNAL_MEMORY_DMA_BUF_NAME.as_ptr(),
        #[cfg(windows)]
        ash::khr::external_semaphore_win32::NAME.as_ptr(),
        #[cfg(not(windows))]
        ash::khr::external_semaphore_fd::NAME.as_ptr(),
    ];

    let optional_skia_exts = [
        ash::khr::get_memory_requirements2::NAME,
        ash::khr::dedicated_allocation::NAME,
        ash::khr::bind_memory2::NAME,
        ash::khr::maintenance1::NAME,
        ash::khr::maintenance2::NAME,
        ash::khr::maintenance3::NAME,
        ash::khr::create_renderpass2::NAME,
        ash::khr::image_format_list::NAME,
        ash::khr::sampler_ycbcr_conversion::NAME,
    ];

    for ext in optional_skia_exts {
        if device_supports_extension(&instance, physical_device, ext)
            && !device_extensions.contains(&ext.as_ptr())
        {
            device_extensions.push(ext.as_ptr());
        }
    }

    if std::env::var("STOKES_VK_DEBUG").ok().as_deref() == Some("1") {
        let api = api_version;
        let inst_exts = required_extensions.iter()
            .map(|&ptr| unsafe { CStr::from_ptr(ptr) }.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        let dev_exts = device_extensions.iter()
            .map(|&ptr| unsafe { CStr::from_ptr(ptr) }.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        let avail_dev_exts = collect_device_extension_names(&instance, physical_device);
        eprintln!("[Vulkan] api_version=0x{api:x}");
        eprintln!("[Vulkan] instance_extensions={:?}", inst_exts);
        eprintln!("[Vulkan] device_extensions={:?}", dev_exts);
        eprintln!("[Vulkan] available_device_extensions={} entries", avail_dev_exts.len());
    }

    let features = unsafe { instance.get_physical_device_features(physical_device) };
    let device_ci = vk::DeviceCreateInfo::default()
        .queue_create_infos(std::slice::from_ref(&queue_ci))
        .enabled_extension_names(&device_extensions)
        .enabled_features(&features);

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

    // ── 8. Skia DirectContext via ash raw handles ────────────────────────────
    // Extract both raw function pointers before building the closure so neither
    // `entry` nor `instance` is borrowed, allowing them to be moved into VkState.
    let get_instance_proc_addr = entry.static_fn().get_instance_proc_addr;
    let get_device_proc_addr = instance.fp_v1_0().get_device_proc_addr;

    let get_proc = |of: gpu::vk::GetProcOf| {
        let fp: Option<unsafe extern "system" fn()> = match of {
            skia_safe::gpu::vk::GetProcOf::Instance(inst_raw, name) => {
                let vk_inst = vk::Instance::from_raw(inst_raw as _);
                unsafe { get_instance_proc_addr(vk_inst, name) }
            }
            skia_safe::gpu::vk::GetProcOf::Device(dev_raw, name) => {
                let vk_dev = vk::Device::from_raw(dev_raw as _);
                unsafe { get_device_proc_addr(vk_dev, name) }
            }
        };
        fp.map(|f| f as _).unwrap_or(ptr::null())
    };

    let backend_context = unsafe {
        skia_safe::gpu::vk::BackendContext::new(
            instance.handle().as_raw() as _,
            physical_device.as_raw() as _,
            device.handle().as_raw() as _,
            (queue.as_raw() as _, queue_family_index as usize),
            &get_proc,
        )
    };

    let mut gr_context =
        skia_safe::gpu::direct_contexts::make_vulkan(&backend_context, None)
            .expect("Failed to create Skia Vulkan DirectContext");

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
    };

    Env {
        surface: initial_surface,
        gr_context,
        window,
        vk,
    }
}

unsafe fn device_supports_extension(
    instance: &ash::Instance,
    physical_device: vk::PhysicalDevice,
    name: &CStr,
) -> bool {
    instance
        .enumerate_device_extension_properties(physical_device)
        .map(|exts| {
            exts.iter().any(|ext| unsafe { CStr::from_ptr(ext.extension_name.as_ptr()) } == name)
        })
        .unwrap_or(false)
}

unsafe fn collect_device_extension_names(
    instance: &ash::Instance,
    physical_device: vk::PhysicalDevice,
) -> HashSet<String> {
    instance
        .enumerate_device_extension_properties(physical_device)
        .map(|exts| {
            exts.iter()
                .map(|ext| unsafe { CStr::from_ptr(ext.extension_name.as_ptr()) }.to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}
