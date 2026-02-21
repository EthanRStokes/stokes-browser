use gl::types::GLint;
use glutin::config::{ConfigTemplateBuilder, GlConfig};
use glutin::context::{ContextApi, ContextAttributesBuilder, NotCurrentGlContext, PossiblyCurrentContext};
use glutin::display::{GetGlDisplay, GlDisplay};
use glutin::surface::{Surface as GlutinSurface, SurfaceAttributesBuilder, WindowSurface};
use glutin_winit::{ApiPreference, DisplayBuilder};
use skia_safe::gpu::gl::{Format, FramebufferInfo, Interface};
use skia_safe::gpu::surfaces::wrap_backend_render_target;
use skia_safe::gpu::{backend_render_targets, DirectContext};
use skia_safe::{gpu, ColorType, Surface};
use std::ffi::CString;
use std::num::NonZeroU32;
use winit::dpi::LogicalSize;
use winit::event_loop::EventLoop;
use winit::raw_window_handle::HasWindowHandle;
use winit::window::{Window, WindowAttributes};

pub(crate) struct Env {
    pub(crate) surface: Surface,
    pub(crate) gl_surface: GlutinSurface<WindowSurface>,
    pub(crate) gr_context: DirectContext,
    pub(crate) gl_context: PossiblyCurrentContext,
    pub(crate) window: Window,
    pub(crate) fb_info: FramebufferInfo,
    pub(crate) num_samples: usize,
    pub(crate) stencil_size: usize,
}

pub(crate) fn create_window(el: &EventLoop<()>) -> Env {
    // Load and set window icon
    let icon_data = include_bytes!("../assets/com.ethanstokes.stokes-browser.png");
    let icon = image::load_from_memory(icon_data)
        .expect("Failed to load icon")
        .into_rgba8();
    let (icon_width, icon_height) = icon.dimensions();
    let icon = winit::window::Icon::from_rgba(icon.into_raw(), icon_width, icon_height)
        .expect("Failed to create icon");

    // Create window
    let window_attrs = WindowAttributes::default()
        .with_title("Web Browser")
        .with_inner_size(LogicalSize::new(1024, 768))
        .with_min_inner_size(LogicalSize::new(500, crate::ui::BrowserUI::CHROME_HEIGHT as i32))
        .with_window_icon(Some(icon));

    let template = ConfigTemplateBuilder::new()
        .with_alpha_size(8)
        .with_transparency(true);

    let display_builder = DisplayBuilder::new().with_preference(ApiPreference::PreferEgl).with_window_attributes(window_attrs.into());
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

    let window = window.expect("Could not create window with OpenGL context.");
    let window_handle = window.window_handle().expect("Failed to retrieve RawWindowHandle");
    let raw_window_handle = window_handle.as_raw();

    // Create GL context
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
        NonZeroU32::new(height).unwrap()
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
    }).expect("Could not create interface");

    let context_options = gpu::ContextOptions::default();
    let mut gr_context = gpu::direct_contexts::make_gl(interface, Some(&context_options))
        .expect("Failed to create Skia GL context");

    let size = window.inner_size();

    let fb_info = {
        let mut fboid: GLint = 0;
        unsafe { gl::GetIntegerv(gl::FRAMEBUFFER_BINDING, &mut fboid) };
        FramebufferInfo {
            fboid: fboid.try_into().unwrap(),
            format: Format::RGBA8.into(),
            ..Default::default()
        }
    };

    let num_samples = gl_config.num_samples() as usize;
    let stencil_size = gl_config.stencil_size() as usize;

    let surface = create_surface(&window, fb_info, &mut gr_context, num_samples, stencil_size);

    Env {
        surface,
        gl_surface,
        gr_context: gr_context.clone(),
        gl_context,
        window,
        fb_info,
        num_samples,
        stencil_size,
    }
}

pub(crate) fn create_surface(
    window: &Window,
    fb_info: FramebufferInfo,
    gr_context: &mut DirectContext,
    num_samples: usize,
    stencil_size: usize
) -> Surface {
    let size = window.inner_size();
    let size = (
        size.width.try_into().expect("Could not convert width"),
        size.height.try_into().expect("Could not convert height")
    );
    let backend_render_target = backend_render_targets::make_gl(size, num_samples, stencil_size, fb_info);
    wrap_backend_render_target(
        gr_context,
        &backend_render_target,
        gpu::SurfaceOrigin::BottomLeft,
        ColorType::RGBA8888,
        None,
        None
    ).expect("Failed to wrap backend render target")
}