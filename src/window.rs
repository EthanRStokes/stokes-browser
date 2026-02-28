use std::ptr;
use std::sync::Arc;
use ash::vk::{self, Handle};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use skia_safe::gpu::{self, DirectContext};
use skia_safe::{ColorType, Surface};
use winit::dpi::LogicalSize;
use winit::window::{Window, WindowAttributes};
use winit_core::event_loop::ActiveEventLoop;
use winit_core::icon::{Icon, RgbaIcon};
use crate::vk_shared::VulkanDeviceInfo;

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
        // (Skia already recorded commands via the DirectContext; we just need the semaphore sync.)
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

    // Collect required instance extensions for surface creation.
    let display_handle = el.as_ref().display_handle().expect("Failed to get display handle");
    let required_extensions = ash_window::enumerate_required_extensions(display_handle.as_raw())
        .expect("Failed to enumerate required Vulkan extensions")
        .to_vec();

    let app_info = vk::ApplicationInfo::default()
        .application_name(c"stokes-browser")
        .api_version(vk::API_VERSION_1_1);

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

    let device_extensions = [
        ash::khr::swapchain::NAME.as_ptr(),
        ash::khr::external_memory::NAME.as_ptr(),
        #[cfg(windows)]
        ash::khr::external_memory_win32::NAME.as_ptr(),
        #[cfg(not(windows))]
        ash::khr::external_memory_fd::NAME.as_ptr(),
        #[cfg(not(windows))]
        ash::vk::EXT_EXTERNAL_MEMORY_DMA_BUF_NAME.as_ptr(),
    ];

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

    // ── 8. Skia DirectContext via ash raw handles ────────────────────────────
    // Extract the raw function pointers *before* the closure so `entry` is not
    // captured and can still be moved into VkState afterwards.
    let get_instance_proc_addr = entry.static_fn().get_instance_proc_addr;
    let get_device_proc_addr = instance.fp_v1_0().get_device_proc_addr;

    let get_proc = |of: skia_safe::gpu::vk::GetProcOf| unsafe {
        match of {
            skia_safe::gpu::vk::GetProcOf::Instance(inst_raw, name) => {
                let vk_inst = vk::Instance::from_raw(inst_raw as usize as u64);
                (get_instance_proc_addr)(vk_inst, name)
            }
            skia_safe::gpu::vk::GetProcOf::Device(dev_raw, name) => {
                let vk_dev = vk::Device::from_raw(dev_raw as usize as u64);
                (get_device_proc_addr)(vk_dev, name)
            }
        }
        .map(|f| std::mem::transmute(f))
        .unwrap_or(ptr::null())
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

