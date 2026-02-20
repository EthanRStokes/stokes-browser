use gl::types::GLint;
use glutin::config::{ConfigTemplateBuilder, GlConfig};
use glutin::context::{ContextApi, ContextAttributesBuilder, NotCurrentGlContext, PossiblyCurrentContext};
use glutin::display::{GetGlDisplay, GlDisplay};
use glutin::surface::{GlSurface, Surface as GlutinSurface, SurfaceAttributesBuilder, WindowSurface};
use glutin_winit::{ApiPreference, DisplayBuilder};
use skia_safe::gpu::gl::{Format, FramebufferInfo, Interface};
use skia_safe::gpu::surfaces::wrap_backend_render_target;
use skia_safe::gpu::{backend_render_targets, DirectContext};
use skia_safe::{gpu, ColorType, Surface};
use std::ffi::CString;
use std::num::NonZeroU32;
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
use winit::dpi::LogicalSize;
use winit::event_loop::EventLoop;
use winit::raw_window_handle::HasWindowHandle;
use winit::window::{Window, WindowAttributes};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Which GPU backend is currently active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    OpenGl,
    Vulkan,
}

/// OpenGL-specific state.
pub(crate) struct GlState {
    pub(crate) gl_surface: GlutinSurface<WindowSurface>,
    pub(crate) gl_context: PossiblyCurrentContext,
    pub(crate) fb_info: FramebufferInfo,
    pub(crate) num_samples: usize,
    pub(crate) stencil_size: usize,
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
}

/// Backend-specific rendering state (either OpenGL or Vulkan).
pub(crate) enum BackendState {
    Gl(GlState),
    Vk(VkState),
}

impl BackendState {
    pub fn kind(&self) -> BackendKind {
        match self {
            BackendState::Gl(_) => BackendKind::OpenGl,
            BackendState::Vk(_) => BackendKind::Vulkan,
        }
    }
}

/// Main environment holding the window, the Skia surface + context, and the active backend.
pub(crate) struct Env {
    pub(crate) surface: Surface,
    pub(crate) gr_context: DirectContext,
    pub(crate) window: Arc<Window>,
    pub(crate) backend: BackendState,
}

impl Env {
    pub fn backend_kind(&self) -> BackendKind {
        self.backend.kind()
    }

    /// Swap to the opposite backend (OpenGL ↔ Vulkan).
    ///
    /// Abandons the old Skia context before tearing down the underlying GPU objects, then
    /// builds a fresh context for the new backend.
    pub fn swap_backend(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        match self.backend.kind() {
            BackendKind::OpenGl => {
                self.gr_context.abandon();
                let (vk_state, mut new_ctx) = vk_backend_init(self.window.clone(), event_loop);
                let initial_surface = vk_state.skia_surfaces[0].clone();
                self.backend = BackendState::Vk(vk_state);
                self.gr_context = new_ctx;
                self.surface = initial_surface;
            }
            BackendKind::Vulkan => {
                self.gr_context.abandon();
                panic!(
                    "Swapping from Vulkan → OpenGL at runtime is not supported \
                     (glutin requires the event loop to create a GL context for an existing \
                     window). Start the application with the OpenGL backend instead."
                );
            }
        }
    }

    /// Recreate the Skia surface after a window resize.
    pub fn recreate_surface(&mut self) {
        match &mut self.backend {
            BackendState::Gl(gl) => {
                let (w, h) = self.window.inner_size().into();
                let _ = gl.gl_surface.resize(
                    &gl.gl_context,
                    NonZeroU32::new(u32::max(w, 1)).unwrap(),
                    NonZeroU32::new(u32::max(h, 1)).unwrap(),
                );
                self.surface = gl_surface_from_framebuffer(&self.window, gl, &mut self.gr_context);
            }
            BackendState::Vk(vk) => {
                vk.swapchain_valid = false;
                vk_prepare_swapchain(vk, &self.window, &mut self.gr_context);
                // Point env.surface at index 0 as a safe placeholder until the next present.
                self.surface = vk.skia_surfaces[0].clone();
            }
        }
    }

    /// Present the rendered frame.
    ///
    /// - **OpenGL**: swaps the front/back buffers via glutin.
    /// - **Vulkan**: acquires the next swapchain image, re-targets the Skia surface to it,
    ///   flushes, and presents.
    pub fn present(&mut self) -> Result<(), String> {
        match &mut self.backend {
            BackendState::Gl(gl) => gl
                .gl_surface
                .swap_buffers(&gl.gl_context)
                .map_err(|e| format!("GL swap_buffers failed: {e}")),
            BackendState::Vk(vk) => {
                vk_present(vk, &self.window, &mut self.surface, &mut self.gr_context)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// GL helpers
// ---------------------------------------------------------------------------

/// Wrap the current GL framebuffer as a Skia `Surface`.
fn gl_surface_from_framebuffer(
    window: &Window,
    gl: &GlState,
    gr_context: &mut DirectContext,
) -> Surface {
    let size = window.inner_size();
    let size = (
        size.width.try_into().expect("Could not convert width"),
        size.height.try_into().expect("Could not convert height"),
    );
    let brt = backend_render_targets::make_gl(size, gl.num_samples, gl.stencil_size, gl.fb_info);
    wrap_backend_render_target(
        gr_context,
        &brt,
        gpu::SurfaceOrigin::BottomLeft,
        ColorType::RGBA8888,
        None,
        None,
    )
    .expect("Failed to wrap GL backend render target")
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
/// The caller is responsible for rebuilding `skia_surfaces` afterward via `vk_build_surfaces`.
fn vk_prepare_swapchain(vk: &mut VkState, window: &Window, gr_context: &mut DirectContext) {
    if let Some(last) = vk.last_render.as_mut() {
        last.cleanup_finished();
    }

    let sz = window.inner_size();
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
    // Rebuild the per-image Skia surfaces for the new framebuffer sizes.
    vk.skia_surfaces = vk_build_surfaces(gr_context, &vk.framebuffers, vk.vk_format, vk.color_type);
    vk.swapchain_valid = true;
}

/// Build a Skia `Surface` that renders into a specific Vulkan `Framebuffer`.
///
/// **Image layout must be `UNDEFINED`** when Skia first wraps an image it hasn't seen
/// before.  Skia records its own layout transitions internally; providing
/// `COLOR_ATTACHMENT_OPTIMAL` at wrap time makes `wrap_backend_render_target` return `None`
/// because Skia cannot reconcile an already-transitioned image with its own state machine.
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
            // UNDEFINED — lets Skia own the very first layout transition.
            skia_safe::gpu::vk::ImageLayout::UNDEFINED,
            vk_format,
            1,    // mip-level count
            None, // current queue family  (none / external)
            None, // protected image
            None, // YCbCr conversion info
            None, // sharing mode
        )
    };

    let render_target = backend_render_targets::make_vk(
        (width as i32, height as i32),
        &image_info,
    );

    skia_safe::gpu::surfaces::wrap_backend_render_target(
        gr_context,
        &render_target,
        gpu::SurfaceOrigin::TopLeft,
        color_type,
        None, // colour space — let Skia pick
        None,
    )
    .expect("Failed to wrap Vulkan backend render target")
}

/// Pre-allocate one Skia `Surface` for every framebuffer in the swapchain.
/// Call this once at startup and again whenever the swapchain is recreated.
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

/// Return the placeholder surface (index 0) — only used as the initial `Env::surface`
/// value before the first frame is presented.
fn vk_placeholder_surface(gr_context: &mut DirectContext, vk: &VkState) -> Surface {
    vk_surface_for_framebuffer(
        gr_context,
        vk.framebuffers[0].clone(),
        vk.vk_format,
        vk.color_type,
    )
}

/// Acquire the next swapchain image, redirect the Skia surface to it, flush, and present.
fn vk_present(
    vk: &mut VkState,
    window: &Window,
    current_surface: &mut Surface,
    gr_context: &mut DirectContext,
) -> Result<(), String> {
    // Ensure swapchain is valid (may have been invalidated by a resize).
    if !vk.swapchain_valid {
        vk_prepare_swapchain(vk, window, gr_context);
    }

    // Acquire the image to render into.
    let (image_index, suboptimal, acquire_future) =
        match acquire_next_image(vk.swapchain.clone(), None).map_err(Validated::unwrap) {
            Ok(r) => r,
            Err(VulkanError::OutOfDate) => {
                vk.swapchain_valid = false;
                return Ok(()); // will retry next frame
            }
            Err(e) => return Err(format!("Vulkan acquire_next_image failed: {e}")),
        };

    if suboptimal {
        vk.swapchain_valid = false;
    }

    // Point env.surface at the pre-built surface for this image — O(1), no allocation.
    *current_surface = vk.skia_surfaces[image_index as usize].clone();

    // Flush all pending Skia commands into the image.
    gr_context.flush_and_submit();

    // Submit presentation.
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

/// Create all Vulkan objects (instance, device, queue, swapchain) and build the Skia
/// `DirectContext`.  Returns `(VkState, DirectContext)` so the caller can assemble `Env`.
fn vk_backend_init(
    window: Arc<Window>,
    event_loop: &winit::event_loop::ActiveEventLoop,
) -> (VkState, DirectContext) {
    let library = VulkanLibrary::new().expect("Vulkan library not found");

    let required_extensions =
        VkSurface::required_extensions(event_loop).expect("Could not get required Vulkan extensions");

    let instance = Instance::new(
        library.clone(),
        InstanceCreateInfo {
            flags: InstanceCreateFlags::ENUMERATE_PORTABILITY,
            enabled_extensions: required_extensions,
            ..Default::default()
        },
    )
    .expect("Failed to create Vulkan instance");

    let surface =
        VkSurface::from_window(instance.clone(), window.clone())
            .expect("Failed to create Vulkan surface");

    let device_extensions = DeviceExtensions {
        khr_swapchain: true,
        ..DeviceExtensions::empty()
    };

    let (physical_device, queue_family_index) = instance
        .enumerate_physical_devices()
        .expect("Could not enumerate physical devices")
        .filter(|p| p.supported_extensions().contains(&device_extensions))
        .filter_map(|p| {
            p.queue_family_properties()
                .iter()
                .enumerate()
                .position(|(i, q)| {
                    q.queue_flags.intersects(QueueFlags::GRAPHICS)
                        && p.surface_support(i as u32, &surface).unwrap_or(false)
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

    println!(
        "Vulkan: using device '{}' ({:?})",
        physical_device.properties().device_name,
        physical_device.properties().device_type,
    );

    let (device, mut queues) = Device::new(
        physical_device.clone(),
        DeviceCreateInfo {
            enabled_extensions: device_extensions,
            queue_create_infos: vec![QueueCreateInfo {
                queue_family_index,
                ..Default::default()
            }],
            ..Default::default()
        },
    )
    .expect("Failed to create Vulkan device");

    let queue = queues.next().unwrap();

    let window_size        = window.inner_size();
    let surface_caps       = physical_device
        .surface_capabilities(&surface, Default::default())
        .unwrap();

    let (image_format, vk_format, color_type) = vk_pick_format(&physical_device, &surface);
    println!("Vulkan: swapchain format = {image_format:?}  skia color_type = {color_type:?}");

    let (swapchain, images) = Swapchain::new(
        device.clone(),
        surface,
        SwapchainCreateInfo {
            min_image_count: surface_caps.min_image_count.max(2),
            image_extent: window_size.into(),
            // COLOR_ATTACHMENT is mandatory; TRANSFER_SRC/DST lets Skia blit.
            image_usage: ImageUsage::COLOR_ATTACHMENT
                | ImageUsage::TRANSFER_SRC
                | ImageUsage::TRANSFER_DST,
            image_format,
            present_mode: PresentMode::Fifo,
            composite_alpha: surface_caps
                .supported_composite_alpha
                .into_iter()
                .next()
                .unwrap(),
            ..Default::default()
        },
    )
    .expect("Failed to create Vulkan swapchain");

    let render_pass = vulkano::single_pass_renderpass!(
        device.clone(),
        attachments: {
            color: {
                format: swapchain.image_format(),
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
    .unwrap();

    let framebuffers = vk_make_framebuffers(&images, &render_pass);
    let last_render: Option<Box<dyn GpuFuture>> = Some(sync::now(device.clone()).boxed());

    // Build the Skia Vulkan DirectContext by providing proc-address lookup callbacks.
    let mut skia_ctx = unsafe {
        let get_device_proc_addr = instance.fns().v1_0.get_device_proc_addr;

        let get_proc = |gpo: skia_safe::gpu::vk::GetProcOf| {
            match gpo {
                skia_safe::gpu::vk::GetProcOf::Instance(raw_instance, name) => {
                    let vk_instance = ash::vk::Instance::from_raw(raw_instance as _);
                    library.get_instance_proc_addr(vk_instance, name)
                }
                skia_safe::gpu::vk::GetProcOf::Device(raw_device, name) => {
                    let vk_device = ash::vk::Device::from_raw(raw_device as _);
                    get_device_proc_addr(vk_device, name)
                }
            }
            .map(|f| f as _)
            .unwrap_or_else(|| {
                eprintln!(
                    "Vulkan: failed to resolve {:?}",
                    gpo.name().to_str().unwrap_or("<invalid>")
                );
                ptr::null()
            })
        };

        gpu::direct_contexts::make_vulkan(
            &skia_safe::gpu::vk::BackendContext::new(
                instance.handle().as_raw() as _,
                physical_device.handle().as_raw() as _,
                device.handle().as_raw() as _,
                (
                    queue.handle().as_raw() as _,
                    queue.queue_family_index() as usize,
                ),
                &get_proc,
            ),
            None,
        )
        .expect("Failed to create Skia Vulkan DirectContext")
    };

    let skia_surfaces = vk_build_surfaces(&mut skia_ctx, &framebuffers, vk_format, color_type);

    let vk_state = VkState {
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
        vk_format,
    };

    (vk_state, skia_ctx)
}

// ---------------------------------------------------------------------------
// Public window constructors
// ---------------------------------------------------------------------------

/// Create the main window using the **OpenGL** backend (default).
pub(crate) fn create_window(el: &EventLoop<()>) -> Env {
    let icon_data = include_bytes!("../assets/com.ethanstokes.stokes-browser.png");
    let icon = image::load_from_memory(icon_data)
        .expect("Failed to load icon")
        .into_rgba8();
    let (icon_width, icon_height) = icon.dimensions();
    let icon = winit::window::Icon::from_rgba(icon.into_raw(), icon_width, icon_height)
        .expect("Failed to create icon");

    let window_attrs = WindowAttributes::default()
        .with_title("Web Browser")
        .with_inner_size(LogicalSize::new(1024, 768))
        .with_min_inner_size(LogicalSize::new(500, crate::ui::BrowserUI::CHROME_HEIGHT as i32))
        .with_window_icon(Some(icon));

    let template = ConfigTemplateBuilder::new()
        .with_alpha_size(8)
        .with_transparency(true);

    let display_builder = DisplayBuilder::new()
        .with_preference(ApiPreference::PreferEgl)
        .with_window_attributes(window_attrs.into());

    let (window, gl_config) = display_builder
        .build(el, template, |configs| {
            configs
                .reduce(|accum, config| {
                    let transparency_check = config.supports_transparency().unwrap_or(false)
                        & !accum.supports_transparency().unwrap_or(false);
                    if transparency_check || config.num_samples() < accum.num_samples() {
                        config
                    } else {
                        accum
                    }
                })
                .unwrap()
        })
        .unwrap();

    let window = Arc::new(window.expect("Could not create window with OpenGL context."));
    let window_handle = window.window_handle().expect("Failed to retrieve RawWindowHandle");
    let raw_window_handle = window_handle.as_raw();

    let context_attributes = ContextAttributesBuilder::new().build(Some(raw_window_handle));
    let fallback_context_attributes = ContextAttributesBuilder::new()
        .with_context_api(ContextApi::Gles(None))
        .build(Some(raw_window_handle));

    let not_current_gl_context = unsafe {
        gl_config
            .display()
            .create_context(&gl_config, &context_attributes)
            .unwrap_or_else(|_| {
                gl_config
                    .display()
                    .create_context(&gl_config, &fallback_context_attributes)
                    .expect("failed to create context")
            })
    };

    let (width, height) = window.inner_size().into();
    let attrs = SurfaceAttributesBuilder::<WindowSurface>::new().build(
        raw_window_handle,
        NonZeroU32::new(width).unwrap(),
        NonZeroU32::new(height).unwrap(),
    );

    let gl_surface = unsafe {
        gl_config
            .display()
            .create_window_surface(&gl_config, &attrs)
            .expect("Failed to create GL surface")
    };

    let gl_context = not_current_gl_context
        .make_current(&gl_surface)
        .expect("Failed to make GL context current");

    gl::load_with(|s| {
        gl_config
            .display()
            .get_proc_address(CString::new(s).unwrap().as_c_str())
    });

    let interface = Interface::new_load_with(|name| {
        if name == "eglGetCurrentDisplay" {
            return std::ptr::null();
        }
        gl_config
            .display()
            .get_proc_address(CString::new(name).unwrap().as_c_str())
    })
    .expect("Could not create GL interface");

    let context_options = gpu::ContextOptions::default();
    let mut gr_context = gpu::direct_contexts::make_gl(interface, Some(&context_options))
        .expect("Failed to create Skia GL context");

    let fb_info = {
        let mut fboid: GLint = 0;
        unsafe { gl::GetIntegerv(gl::FRAMEBUFFER_BINDING, &mut fboid) };
        FramebufferInfo {
            fboid: fboid.try_into().unwrap(),
            format: Format::RGBA8.into(),
            ..Default::default()
        }
    };

    let num_samples  = gl_config.num_samples()  as usize;
    let stencil_size = gl_config.stencil_size()  as usize;

    let gl_state = GlState { gl_surface, gl_context, fb_info, num_samples, stencil_size };
    let surface  = gl_surface_from_framebuffer(&window, &gl_state, &mut gr_context);

    Env { surface, gr_context, window, backend: BackendState::Gl(gl_state) }
}

/// Create the main window using the **Vulkan** backend.
pub(crate) fn create_window_vk(el: &EventLoop<()>) -> Env {
    let icon_data = include_bytes!("../assets/com.ethanstokes.stokes-browser.png");
    let icon = image::load_from_memory(icon_data)
        .expect("Failed to load icon")
        .into_rgba8();
    let (icon_width, icon_height) = icon.dimensions();
    let icon = winit::window::Icon::from_rgba(icon.into_raw(), icon_width, icon_height)
        .expect("Failed to create icon");

    let window_attrs = WindowAttributes::default()
        .with_title("Web Browser")
        .with_inner_size(LogicalSize::new(1024, 768))
        .with_min_inner_size(LogicalSize::new(500, crate::ui::BrowserUI::CHROME_HEIGHT as i32))
        .with_window_icon(Some(icon));

    // Vulkan doesn't need glutin — create the window directly through winit.
    let window = Arc::new(
        el.create_window(window_attrs)
            .expect("Failed to create window for Vulkan"),
    );

    // `VkSurface::required_extensions` accepts anything implementing `HasDisplayHandle`.
    let required_extensions = {
        use winit::raw_window_handle::HasDisplayHandle;
        VkSurface::required_extensions(el).expect("Could not get required Vulkan extensions")
    };

    let library = VulkanLibrary::new().expect("Vulkan library not found");

    let instance = Instance::new(
        library.clone(),
        InstanceCreateInfo {
            flags: InstanceCreateFlags::ENUMERATE_PORTABILITY,
            enabled_extensions: required_extensions,
            ..Default::default()
        },
    )
    .expect("Failed to create Vulkan instance");

    let vk_surface =
        VkSurface::from_window(instance.clone(), window.clone())
            .expect("Failed to create Vulkan surface");

    let device_extensions = DeviceExtensions {
        khr_swapchain: true,
        ..DeviceExtensions::empty()
    };

    let (physical_device, queue_family_index) = instance
        .enumerate_physical_devices()
        .expect("Could not enumerate physical devices")
        .filter(|p| p.supported_extensions().contains(&device_extensions))
        .filter_map(|p| {
            p.queue_family_properties()
                .iter()
                .enumerate()
                .position(|(i, q)| {
                    q.queue_flags.intersects(QueueFlags::GRAPHICS)
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

    println!(
        "Vulkan: using device '{}' ({:?})",
        physical_device.properties().device_name,
        physical_device.properties().device_type,
    );

    let (device, mut queues) = Device::new(
        physical_device.clone(),
        DeviceCreateInfo {
            enabled_extensions: device_extensions,
            queue_create_infos: vec![QueueCreateInfo {
                queue_family_index,
                ..Default::default()
            }],
            ..Default::default()
        },
    )
    .expect("Failed to create Vulkan device");

    let queue       = queues.next().unwrap();
    let window_size = window.inner_size();
    let surface_caps = physical_device
        .surface_capabilities(&vk_surface, Default::default())
        .unwrap();

    let (image_format, vk_format, color_type) = vk_pick_format(&physical_device, &vk_surface);
    println!("Vulkan: swapchain format = {image_format:?}  skia color_type = {color_type:?}");

    let (swapchain, images) = Swapchain::new(
        device.clone(),
        vk_surface,
        SwapchainCreateInfo {
            min_image_count: surface_caps.min_image_count.max(2),
            image_extent: window_size.into(),
            image_usage: ImageUsage::COLOR_ATTACHMENT
                | ImageUsage::TRANSFER_SRC
                | ImageUsage::TRANSFER_DST,
            image_format,
            present_mode: PresentMode::Fifo,
            composite_alpha: surface_caps
                .supported_composite_alpha
                .into_iter()
                .next()
                .unwrap(),
            ..Default::default()
        },
    )
    .expect("Failed to create Vulkan swapchain");

    let render_pass = vulkano::single_pass_renderpass!(
        device.clone(),
        attachments: {
            color: {
                format: swapchain.image_format(),
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
    .unwrap();

    let framebuffers = vk_make_framebuffers(&images, &render_pass);
    let last_render: Option<Box<dyn GpuFuture>> = Some(sync::now(device.clone()).boxed());

    let mut skia_ctx = unsafe {
        let get_device_proc_addr = instance.fns().v1_0.get_device_proc_addr;

        let get_proc = |gpo: skia_safe::gpu::vk::GetProcOf| {
            match gpo {
                skia_safe::gpu::vk::GetProcOf::Instance(raw_instance, name) => {
                    let vk_instance = ash::vk::Instance::from_raw(raw_instance as _);
                    library.get_instance_proc_addr(vk_instance, name)
                }
                skia_safe::gpu::vk::GetProcOf::Device(raw_device, name) => {
                    let vk_device = ash::vk::Device::from_raw(raw_device as _);
                    get_device_proc_addr(vk_device, name)
                }
            }
            .map(|f| f as _)
            .unwrap_or_else(|| {
                eprintln!(
                    "Vulkan: failed to resolve {:?}",
                    gpo.name().to_str().unwrap_or("<invalid>")
                );
                ptr::null()
            })
        };

        gpu::direct_contexts::make_vulkan(
            &skia_safe::gpu::vk::BackendContext::new(
                instance.handle().as_raw() as _,
                physical_device.handle().as_raw() as _,
                device.handle().as_raw() as _,
                (
                    queue.handle().as_raw() as _,
                    queue.queue_family_index() as usize,
                ),
                &get_proc,
            ),
            None,
        )
        .expect("Failed to create Skia Vulkan DirectContext")
    };

    let skia_surfaces = vk_build_surfaces(&mut skia_ctx, &framebuffers, vk_format, color_type);
    // Use index 0 as a safe placeholder; vk_present will redirect to the acquired image.
    let initial_surface = skia_surfaces[0].clone();

    let vk_state = VkState {
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
        vk_format,
    };

    Env { surface: initial_surface, gr_context: skia_ctx, window, backend: BackendState::Vk(vk_state) }
}

// ---------------------------------------------------------------------------
// Legacy GL surface helper — kept for any remaining call-sites in browser.rs
// ---------------------------------------------------------------------------

pub(crate) fn create_surface(
    window: &Window,
    fb_info: FramebufferInfo,
    gr_context: &mut DirectContext,
    num_samples: usize,
    stencil_size: usize,
) -> Surface {
    let size = window.inner_size();
    let size = (
        size.width.try_into().expect("Could not convert width"),
        size.height.try_into().expect("Could not convert height"),
    );
    let brt = backend_render_targets::make_gl(size, num_samples, stencil_size, fb_info);
    wrap_backend_render_target(
        gr_context,
        &brt,
        gpu::SurfaceOrigin::BottomLeft,
        ColorType::RGBA8888,
        None,
        None,
    )
    .expect("Failed to wrap backend render target")
}

