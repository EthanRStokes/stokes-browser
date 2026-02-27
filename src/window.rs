use ash::StaticFn;
use std::ptr;
use std::sync::Arc;
use ash::vk::Handle;
use vulkano::device::{Device, DeviceCreateInfo, DeviceExtensions, Queue, QueueCreateInfo, QueueFlags};
use vulkano::format::Format as VkFormat;
use vulkano::image::{Image, ImageUsage};
use vulkano::instance::{Instance, InstanceCreateFlags, InstanceCreateInfo};
use vulkano::swapchain::{
    acquire_next_image, PresentMode, Surface as VkSurface, Swapchain,
    SwapchainCreateInfo, SwapchainPresentInfo,
};
use vulkano::sync::{self, GpuFuture};
use vulkano::{Validated, VulkanError, VulkanLibrary, VulkanObject};
use vulkano::device::physical::PhysicalDeviceType;
use vulkano::image::view::ImageView;
use vulkano::render_pass::{Framebuffer, FramebufferCreateInfo, RenderPass};
use skia_safe::gpu::{self, DirectContext};
use skia_safe::{ColorType, Surface};
use vulkano::library::Loader;
use winit::dpi::LogicalSize;
use winit::window::{Window, WindowAttributes};
use winit_core::event_loop::ActiveEventLoop;
use winit_core::icon::{BadIcon, Icon, RgbaIcon};
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
        self.surface = self.vk.skia_surfaces[0].clone();
    }

    /// Present the rendered frame: acquires the next swapchain image, re-targets the
    /// Skia surface to it, flushes, and presents.
    pub fn present(&mut self) -> Result<(), String> {
        vk_present(&mut self.vk, &**self.window, &mut self.surface, &mut self.gr_context)
    }
}

/// Vulkan-specific state.
pub(crate) struct VkState {
    pub(crate) instance: Arc<Instance>,
    pub(crate) device: Arc<Device>,
    pub(crate) queue: Arc<Queue>,
    pub(crate) swapchain: Arc<Swapchain>,
    pub(crate) images: Vec<Arc<Image>>,
    pub(crate) render_pass: Arc<RenderPass>,
    pub(crate) framebuffers: Vec<Arc<Framebuffer>>,
    /// One pre-built Skia `Surface` per swapchain image — created once, reused every frame.
    pub(crate) skia_surfaces: Vec<Surface>,
    pub(crate) last_render: Option<Box<dyn GpuFuture>>,
    pub(crate) swapchain_valid: bool,
    /// Skia color type matching the swapchain format.
    pub(crate) color_type: ColorType,
    /// Skia/Vulkan format tag for `ImageInfo`.
    pub(crate) vk_format: skia_safe::gpu::vk::Format,
    /// Raw ash handles for external operations (e.g. importing tab VkImages).
    pub(crate) ash_instance: ash::Instance,
    pub(crate) ash_physical_device: ash::vk::PhysicalDevice,
    pub(crate) ash_device: ash::Device,
    /// Queue family index used when creating the device.
    pub(crate) queue_family_index: u32,
    /// The swapchain image format as a raw VkFormat integer.
    pub(crate) swapchain_vk_format: i32,
}

impl VkState {
    /// Build a `VulkanDeviceInfo` that tab processes can use to attach to the
    /// same physical device and share VkImages with the parent.
    pub(crate) fn device_info(&self) -> VulkanDeviceInfo {
        use ash::vk::Handle;
        VulkanDeviceInfo {
            physical_device_handle: self.ash_physical_device.as_raw(),
            queue_family_index: self.queue_family_index,
            image_format: self.swapchain_vk_format,
            parent_pid: std::process::id(),
        }
    }
}

// ---------------------------------------------------------------------------
// Vulkan helpers
// ---------------------------------------------------------------------------

/// Map a vulkano `Format` to the Skia equivalents needed by `ImageInfo`.
/// Returns `None` for formats Skia cannot handle directly.
fn vk_map_format(fmt: VkFormat) -> Option<(skia_safe::gpu::vk::Format, ColorType)> {
    match fmt {
        VkFormat::B8G8R8A8_UNORM => Some((skia_safe::gpu::vk::Format::B8G8R8A8_UNORM, ColorType::BGRA8888)),
        VkFormat::R8G8B8A8_UNORM => Some((skia_safe::gpu::vk::Format::R8G8B8A8_UNORM, ColorType::RGBA8888)),
        VkFormat::B8G8R8A8_SRGB  => Some((skia_safe::gpu::vk::Format::B8G8R8A8_SRGB,  ColorType::BGRA8888)),
        VkFormat::R8G8B8A8_SRGB  => Some((skia_safe::gpu::vk::Format::R8G8B8A8_SRGB,  ColorType::RGBA8888)),
        _ => None,
    }
}

/// Choose the best swapchain format, preferring UNORM over SRGB so Skia's maths stays linear.
fn vk_pick_format(
    physical_device: &vulkano::device::physical::PhysicalDevice,
    surface: &VkSurface,
) -> (VkFormat, skia_safe::gpu::vk::Format, ColorType) {
    let formats = physical_device
        .surface_formats(surface, Default::default())
        .expect("Failed to query surface formats");

    let preferred = [
        VkFormat::B8G8R8A8_UNORM,
        VkFormat::R8G8B8A8_UNORM,
        VkFormat::B8G8R8A8_SRGB,
        VkFormat::R8G8B8A8_SRGB,
    ];
    for want in preferred {
        if formats.iter().any(|(f, _)| *f == want) {
            if let Some((vf, ct)) = vk_map_format(want) {
                return (want, vf, ct);
            }
        }
    }
    for (f, _) in &formats {
        if let Some((vf, ct)) = vk_map_format(*f) {
            return (*f, vf, ct);
        }
    }
    panic!("No Skia-compatible Vulkan swapchain format found. Available: {formats:?}");
}

/// Build `Framebuffer` objects for every swapchain image.
fn vk_make_framebuffers(images: &[Arc<Image>], render_pass: &Arc<RenderPass>) -> Vec<Arc<Framebuffer>> {
    images
        .iter()
        .map(|image| {
            let view = ImageView::new_default(image.clone()).unwrap();
            Framebuffer::new(
                render_pass.clone(),
                FramebufferCreateInfo {
                    attachments: vec![view],
                    ..Default::default()
                },
            )
            .unwrap()
        })
        .collect()
}

/// Recreate the swapchain + framebuffers after a resize (sets `swapchain_valid = true`).
fn vk_prepare_swapchain(vk: &mut VkState, window: &dyn Window, gr_context: &mut DirectContext) {
    if let Some(last) = vk.last_render.as_mut() {
        last.cleanup_finished();
    }

    let sz = window.surface_size();
    if sz.width == 0 || sz.height == 0 || vk.swapchain_valid {
        return;
    }

    let (new_swapchain, new_images) = vk
        .swapchain
        .recreate(SwapchainCreateInfo {
            image_extent: sz.into(),
            ..vk.swapchain.create_info()
        })
        .expect("Failed to recreate Vulkan swapchain");

    vk.swapchain    = new_swapchain;
    vk.framebuffers = vk_make_framebuffers(&new_images, &vk.render_pass);
    vk.images       = new_images;
    vk.skia_surfaces = vk_build_surfaces(gr_context, &vk.framebuffers, vk.vk_format, vk.color_type);
    vk.swapchain_valid = true;
}

/// Build a Skia `Surface` that renders into a specific Vulkan `Framebuffer`.
fn vk_surface_for_framebuffer(
    gr_context: &mut DirectContext,
    framebuffer: Arc<Framebuffer>,
    vk_format: skia_safe::gpu::vk::Format,
    color_type: ColorType,
) -> Surface {
    let [width, height] = framebuffer.extent();
    let image_handle = framebuffer.attachments()[0].image().handle().as_raw();

    let alloc = skia_safe::gpu::vk::Alloc::default();
    let image_info = unsafe {
        skia_safe::gpu::vk::ImageInfo::new(
            image_handle as _,
            alloc,
            skia_safe::gpu::vk::ImageTiling::OPTIMAL,
            skia_safe::gpu::vk::ImageLayout::UNDEFINED,
            vk_format,
            1,
            None,
            None,
            None,
            None,
        )
    };

    use skia_safe::gpu::backend_render_targets;
    let render_target = backend_render_targets::make_vk(
        (width as i32, height as i32),
        &image_info,
    );

    skia_safe::gpu::surfaces::wrap_backend_render_target(
        gr_context,
        &render_target,
        gpu::SurfaceOrigin::TopLeft,
        color_type,
        None,
        None,
    )
    .expect("Failed to wrap Vulkan backend render target")
}

/// Pre-allocate one Skia `Surface` for every framebuffer in the swapchain.
fn vk_build_surfaces(
    gr_context: &mut DirectContext,
    framebuffers: &[Arc<Framebuffer>],
    vk_format: skia_safe::gpu::vk::Format,
    color_type: ColorType,
) -> Vec<Surface> {
    framebuffers
        .iter()
        .map(|fb| vk_surface_for_framebuffer(gr_context, fb.clone(), vk_format, color_type))
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

    let (image_index, suboptimal, acquire_future) =
        match acquire_next_image(vk.swapchain.clone(), None).map_err(Validated::unwrap) {
            Ok(r) => r,
            Err(VulkanError::OutOfDate) => {
                vk.swapchain_valid = false;
                return Ok(());
            }
            Err(e) => return Err(format!("Vulkan acquire_next_image failed: {e}")),
        };

    if suboptimal {
        vk.swapchain_valid = false;
    }

    *current_surface = vk.skia_surfaces[image_index as usize].clone();

    gr_context.flush_and_submit();

    let present_future = vk
        .last_render
        .take()
        .unwrap()
        .join(acquire_future)
        .then_swapchain_present(
            vk.queue.clone(),
            SwapchainPresentInfo::swapchain_image_index(vk.swapchain.clone(), image_index),
        )
        .then_signal_fence_and_flush();

    vk.last_render = match present_future {
        Ok(f) => Some(Box::new(f) as _),
        Err(Validated::Error(VulkanError::OutOfDate)) => {
            vk.swapchain_valid = false;
            Some(sync::now(vk.device.clone()).boxed())
        }
        Err(e) => return Err(format!("Vulkan present failed: {e}")),
    };

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
                Err(_) => None
            }
        })
    };

    let win_attrs = WindowAttributes::default()
        .with_title("Stokes Browser")
        .with_min_surface_size(LogicalSize::new(1280u32, 800u32))
        .with_window_icon(icon);

    let window: Arc<Box<dyn winit_core::window::Window>> = Arc::new(
        el.create_window(win_attrs).expect("Failed to create window"),
    );

    // ── 2. Vulkano Instance ──────────────────────────────────────────────────
    let library = VulkanLibrary::new().expect("Vulkan library not found");

    // Collect required instance extensions from the event loop (surface extensions).
    let required_extensions = VkSurface::required_extensions(el.as_ref())
        .expect("Failed to query required Vulkan surface extensions");

    let instance = Instance::new(
        library.clone(),
        InstanceCreateInfo {
            flags: InstanceCreateFlags::ENUMERATE_PORTABILITY,
            enabled_extensions: required_extensions,
            ..Default::default()
        },
    )
    .expect("Failed to create Vulkan instance");

    // ── 3. Vulkan surface from the winit window ──────────────────────────────
    let vk_surface = VkSurface::from_window(instance.clone(), window.clone())
        .expect("Failed to create Vulkan surface");

    // ── 4. Physical device + queue family ────────────────────────────────────
    let (physical_device, queue_family_index) = instance
        .enumerate_physical_devices()
        .expect("Failed to enumerate physical devices")
        .filter(|p| {
            p.supported_extensions().khr_swapchain
        })
        .filter_map(|p| {
            p.queue_family_properties()
                .iter()
                .enumerate()
                .position(|(i, q)| {
                    q.queue_flags.contains(QueueFlags::GRAPHICS)
                        && p.surface_support(i as u32, &vk_surface).unwrap_or(false)
                })
                .map(|i| (p, i as u32))
        })
        .min_by_key(|(p, _)| match p.properties().device_type {
            PhysicalDeviceType::DiscreteGpu   => 0,
            PhysicalDeviceType::IntegratedGpu => 1,
            PhysicalDeviceType::VirtualGpu    => 2,
            PhysicalDeviceType::Cpu           => 3,
            _                                 => 4,
        })
        .expect("No suitable Vulkan physical device found");

    // ── 5. Logical device + queue ────────────────────────────────────────────
    let device_extensions = DeviceExtensions {
        khr_swapchain: true,
        ..DeviceExtensions::empty()
    };

    let (device, mut queues) = Device::new(
        physical_device.clone(),
        DeviceCreateInfo {
            queue_create_infos: vec![QueueCreateInfo {
                queue_family_index,
                ..Default::default()
            }],
            enabled_extensions: device_extensions,
            ..Default::default()
        },
    )
    .expect("Failed to create Vulkan logical device");

    let queue = queues.next().unwrap();

    // ── 6. Swapchain ─────────────────────────────────────────────────────────
    let (vk_format, skia_vk_format, color_type) =
        vk_pick_format(&physical_device, &vk_surface);

    let surface_capabilities = physical_device
        .surface_capabilities(&vk_surface, Default::default())
        .expect("Failed to query surface capabilities");

    let window_size = window.surface_size();
    let image_extent: [u32; 2] = window_size.into();

    let (swapchain, images) = Swapchain::new(
        device.clone(),
        vk_surface.clone(),
        SwapchainCreateInfo {
            min_image_count: surface_capabilities.min_image_count.max(2),
            image_format: vk_format,
            image_extent,
            image_usage: ImageUsage::COLOR_ATTACHMENT,
            present_mode: PresentMode::Fifo,
            ..Default::default()
        },
    )
    .expect("Failed to create Vulkan swapchain");

    // ── 7. Render pass + framebuffers ────────────────────────────────────────
    let render_pass = vulkano::single_pass_renderpass!(
        device.clone(),
        attachments: {
            color: {
                format: vk_format,
                samples: 1,
                load_op: DontCare,
                store_op: Store,
            },
        },
        pass: {
            color: [color],
            depth_stencil: {},
        },
    )
    .expect("Failed to create render pass");

    let framebuffers = vk_make_framebuffers(&images, &render_pass);

    // ── 8. Skia DirectContext via vulkano raw handles ────────────────────────
    let raw_instance  = instance.handle().as_raw();
    let raw_phys_dev  = physical_device.handle().as_raw();
    let raw_device    = device.handle().as_raw();
    let raw_queue     = queue.handle().as_raw();

    // We need the ash function-pointer tables so Skia can call Vulkan.
    // Vulkano loads the library internally; we can borrow the function pointers via
    // the underlying VulkanLibrary loader.
    let get_instance_proc_addr: ash::vk::PFN_vkGetInstanceProcAddr = unsafe {
        std::mem::transmute(library.get_instance_proc_addr(Default::default(), ()))
    };

    let ash_entry = unsafe {
        ash::Entry::from_static_fn(StaticFn {
            get_instance_proc_addr,
        })
    };

    let ash_instance = unsafe {
        ash::Instance::load(
            ash_entry.static_fn(),
            ash::vk::Instance::from_raw(raw_instance),
        )
    };

    let ash_physical_device = ash::vk::PhysicalDevice::from_raw(raw_phys_dev);

    let ash_device = unsafe {
        ash::Device::load(
            ash_instance.fp_v1_0(),
            ash::vk::Device::from_raw(raw_device),
        )
    };

    let get_device_proc_addr = ash_instance.fp_v1_0().get_device_proc_addr;

    let get_proc = |of: skia_safe::gpu::vk::GetProcOf| unsafe {
        match of {
            skia_safe::gpu::vk::GetProcOf::Instance(inst_raw, name) => {
                let vk_inst = ash::vk::Instance::from_raw(inst_raw as _);
                ash_entry.get_instance_proc_addr(vk_inst, name)
            }
            skia_safe::gpu::vk::GetProcOf::Device(dev_raw, name) => {
                let vk_dev = ash::vk::Device::from_raw(dev_raw as _);
                get_device_proc_addr(vk_dev, name)
            }
        }
        .map(|f| f as _)
        .unwrap_or(ptr::null())
    };

    let backend_context = unsafe {
        skia_safe::gpu::vk::BackendContext::new(
            raw_instance as _,
            raw_phys_dev as _,
            raw_device as _,
            (raw_queue as _, queue_family_index as usize),
            &get_proc,
        )
    };

    let mut gr_context =
        skia_safe::gpu::direct_contexts::make_vulkan(&backend_context, None)
            .expect("Failed to create Skia Vulkan DirectContext");

    // ── 9. Skia surfaces (one per swapchain image) ───────────────────────────
    let skia_surfaces = vk_build_surfaces(&mut gr_context, &framebuffers, skia_vk_format, color_type);
    let initial_surface = skia_surfaces[0].clone();

    let swapchain_vk_format = vk_format;

    let last_render: Option<Box<dyn GpuFuture>> =
        Some(sync::now(device.clone()).boxed());

    let vk = VkState {
        instance,
        device,
        queue,
        swapchain,
        images,
        render_pass,
        framebuffers,
        skia_surfaces,
        last_render,
        swapchain_valid: true,
        color_type,
        vk_format: skia_vk_format,
        ash_instance,
        ash_physical_device,
        ash_device,
        queue_family_index,
        swapchain_vk_format,
    };

    Env {
        surface: initial_surface,
        gr_context,
        window,
        vk,
    }
}

