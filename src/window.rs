use crate::ipc::WgpuRendererInfo;
use futures::executor::block_on;
use vello::{AaConfig, AaSupport, Renderer as VelloRenderer, RendererOptions, Scene};
use wgpu::{CompositeAlphaMode, DeviceDescriptor, ExperimentalFeatures, Features, Instance, InstanceDescriptor, Limits, MemoryHints, PresentMode, SurfaceConfiguration, TextureFormat, TextureUsages};
use winit::dpi::LogicalSize;
use winit::window::{Window, WindowAttributes};
use winit_core::event_loop::ActiveEventLoop;
use winit_core::icon::{Icon, RgbaIcon};

pub(crate) struct Env {
    pub(crate) instance: Instance,
    pub(crate) surface: wgpu::Surface<'static>,
    pub(crate) adapter: wgpu::Adapter,
    pub(crate) device: wgpu::Device,
    pub(crate) queue: wgpu::Queue,
    pub(crate) surface_config: SurfaceConfiguration,
    pub(crate) renderer: VelloRenderer,
    pub(crate) scene: Scene,
    pub(crate) window: Box<dyn Window>,
}

impl Env {
    pub(crate) fn renderer_info(&self) -> WgpuRendererInfo {
        let info = self.adapter.get_info();
        WgpuRendererInfo {
            backend: format!("{:?}", info.backend),
            device_type: format!("{:?}", info.device_type),
            surface_format: format!("{:?}", self.surface_config.format),
        }
    }

    pub(crate) fn render_params(&self) -> vello::RenderParams {
        vello::RenderParams {
            base_color: peniko::Color::WHITE,
            width: self.surface_config.width,
            height: self.surface_config.height,
            antialiasing_method: AaConfig::Msaa16,
        }
    }
}

pub(crate) fn create_window(el: &dyn ActiveEventLoop) -> Env {
    let icon_data = include_bytes!("../assets/com.ethanstokes.stokes-browser.png");
    let icon = image::load_from_memory(icon_data)
        .expect("Failed to load icon")
        .into_rgba8();
    let (icon_width, icon_height) = icon.dimensions();
    let icon: Icon = RgbaIcon::new(icon.into_raw(), icon_width, icon_height)
        .expect("Failed to create icon")
        .into();

    let window_attrs = WindowAttributes::default()
        .with_title("Web Browser")
        .with_surface_size(LogicalSize::new(1024, 768))
        .with_min_surface_size(LogicalSize::new(500, crate::ui::BrowserUI::CHROME_HEIGHT as i32))
        .with_window_icon(Some(icon));

    let window = el.create_window(window_attrs).expect("Failed to create window");

    let instance = Instance::new(&InstanceDescriptor::default());
    let surface = unsafe {
        instance.create_surface_unsafe(
            wgpu::SurfaceTargetUnsafe::from_window(&window).expect("Failed to get raw window handles"),
        )
    }
    .expect("Failed to create wgpu surface");

    let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: Some(&surface),
        force_fallback_adapter: false,
    }))
    .expect("Failed to request wgpu adapter");

    let (device, queue) = block_on(adapter.request_device(&DeviceDescriptor {
        label: Some("stokes-browser-device"),
        required_features: Features::empty(),
        experimental_features: ExperimentalFeatures::disabled(),
        required_limits: Limits::default(),
        memory_hints: MemoryHints::Performance,
        trace: wgpu::Trace::default(),
    }))
    .expect("Failed to create wgpu device");

    let size = window.surface_size();
    let capabilities = surface.get_capabilities(&adapter);
    let format = capabilities
        .formats
        .iter()
        .copied()
        .find(|format| matches!(format, TextureFormat::Bgra8Unorm | TextureFormat::Rgba8Unorm))
        .unwrap_or(capabilities.formats[0]);
    let alpha_mode = capabilities
        .alpha_modes
        .iter()
        .copied()
        .find(|mode| *mode == CompositeAlphaMode::Auto)
        .unwrap_or(capabilities.alpha_modes[0]);

    let surface_config = SurfaceConfiguration {
        usage: TextureUsages::RENDER_ATTACHMENT,
        format,
        width: size.width.max(1),
        height: size.height.max(1),
        present_mode: PresentMode::AutoVsync,
        desired_maximum_frame_latency: 2,
        alpha_mode,
        view_formats: vec![],
    };
    surface.configure(&device, &surface_config);

    let renderer = VelloRenderer::new(
        &device,
        RendererOptions {
            antialiasing_support: AaSupport::all(),
            use_cpu: false,
            num_init_threads: None,
            pipeline_cache: None,
        },
    )
    .expect("Failed to create Vello renderer");

    Env {
        instance,
        surface,
        adapter,
        device,
        queue,
        surface_config,
        renderer,
        scene: Scene::new(),
        window,
    }
}

pub(crate) fn resize_surface(env: &mut Env, width: u32, height: u32) {
    env.surface_config.width = width.max(1);
    env.surface_config.height = height.max(1);
    env.surface.configure(&env.device, &env.surface_config);
}