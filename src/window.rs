use crate::vk_context;
use crate::vk_shared::{self, ImportedVkImage, SkiaGetProc, VulkanDeviceInfo};
use skia_safe::gpu::vk::GetProcOf;
use skia_safe::gpu::{self, DirectContext};
use skia_safe::{ColorType, Surface};
use std::sync::Arc;
use vulkano::command_buffer::allocator::{CommandBufferAllocator, StandardCommandBufferAllocator, StandardCommandBufferAllocatorCreateInfo};
use vulkano::command_buffer::{
    BlitImageInfo, CommandBufferBeginInfo, CommandBufferLevel, CommandBufferUsage, ImageBlit,
    RecordingCommandBuffer,
};
use vulkano::device::physical::PhysicalDevice;
use vulkano::device::{Device, Queue};
use vulkano::format::Format;
use vulkano::image::sampler::Filter;
use vulkano::image::{Image, ImageAspects, ImageLayout, ImageSubresourceLayers, ImageUsage};
use vulkano::instance::Instance;
use vulkano::memory::allocator::MemoryAllocator;
use vulkano::swapchain::{
    AcquireNextImageInfo, ColorSpace, CompositeAlpha, FullScreenExclusive, PresentInfo,
    PresentMode, SemaphorePresentInfo, Swapchain, SwapchainCreateInfo, SwapchainPresentInfo,
};
use vulkano::sync::semaphore::{Semaphore, SemaphoreCreateInfo};
use vulkano::sync::{AccessFlags, DependencyInfo, ImageMemoryBarrier, PipelineStages, Sharing};
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
        tab_frame: Option<(&Arc<ImportedVkImage>, u32, u32)>,
        chrome_px: i32,
    ) -> Result<(), String> {
        vk_blit_tab_then_present(&mut self.vk, self.window.clone(), tab_frame, chrome_px)
    }
}

/// Vulkan-specific state.
pub(crate) struct VkState {
    pub(crate) queue_family_index: u32,
    pub(crate) negotiated_api_version: u32,

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
    swapchain_extent: [u32; 2],

    /// One pre-built Skia `Surface` per swapchain image.
    pub(crate) skia_surfaces: Vec<Surface>,
    pub(crate) swapchain_valid: bool,

    /// Index of the swapchain image acquired for the current frame.
    pub(crate) current_image_index: u32,

    /// Skia color type matching the swapchain format.
    pub(crate) color_type: ColorType,
    /// Skia/Vulkan format tag for `ImageInfo`.
    pub(crate) vk_format: skia_safe::gpu::vk::Format,

    image_available_semaphore: Arc<Semaphore>,
    render_finished_semaphore: Arc<Semaphore>,
    cmd_buf_allocator: Arc<dyn CommandBufferAllocator>,

    /// Keep the submitted tab image alive until the current frame finishes.
    in_flight_tab_image: Option<Arc<ImportedVkImage>>,
}

impl VkState {
    /// Build a `VulkanDeviceInfo` that tab processes can use to attach to the
    /// same physical device and share VkImages with the parent.
    pub(crate) fn device_info(&self) -> VulkanDeviceInfo {
        let device_uuid = crate::vk_shared::physical_device_uuid(&self.physical_device);
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
        let _ = self.queue.with(|mut q| q.wait_idle());
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

fn clamp_surface_extent(
    caps: &vulkano::swapchain::SurfaceCapabilities,
    width: u32,
    height: u32,
) -> [u32; 2] {
    if let Some(current_extent) = caps.current_extent {
        [current_extent[0], current_extent[1]]
    } else {
        [
            width.max(caps.min_image_extent[0]).min(caps.max_image_extent[0]),
            height.max(caps.min_image_extent[1]).min(caps.max_image_extent[1]),
        ]
    }
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

    let _ = vk.queue.with(|mut q| q.wait_idle());

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

    let extent = clamp_surface_extent(&caps, sz.width, sz.height);

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
        image_extent: extent,
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

    Ok(())
}

/// Build a Skia `Surface` that renders into a specific Vulkan swapchain image.
fn vk_surface_for_image(
    gr_context: &mut DirectContext,
    image: &Arc<Image>,
    extent: [u32; 2],
    vk_format: skia_safe::gpu::vk::Format,
    color_type: ColorType,
) -> Surface {
    let alloc = skia_safe::gpu::vk::Alloc::default();
    let image_info = unsafe {
        skia_safe::gpu::vk::ImageInfo::new(
            crate::vk_shared::raw_image_handle(image) as _,
            alloc,
            skia_safe::gpu::vk::ImageTiling::OPTIMAL,
            skia_safe::gpu::vk::ImageLayout::UNDEFINED,
            vk_format,
            1,
            None, None, None, None,
        )
    };

    let render_target = gpu::backend_render_targets::make_vk(
        (extent[0] as i32, extent[1] as i32),
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
    extent: [u32; 2],
    vk_format: skia_safe::gpu::vk::Format,
    color_type: ColorType,
) -> Vec<Surface> {
    images
        .iter()
        .map(|img| vk_surface_for_image(gr_context, img, extent, vk_format, color_type))
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

    vk.queue
        .with(|mut q| q.wait_idle())
        .map_err(|e| format!("queue_wait_idle: {e:?}"))?;
    vk.in_flight_tab_image = None;

    let acquired = match unsafe {
        vk.vk_swapchain.acquire_next_image(&AcquireNextImageInfo {
            timeout: None,
            semaphore: Some(vk.image_available_semaphore.clone()),
            fence: None,
            ..Default::default()
        })
    } {
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

    Ok(true)
}

/// Flush Skia GPU work, optionally blit a tab frame into the page region of the
/// swapchain image, then present.
fn vk_blit_tab_then_present(
    vk: &mut VkState,
    window: Arc<Box<dyn Window>>,
    tab_frame: Option<(&Arc<ImportedVkImage>, u32, u32)>,
    chrome_px: i32,
) -> Result<(), String> {
    let swapchain_image = vk.swapchain_images[vk.current_image_index as usize].clone();
    let mut submitted_tab_image: Option<Arc<ImportedVkImage>> = None;

    let begin_info = CommandBufferBeginInfo {
        usage: CommandBufferUsage::OneTimeSubmit,
        ..Default::default()
    };
    let mut cmd_buf = RecordingCommandBuffer::new(
        vk.cmd_buf_allocator.clone(),
        vk.queue_family_index,
        CommandBufferLevel::Primary,
        begin_info,
    )
    .map_err(|e| format!("RecordingCommandBuffer::new (blit-present): {e:?}"))?;

    if let Some((tab, tab_w, tab_h)) = tab_frame {
        submitted_tab_image = Some(tab.clone());

        let sw = vk.swapchain_extent[0];
        let sh = vk.swapchain_extent[1] as i32;
        let dst_h = (sh - chrome_px).max(0) as u32;
        let color_range = crate::vk_shared::color_subresource_range();

        let pre_barrier = DependencyInfo {
            image_memory_barriers: vec![
                ImageMemoryBarrier {
                    src_stages: PipelineStages::COLOR_ATTACHMENT_OUTPUT | PipelineStages::ALL_COMMANDS,
                    src_access: AccessFlags::COLOR_ATTACHMENT_WRITE,
                    dst_stages: PipelineStages::ALL_COMMANDS,
                    dst_access: AccessFlags::TRANSFER_WRITE,
                    old_layout: ImageLayout::ColorAttachmentOptimal,
                    new_layout: ImageLayout::TransferDstOptimal,
                    subresource_range: color_range.clone(),
                    ..ImageMemoryBarrier::image(swapchain_image.clone())
                },
                ImageMemoryBarrier {
                    src_stages: PipelineStages::COLOR_ATTACHMENT_OUTPUT | PipelineStages::ALL_COMMANDS,
                    src_access: AccessFlags::COLOR_ATTACHMENT_WRITE,
                    dst_stages: PipelineStages::ALL_COMMANDS,
                    dst_access: AccessFlags::TRANSFER_READ,
                    old_layout: ImageLayout::ColorAttachmentOptimal,
                    new_layout: ImageLayout::TransferSrcOptimal,
                    subresource_range: color_range.clone(),
                    ..ImageMemoryBarrier::image(tab.image())
                },
            ]
            .into(),
            ..Default::default()
        };
        unsafe {
            cmd_buf
                .pipeline_barrier(&pre_barrier)
                .map_err(|e| format!("pipeline_barrier pre (blit-present): {e:?}"))?;
        }

        let blit_info = BlitImageInfo {
            src_image: tab.image(),
            src_image_layout: ImageLayout::TransferSrcOptimal,
            dst_image: swapchain_image.clone(),
            dst_image_layout: ImageLayout::TransferDstOptimal,
            regions: vec![ImageBlit {
                src_subresource: ImageSubresourceLayers {
                    aspects: ImageAspects::COLOR,
                    mip_level: 0,
                    array_layers: 0..1,
                },
                src_offsets: [[0, 0, 0], [tab_w, tab_h, 1]],
                dst_subresource: ImageSubresourceLayers {
                    aspects: ImageAspects::COLOR,
                    mip_level: 0,
                    array_layers: 0..1,
                },
                dst_offsets: [[0, chrome_px.max(0) as u32, 0], [sw, chrome_px.max(0) as u32 + dst_h, 1]],
                ..Default::default()
            }]
            .into(),
            filter: Filter::Linear,
            ..BlitImageInfo::images(tab.image(), swapchain_image.clone())
        };
        unsafe {
            cmd_buf
                .blit_image(&blit_info)
                .map_err(|e| format!("blit_image (blit-present): {e:?}"))?;
        }

        let post_barrier = DependencyInfo {
            image_memory_barriers: vec![
                ImageMemoryBarrier {
                    src_stages: PipelineStages::ALL_COMMANDS,
                    src_access: AccessFlags::TRANSFER_WRITE,
                    dst_stages: PipelineStages::ALL_COMMANDS,
                    dst_access: AccessFlags::empty(),
                    old_layout: ImageLayout::TransferDstOptimal,
                    new_layout: ImageLayout::PresentSrc,
                    subresource_range: color_range.clone(),
                    ..ImageMemoryBarrier::image(swapchain_image.clone())
                },
                ImageMemoryBarrier {
                    src_stages: PipelineStages::ALL_COMMANDS,
                    src_access: AccessFlags::TRANSFER_READ,
                    dst_stages: PipelineStages::COLOR_ATTACHMENT_OUTPUT,
                    dst_access: AccessFlags::COLOR_ATTACHMENT_WRITE,
                    old_layout: ImageLayout::TransferSrcOptimal,
                    new_layout: ImageLayout::ColorAttachmentOptimal,
                    subresource_range: color_range,
                    ..ImageMemoryBarrier::image(tab.image())
                },
            ]
            .into(),
            ..Default::default()
        };
        unsafe {
            cmd_buf
                .pipeline_barrier(&post_barrier)
                .map_err(|e| format!("pipeline_barrier post (blit-present): {e:?}"))?;
        }
    } else {
        let present_barrier = DependencyInfo {
            image_memory_barriers: vec![ImageMemoryBarrier {
                src_stages: PipelineStages::COLOR_ATTACHMENT_OUTPUT,
                src_access: AccessFlags::COLOR_ATTACHMENT_WRITE,
                dst_stages: PipelineStages::ALL_COMMANDS,
                dst_access: AccessFlags::empty(),
                old_layout: ImageLayout::ColorAttachmentOptimal,
                new_layout: ImageLayout::PresentSrc,
                subresource_range: crate::vk_shared::color_subresource_range(),
                ..ImageMemoryBarrier::image(swapchain_image.clone())
            }]
            .into(),
            ..Default::default()
        };
        unsafe {
            cmd_buf
                .pipeline_barrier(&present_barrier)
                .map_err(|e| format!("pipeline_barrier present-only: {e:?}"))?;
        }
    }

    let command_buffer = unsafe { cmd_buf.end() }
        .map_err(|e| format!("end command buffer (blit-present): {e:?}"))?;

    let wait_semaphore_handles = [vk.image_available_semaphore.handle()];
    let wait_stage_masks = [ash::vk::PipelineStageFlags::ALL_COMMANDS];
    let command_buffers = [command_buffer.handle()];
    let signal_semaphores = [vk.render_finished_semaphore.handle()];
    let submit_info = ash::vk::SubmitInfo::default()
        .wait_semaphores(&wait_semaphore_handles)
        .wait_dst_stage_mask(&wait_stage_masks)
        .command_buffers(&command_buffers)
        .signal_semaphores(&signal_semaphores);

    let submit_res = unsafe {
        (vk.device.fns().v1_0.queue_submit)(
            vk.queue.handle(),
            1,
            &submit_info,
            ash::vk::Fence::null(),
        )
    };
    if submit_res != ash::vk::Result::SUCCESS {
        return Err(format!("queue_submit (blit-present): {submit_res:?}"));
    }

    vk.in_flight_tab_image = submitted_tab_image;

    let present_results = vk
        .queue
        .with(|mut q| unsafe {
            q.present(&PresentInfo {
                wait_semaphores: vec![SemaphorePresentInfo::new(vk.render_finished_semaphore.clone())],
                swapchain_infos: vec![SwapchainPresentInfo::swapchain_image_index(
                    vk.vk_swapchain.clone(),
                    vk.current_image_index,
                )],
                ..Default::default()
            })
        })
        .map_err(|e| format!("queue_present: {e:?}"))?;

    for result in present_results {
        match result {
            Ok(false) => {}
            Ok(true) => vk.swapchain_valid = false,
            Err(err) => {
                let msg = format!("queue_present result: {err:?}");
                if vk_error_is_surface_lost_message(&msg) {
                    vk.swapchain_valid = false;
                    vk_recreate_surface(vk, window.clone())?;
                } else if vk_error_is_recoverable_message(&msg) {
                    vk.swapchain_valid = false;
                } else {
                    return Err(msg);
                }
            }
        }
    }

    Ok(())
}

/// Recreate the Vulkan surface from the current native window handles.
fn vk_recreate_surface(vk: &mut VkState, window: Arc<Box<dyn Window>>) -> Result<(), String> {
    let _ = vk.queue.with(|mut q| q.wait_idle());

    vk.swapchain_images.clear();
    vk.skia_surfaces.clear();

    let new_surface = unsafe {
        vulkano::swapchain::Surface::from_window_ref(vk.instance.clone(), &*window)
    }
    .map_err(|e| format!("Surface::from_window_ref (recreate): {e:?}"))?;

    vk.surface = new_surface;
    vk.current_image_index = 0;
    vk.swapchain_valid = false;

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

    let physical_device = bootstrap.physical_device;
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
    let extent = clamp_surface_extent(&caps, window_size.width, window_size.height);

    let min_image_count = caps.min_image_count.max(2).min(caps.max_image_count.unwrap_or(u32::MAX));

    let swapchain_ci = SwapchainCreateInfo {
        flags: Default::default(),
        min_image_count,
        image_format: swapchain_format,
        image_view_formats: Default::default(),
        image_color_space: ColorSpace::SrgbNonLinear,
        image_extent: extent,
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

    let cmd_buf_allocator = Arc::new(StandardCommandBufferAllocator::new(
        vk_device.clone(),
        StandardCommandBufferAllocatorCreateInfo::default(),
    ));

    let mut gr_context = {
        let get_proc = SkiaGetProc::new(&vk_instance, &vk_device);
        let get_proc_fn = |of: GetProcOf| get_proc.resolve(of);

        let mut backend_context = unsafe {
            skia_safe::gpu::vk::BackendContext::new(
                crate::vk_shared::raw_instance_handle(&vk_instance) as _,
                crate::vk_shared::raw_physical_device_handle(&physical_device) as _,
                crate::vk_shared::raw_device_handle(&vk_device) as _,
                (crate::vk_shared::raw_queue_handle(&vk_queue_owner) as _, queue_family_index as usize),
                &get_proc_fn,
            )
        };
        backend_context.set_max_api_version(negotiated_api_version);

        skia_safe::gpu::direct_contexts::make_vulkan(&backend_context, None)
            .unwrap_or_else(|| {
                panic!(
                    "Failed to create Skia Vulkan DirectContext. GPU: {} (type {:?}), \
                     queue_family_index: {}, negotiated_api_version: 0x{:X}",
                    physical_device.properties().device_name,
                    physical_device.properties().device_type,
                    queue_family_index,
                    negotiated_api_version,
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
        queue_family_index,
        negotiated_api_version,
        instance: vk_instance,
        device: vk_device.clone(),
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
        cmd_buf_allocator,
        in_flight_tab_image: None,
    };

    Env {
        surface: initial_surface,
        gr_context,
        window,
        vk,
    }
}
