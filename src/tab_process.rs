use crate::dom::{AbstractDom, Dom};
use crate::engine::nav_provider::{NavigationProviderMessage, StokesNavigationProvider};
// Tab process module - runs the browser engine in a separate process
use crate::engine::{Engine, EngineConfig, ENGINE_REF, USER_AGENT_REF};
use crate::engine::js_provider::{JsProviderMessage, StokesJsProvider};
use crate::ipc::{connect, IpcChannel, ParentToTabMessage, TabToParentMessage};
use crate::shell_provider::{ShellProviderMessage, StokesShellProvider};
use crate::{js, networking};
use crate::renderer::painter::{ScenePainter, SkiaCache};
use blitz_traits::shell::{ShellProvider, Viewport};
use gl::types::GLint;
use glutin::config::{Config, ConfigSurfaceTypes, ConfigTemplateBuilder, GlConfig};
use glutin::context::{ContextApi, ContextAttributesBuilder, NotCurrentGlContext, PossiblyCurrentContext};
use glutin::display::{Display as GlutinDisplay, DisplayApiPreference, GetGlDisplay, GlDisplay};
use glutin::surface::{PbufferSurface, Surface as GlutinSurface, SurfaceAttributesBuilder};
use raw_window_handle::{RawDisplayHandle, XlibDisplayHandle};
use shared_memory::{Shmem, ShmemConf};
use skia_safe::gpu::gl::{Format, FramebufferInfo, Interface};
use skia_safe::gpu::surfaces::wrap_backend_render_target;
use skia_safe::gpu::{backend_render_targets, DirectContext};
use skia_safe::gpu::{self};
use skia_safe::{ColorType, Surface};
use std::cmp::Ordering as CmpOrdering;
use std::ffi::CString;
use std::io;
use std::num::NonZeroU32;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};
use tracing::trace;
use tracing::metadata::LevelFilter;

/// Tab process that runs in its own OS process
pub struct TabProcess {
    pub(crate) engine: Engine,
    scene_cache: SkiaCache,
    animation_time: Option<Instant>,
    channel: IpcChannel,
    tab_id: String,
    shared_surface: Option<SharedSurface>,
    surface_generation: u32,
    shell_receiver: UnboundedReceiver<ShellProviderMessage>,
    nav_receiver: UnboundedReceiver<NavigationProviderMessage>,
    redraw_request: AtomicBool,
    navigation_id: u64,
}

/// Shared memory surface for efficient rendering data transfer
struct SharedSurface {
    shmem: Shmem,
    shmem_name: String,
    renderer: HeadlessGlRenderer,
    width: u32,
    height: u32,
}

struct HeadlessGlRenderer {
    _gl_surface: GlutinSurface<PbufferSurface>,
    _gl_context: PossiblyCurrentContext,
    gr_context: DirectContext,
    surface: Surface,
    fb_info: FramebufferInfo,
    readback: ReadbackPipeline,
    readback_stats: ReadbackStats,
}

enum ReadbackPipeline {
    Sync,
    Async(AsyncReadback),
}

struct AsyncReadback {
    slots: Vec<ReadbackSlot>,
    bytes_per_frame: usize,
}

struct ReadbackSlot {
    pbo: gl::types::GLuint,
    fence: Option<gl::types::GLsync>,
}

#[derive(Default)]
struct ReadbackStats {
    total_frames: u64,
    async_frames: u64,
    sync_fallback_frames: u64,
}

impl HeadlessGlRenderer {
    fn new(width: u32, height: u32) -> io::Result<Self> {
        let width = width.max(1);
        let height = height.max(1);
        let display = create_headless_display()?;
        let gl_config = pick_gl_config(&display, width, height)?;

        let context_attributes = ContextAttributesBuilder::new().build(None);
        let fallback_context_attributes =
            ContextAttributesBuilder::new().with_context_api(ContextApi::Gles(None)).build(None);

        let not_current_gl_context = unsafe {
            display
                .create_context(&gl_config, &context_attributes)
                .or_else(|_| display.create_context(&gl_config, &fallback_context_attributes))
                .map_err(io_other)?
        };

        let attrs = SurfaceAttributesBuilder::<PbufferSurface>::new().build(
            NonZeroU32::new(width).ok_or_else(|| io::Error::other("Invalid pbuffer width"))?,
            NonZeroU32::new(height).ok_or_else(|| io::Error::other("Invalid pbuffer height"))?,
        );

        let gl_surface = unsafe {
            display
                .create_pbuffer_surface(&gl_config, &attrs)
                .map_err(io_other)?
        };
        let gl_context = not_current_gl_context
            .make_current(&gl_surface)
            .map_err(io_other)?;

        gl::load_with(|symbol| {
            gl_config
                .display()
                .get_proc_address(CString::new(symbol).unwrap().as_c_str())
        });

        let interface = Interface::new_load_with(|name| {
            if name == "eglGetCurrentDisplay" {
                return std::ptr::null();
            }
            gl_config
                .display()
                .get_proc_address(CString::new(name).unwrap().as_c_str())
        })
        .ok_or_else(|| io::Error::other("Could not create GL interface"))?;

        let mut gr_context = gpu::direct_contexts::make_gl(interface, Some(&gpu::ContextOptions::default()))
            .ok_or_else(|| io::Error::other("Failed to create Skia GL context"))?;

        let mut fboid: GLint = 0;
        unsafe { gl::GetIntegerv(gl::FRAMEBUFFER_BINDING, &mut fboid) };
        let fb_info = FramebufferInfo {
            fboid: fboid.try_into().map_err(io_other)?,
            format: Format::RGBA8.into(),
            ..Default::default()
        };

        let surface = create_skia_gl_surface(
            width,
            height,
            fb_info,
            &mut gr_context,
            gl_config.num_samples() as usize,
            gl_config.stencil_size() as usize,
        )?;

        let readback = match AsyncReadback::new((width * height * 4) as usize) {
            Ok(async_readback) => ReadbackPipeline::Async(async_readback),
            Err(e) => {
                eprintln!("[readback] async path unavailable, using sync mode: {e}");
                ReadbackPipeline::Sync
            }
        };

        Ok(Self {
            _gl_surface: gl_surface,
            _gl_context: gl_context,
            gr_context,
            surface,
            fb_info,
            readback,
            readback_stats: ReadbackStats::default(),
        })
    }

    fn readback_into_shmem(&mut self, dst: &mut [u8], width: u32, height: u32) -> io::Result<()> {
        self.readback_stats.total_frames = self.readback_stats.total_frames.saturating_add(1);
        match &mut self.readback {
            ReadbackPipeline::Sync => {
                self.readback_stats.sync_fallback_frames =
                    self.readback_stats.sync_fallback_frames.saturating_add(1);
                sync_readback(self.fb_info.fboid, dst, width, height)?;
            }
            ReadbackPipeline::Async(async_readback) => {
                if async_readback
                    .enqueue_readback(self.fb_info.fboid, width, height)
                    .and_then(|_| async_readback.try_harvest(dst))
                    .unwrap_or(false)
                {
                    self.readback_stats.async_frames = self.readback_stats.async_frames.saturating_add(1);
                } else {
                    // Always deliver a frame even if no async slot is ready yet.
                    self.readback_stats.sync_fallback_frames =
                        self.readback_stats.sync_fallback_frames.saturating_add(1);
                    sync_readback(self.fb_info.fboid, dst, width, height)?;
                }
            }
        }

        // Emit periodic telemetry so we can confirm whether async is actually active.
        if self.readback_stats.total_frames.is_multiple_of(120) {
            trace!(
                "[readback] frames={} async={} sync_fallback={}",
                self.readback_stats.total_frames,
                self.readback_stats.async_frames,
                self.readback_stats.sync_fallback_frames
            );
        }

        Ok(())
    }
}

impl AsyncReadback {
    const SLOT_COUNT: usize = 3;

    fn new(bytes_per_frame: usize) -> io::Result<Self> {
        if bytes_per_frame == 0 {
            return Err(io::Error::other("Invalid readback size"));
        }

        let mut slots = Vec::with_capacity(Self::SLOT_COUNT);
        for _ in 0..Self::SLOT_COUNT {
            let mut pbo: gl::types::GLuint = 0;
            unsafe {
                gl::GenBuffers(1, &mut pbo);
                if pbo == 0 {
                    return Err(io::Error::other("Failed to allocate PBO"));
                }
                gl::BindBuffer(gl::PIXEL_PACK_BUFFER, pbo);
                gl::BufferData(
                    gl::PIXEL_PACK_BUFFER,
                    bytes_per_frame as isize,
                    ptr::null(),
                    gl::STREAM_READ,
                );
                gl::BindBuffer(gl::PIXEL_PACK_BUFFER, 0);
            }

            slots.push(ReadbackSlot { pbo, fence: None });
        }

        Ok(Self {
            slots,
            bytes_per_frame,
        })
    }

    fn enqueue_readback(&mut self, fboid: u32, width: u32, height: u32) -> io::Result<()> {
        let Some(slot) = self.slots.iter_mut().find(|slot| slot.fence.is_none()) else {
            return Ok(());
        };

        unsafe {
            gl::BindFramebuffer(gl::FRAMEBUFFER, fboid);
            gl::PixelStorei(gl::PACK_ALIGNMENT, 1);
            gl::BindBuffer(gl::PIXEL_PACK_BUFFER, slot.pbo);
            gl::ReadPixels(
                0,
                0,
                width as i32,
                height as i32,
                gl::RGBA,
                gl::UNSIGNED_BYTE,
                ptr::null_mut(),
            );

            let fence = gl::FenceSync(gl::SYNC_GPU_COMMANDS_COMPLETE, 0);
            gl::BindBuffer(gl::PIXEL_PACK_BUFFER, 0);

            if fence.is_null() {
                return Err(io::Error::other("Failed to create GL fence"));
            }

            slot.fence = Some(fence);
        }

        Ok(())
    }

    fn try_harvest(&mut self, dst: &mut [u8]) -> io::Result<bool> {
        if dst.len() < self.bytes_per_frame {
            return Err(io::Error::other("Shared-memory destination too small"));
        }

        // Prefer harvesting older slots first to keep queue depth bounded.
        self.slots.sort_by(|a, b| match (a.fence.is_some(), b.fence.is_some()) {
            (true, false) => CmpOrdering::Less,
            (false, true) => CmpOrdering::Greater,
            _ => CmpOrdering::Equal,
        });

        for slot in &mut self.slots {
            let Some(fence) = slot.fence else {
                continue;
            };

            let status = unsafe { gl::ClientWaitSync(fence, 0, 0) };
            if status != gl::ALREADY_SIGNALED && status != gl::CONDITION_SATISFIED {
                continue;
            }

            unsafe {
                gl::BindBuffer(gl::PIXEL_PACK_BUFFER, slot.pbo);
                let mapped = gl::MapBufferRange(
                    gl::PIXEL_PACK_BUFFER,
                    0,
                    self.bytes_per_frame as isize,
                    gl::MAP_READ_BIT,
                );

                if mapped.is_null() {
                    gl::BindBuffer(gl::PIXEL_PACK_BUFFER, 0);
                    gl::DeleteSync(fence);
                    slot.fence = None;
                    return Err(io::Error::other("Failed to map PBO readback buffer"));
                }

                ptr::copy_nonoverlapping(mapped as *const u8, dst.as_mut_ptr(), self.bytes_per_frame);
                gl::UnmapBuffer(gl::PIXEL_PACK_BUFFER);
                gl::BindBuffer(gl::PIXEL_PACK_BUFFER, 0);

                gl::DeleteSync(fence);
                slot.fence = None;
            }

            return Ok(true);
        }

        Ok(false)
    }
}

impl Drop for AsyncReadback {
    fn drop(&mut self) {
        for slot in &mut self.slots {
            unsafe {
                if let Some(fence) = slot.fence.take() {
                    gl::DeleteSync(fence);
                }
                if slot.pbo != 0 {
                    gl::DeleteBuffers(1, &slot.pbo);
                    slot.pbo = 0;
                }
            }
        }
    }
}

fn create_headless_display() -> io::Result<GlutinDisplay> {
    let raw_display = RawDisplayHandle::Xlib(XlibDisplayHandle::new(None, 0));
    unsafe { GlutinDisplay::new(raw_display, DisplayApiPreference::Egl).map_err(io_other) }
}

fn pick_gl_config(display: &GlutinDisplay, width: u32, height: u32) -> io::Result<Config> {
    let pbuffer_width =
        NonZeroU32::new(width.max(1)).ok_or_else(|| io::Error::other("Invalid pbuffer width"))?;
    let pbuffer_height =
        NonZeroU32::new(height.max(1)).ok_or_else(|| io::Error::other("Invalid pbuffer height"))?;

    let template = ConfigTemplateBuilder::new()
        .with_alpha_size(8)
        .with_surface_type(ConfigSurfaceTypes::PBUFFER)
        .with_pbuffer_sizes(pbuffer_width, pbuffer_height)
        .build();

    unsafe {
        display
            .find_configs(template)
            .map_err(io_other)?
            .reduce(|best, config| {
                if config.num_samples() < best.num_samples() {
                    config
                } else {
                    best
                }
            })
            .ok_or_else(|| io::Error::other("No GL config available for headless renderer"))
    }
}

fn create_skia_gl_surface(
    width: u32,
    height: u32,
    fb_info: FramebufferInfo,
    gr_context: &mut DirectContext,
    num_samples: usize,
    stencil_size: usize,
) -> io::Result<Surface> {
    let backend_render_target =
        backend_render_targets::make_gl((width as i32, height as i32), num_samples, stencil_size, fb_info);
    wrap_backend_render_target(
        gr_context,
        &backend_render_target,
        gpu::SurfaceOrigin::BottomLeft,
        ColorType::RGBA8888,
        None,
        None,
    )
    .ok_or_else(|| io::Error::other("Failed to wrap backend render target"))
}

fn io_other<E: std::fmt::Display>(error: E) -> io::Error {
    io::Error::other(error.to_string())
}

fn sync_readback(fboid: u32, dst: &mut [u8], width: u32, height: u32) -> io::Result<()> {
    let required = (width as usize) * (height as usize) * 4;
    if dst.len() < required {
        return Err(io::Error::other("Shared-memory destination too small"));
    }

    unsafe {
        gl::BindFramebuffer(gl::FRAMEBUFFER, fboid);
        gl::PixelStorei(gl::PACK_ALIGNMENT, 1);
        gl::ReadPixels(
            0,
            0,
            width as i32,
            height as i32,
            gl::RGBA,
            gl::UNSIGNED_BYTE,
            dst.as_mut_ptr() as *mut _,
        );
    }

    Ok(())
}

impl TabProcess {
    /// Create a new tab process and connect to the parent
    pub fn new(tab_id: String, server_name: String) -> io::Result<Self> {
        let channel = connect(&server_name)?;

        // Create an unbounded channel for shell provider messages which can be sent from any thread
        let (shell_tx, shell_rx) = unbounded_channel::<ShellProviderMessage>();

        let shell_provider = StokesShellProvider::new(shell_tx);

        let (nav_tx, nav_rx) = unbounded_channel::<NavigationProviderMessage>();
        let navigation_provider = StokesNavigationProvider::new(nav_tx);

        let config = EngineConfig {
            ..Default::default()
        };

        let mut engine = Engine::new(
            config,
            Viewport::default(),
            Arc::new(shell_provider),
            Arc::new(navigation_provider),
        );

        // Set the engine reference in the thread-local storage
        ENGINE_REF.with(|engine_ref| {
            *engine_ref.borrow_mut() = Some(&mut engine as *mut Engine);
        });
        USER_AGENT_REF.with(|agent_ref| {
            *agent_ref.borrow_mut() = Some(engine.config.user_agent.clone());
        });

        Ok(Self {
            engine,
            scene_cache: SkiaCache::default(),
            animation_time: None,
            channel,
            tab_id,
            shared_surface: None,
            surface_generation: 0,
            shell_receiver: shell_rx,
            nav_receiver: nav_rx,
            redraw_request: AtomicBool::new(false),
            navigation_id: 0,
        })
    }

    fn animation_time(&mut self) -> f64 {
        match &self.animation_time {
            Some(start) => Instant::now().duration_since(*start).as_secs_f64(),
            None => {
                self.animation_time = Some(Instant::now());
                0.0
            }
        }
    }

    /// Initialize shared memory surface
    fn init_shared_surface(&mut self, width: u32, height: u32) -> io::Result<()> {
        let width = width.max(1);
        let height = height.max(1);

        // Drop the old shared memory surface first to avoid conflicts
        if let Some(old_surface) = self.shared_surface.take() {
            // Explicitly drop the old shmem to release the OS resource
            drop(old_surface.shmem);
        }

        // Increment generation counter for unique ID
        self.surface_generation = self.surface_generation.wrapping_add(1);

        let shmem_name = format!("stokes_tab_{}_{}_{}", self.tab_id, std::process::id(), self.surface_generation);

        // Calculate required size (RGBA8888 = 4 bytes per pixel)
        let size = (width * height * 4) as usize;

        let shmem = ShmemConf::new()
            .size(size)
            .os_id(&shmem_name)
            .create()
            .map_err(io_other)?;

        let renderer = HeadlessGlRenderer::new(width, height)?;

        self.shared_surface = Some(SharedSurface {
            shmem,
            shmem_name,
            renderer,
            width,
            height,
        });

        Ok(())
    }

    /// Main event loop for the tab process
    pub async fn run(&mut self) -> io::Result<()> {
        // Send ready message
        self.channel.send(&TabToParentMessage::Ready)?;

        loop {
            match self.shell_receiver.try_recv() {
                Ok(msg) => {
                    let _ = self.handle_shell_provider_message(&msg).await;
                    let _ = self.channel.send(&TabToParentMessage::ShellProvider(msg));
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {},
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {},
            }
            match self.nav_receiver.try_recv() {
                Ok(msg) => {
                    match msg {
                        NavigationProviderMessage::NavigateTo(options) => {
                            if self.engine.dom.is_none() {
                                continue;
                            }

                            // Only let the latest async navigation callback commit a document.
                            self.navigation_id = self.navigation_id.wrapping_add(1);
                            let navigation_id = self.navigation_id;

                            let nav_provider = self.engine.navigation_provider.clone();
                            let _ = self.channel.send(&TabToParentMessage::LoadingStateChanged(true));
                            let url = options.url.as_str().to_string();
                            let _ = self.channel.send(&TabToParentMessage::NavigationStarted(url.clone()));
                            self.dom().unwrap().net_provider.fetch_with_callback(
                                options.into_request(),
                                Box::new(move |result| {
                                    let (url, bytes) = match result {
                                        Ok(res) => res,
                                        Err(_) => {
                                            (url, include_str!("../assets/404.html").into())
                                        }
                                    };
                                    let contents = std::str::from_utf8(&bytes).unwrap().to_string();
                                    let _ = nav_provider.sender.send(NavigationProviderMessage::Navigate {
                                        navigation_id,
                                        url,
                                        contents,
                                        is_md: false,
                                        retain_scroll_position: false,
                                    });
                                })
                            );
                        }
                        NavigationProviderMessage::Navigate {
                            navigation_id,
                            url,
                            contents,
                            retain_scroll_position: _,
                            is_md: _,
                        } => {
                            if navigation_id != self.navigation_id {
                                continue;
                            }
                            self.engine.set_loading_state(true);
                            match self.engine.navigate(&url, contents, true, true).await {
                                Ok(_) => {
                                    let title = self.engine.page_title().to_string();
                                    let _ = self.channel.send(&TabToParentMessage::NavigationCompleted {
                                        url: url.clone(),
                                        title: title.clone(),
                                    });
                                    let _ = self.channel.send(&TabToParentMessage::TitleChanged(title));
                                    let _ = self.channel.send(&TabToParentMessage::LoadingStateChanged(false));
                                    self.render_frame()?;
                                }
                                Err(e) => {
                                    let _ = self.channel.send(&TabToParentMessage::NavigationFailed(e.to_string()));
                                    let _ = self.channel.send(&TabToParentMessage::LoadingStateChanged(false));
                                }
                            }
                        }
                        NavigationProviderMessage::NavigateReplace(options) => {
                            if self.engine.dom.is_none() {
                                continue;
                            }

                            self.navigation_id = self.navigation_id.wrapping_add(1);
                            let navigation_id = self.navigation_id;

                            let nav_provider = self.engine.navigation_provider.clone();
                            let _ = self.channel.send(&TabToParentMessage::LoadingStateChanged(true));
                            let url = options.url.as_str().to_string();
                            let _ = self.channel.send(&TabToParentMessage::NavigationStarted(url.clone()));
                            self.dom().unwrap().net_provider.fetch_with_callback(
                                options.into_request(),
                                Box::new(move |result| {
                                    let (url, bytes) = match result {
                                        Ok(res) => res,
                                        Err(_) => {
                                            (url, include_str!("../assets/404.html").into())
                                        }
                                    };
                                    let contents = std::str::from_utf8(&bytes).unwrap().to_string();
                                    let _ = nav_provider.sender.send(NavigationProviderMessage::NavigateReplaceCommit {
                                        navigation_id,
                                        url,
                                        contents,
                                    });
                                })
                            );
                        }
                        NavigationProviderMessage::NavigateReplaceCommit {
                            navigation_id,
                            url,
                            contents,
                        } => {
                            if navigation_id != self.navigation_id {
                                continue;
                            }
                            self.engine.set_loading_state(true);
                            // Navigate without adding to history, then replace the current entry.
                            match self.engine.navigate(&url, contents, true, false).await {
                                Ok(_) => {
                                    self.engine.replace_current_history_entry(url.clone());
                                    let title = self.engine.page_title().to_string();
                                    let _ = self.channel.send(&TabToParentMessage::NavigationCompleted {
                                        url: url.clone(),
                                        title: title.clone(),
                                    });
                                    let _ = self.channel.send(&TabToParentMessage::TitleChanged(title));
                                    let _ = self.channel.send(&TabToParentMessage::LoadingStateChanged(false));
                                    self.render_frame()?;
                                }
                                Err(e) => {
                                    let _ = self.channel.send(&TabToParentMessage::NavigationFailed(e.to_string()));
                                    let _ = self.channel.send(&TabToParentMessage::LoadingStateChanged(false));
                                }
                            }
                        }
                    }
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {},
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {},
            }

            // Process all pending messages from parent (non-blocking)
            let mut has_messages = true;
            let mut should_render_after_messages = false;
            while has_messages {
                let msg_option = self.channel.try_receive()?;
                match msg_option {
                    Some(msg) => {
                        let (should_render, should_continue) = self.handle_message(msg).await?;
                        if !should_continue {
                            println!("Shutting down");
                            return Ok(()); // Shutdown requested
                        }
                        if should_render {
                            should_render_after_messages = true;
                        }
                    }
                    None => {
                        has_messages = false;
                    }
                }
            }
            if self.redraw_request.load(Ordering::Relaxed) {
                should_render_after_messages = true;
                self.redraw_request.store(false, Ordering::Relaxed);
            }

            if should_render_after_messages {
                self.render_frame()?;
            }

            // Small sleep to prevent CPU spinning
            //tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
    }

    fn dom(&self) -> Option<&Dom> {
        self.engine.dom.as_ref()
    }

    fn dom_mut(&mut self) -> Option<&mut Dom> {
        self.engine.dom.as_mut()
    }

    /// Handle a message from the parent process
    async fn handle_message(&mut self, message: ParentToTabMessage) -> io::Result<(bool, bool)> {
        let mut should_render: bool = false;
        match message {
            ParentToTabMessage::Navigate(url) => {
                // Invalidate any in-flight async navigation callback.
                self.navigation_id = self.navigation_id.wrapping_add(1);
                let _ = self.channel.send(&TabToParentMessage::NavigationStarted(url.clone()));
                self.engine.set_loading_state(true);

                let contents = networking::fetch(&url, &self.engine.config.user_agent).unwrap_or_else(|e| {
                    eprintln!("[navigate] networking::fetch failed for {url}: {e}");
                    include_str!("../assets/404.html").to_string()
                });
                match self.engine.navigate(&url, contents, true, true).await {
                    Ok(_) => {
                        let title = self.engine.page_title().to_string();
                        let _ = self.channel.send(&TabToParentMessage::NavigationCompleted {
                            url: url.clone(),
                            title: title.clone(),
                        });
                        let _ = self.channel.send(&TabToParentMessage::TitleChanged(title));
                        let _ = self.channel.send(&TabToParentMessage::LoadingStateChanged(false));
                        should_render = true;
                    }
                    Err(e) => {
                        let _ = self.channel.send(&TabToParentMessage::NavigationFailed(e.to_string()));
                        let _ = self.channel.send(&TabToParentMessage::LoadingStateChanged(false));
                    }
                }
            }
            ParentToTabMessage::Reload => {
                self.navigation_id = self.navigation_id.wrapping_add(1);
                let url = self.engine.current_url().to_string();
                if !url.is_empty() {
                    let _ = self.channel.send(&TabToParentMessage::NavigationStarted(url.clone()));
                    self.engine.set_loading_state(true);
                    let contents = networking::fetch(&url, &self.engine.config.user_agent).unwrap_or_else(|e| {
                        eprintln!("[reload] networking::fetch failed for {url}: {e}");
                        include_str!("../assets/404.html").to_string()
                    });
                    match self.engine.navigate(&url, contents, true, true).await {
                        Ok(_) => {
                            let title = self.engine.page_title().to_string();
                            let _ = self.channel.send(&TabToParentMessage::NavigationCompleted { url, title });
                            let _ = self.channel.send(&TabToParentMessage::LoadingStateChanged(false));
                            should_render = true;
                        }
                        Err(e) => {
                            let _ = self.channel.send(&TabToParentMessage::NavigationFailed(e.to_string()));
                            let _ = self.channel.send(&TabToParentMessage::LoadingStateChanged(false));
                        }
                    }
                }
            }
            ParentToTabMessage::GoBack => {
                self.navigation_id = self.navigation_id.wrapping_add(1);
                if self.engine.can_go_back() {
                    let url = self.engine.current_url().to_string();
                    let _ = self.channel.send(&TabToParentMessage::NavigationStarted(url.clone()));
                    self.engine.set_loading_state(true);
                    match self.engine.go_back().await {
                        Ok(_) => {
                            let title = self.engine.page_title().to_string();
                            let url = self.engine.current_url().to_string();
                            let _ = self.channel.send(&TabToParentMessage::NavigationCompleted { url, title });
                            let _ = self.channel.send(&TabToParentMessage::LoadingStateChanged(false));
                            should_render = true;
                        }
                        Err(e) => {
                            eprintln!("Go back failed: {}", e);
                            let _ = self.channel.send(&TabToParentMessage::LoadingStateChanged(false));
                        }
                    }
                }
            }
            ParentToTabMessage::GoForward => {
                self.navigation_id = self.navigation_id.wrapping_add(1);
                if self.engine.can_go_forward() {
                    let url = self.engine.current_url().to_string();
                    let _ = self.channel.send(&TabToParentMessage::NavigationStarted(url.clone()));
                    self.engine.set_loading_state(true);
                    match self.engine.go_forward().await {
                        Ok(_) => {
                            let title = self.engine.page_title().to_string();
                            let url = self.engine.current_url().to_string();
                            let _ = self.channel.send(&TabToParentMessage::NavigationCompleted { url, title });
                            let _ = self.channel.send(&TabToParentMessage::LoadingStateChanged(false));
                            should_render = true;
                        }
                        Err(e) => {
                            eprintln!("Go forward failed: {}", e);
                            let _ = self.channel.send(&TabToParentMessage::LoadingStateChanged(false));
                        }
                    }
                }
            }
            ParentToTabMessage::Resize { width, height } => {
                self.engine.resize(width, height);
                self.init_shared_surface(width as u32, height as u32)?;
                should_render = true;
            }
            // todo ctrl+click nav new tab, middle click, Home + End keys, keyboard scrolling
            ParentToTabMessage::UI(event) => {
                if let Some(dom) = self.dom_mut() {
                    dom.handle_ui_event(event);
                }
            }
            /*ParentToTabMessage::KeyboardInput { key_type, modifiers } => {
                use crate::ipc::KeyInputType;
                match key_type {
                    KeyInputType::Scroll { direction, amount } => {}
                    KeyInputType::Named(key_name) => {
                        match key_name.as_str() {
                            "Home" => { self.engine.set_scroll_position(0.0, 0.0); }
                            "End" => { self.engine.set_scroll_position(0.0, f32::MAX); }
                            _ => {}
                        }
                    }
                    KeyInputType::Character(text) => {
                        if modifiers.ctrl {
                            match text.as_str() {
                                _ => {}
                            }
                        }
                    }
                }
                should_render = true;
            }*/
            ParentToTabMessage::RequestFrame => {
                should_render = true;
            }
            ParentToTabMessage::SetScaleFactor(scale) => {
                self.engine.set_viewport(Viewport {
                    hidpi_scale: scale,
                    ..self.engine.viewport
                });
            }
            ParentToTabMessage::SetZoom(zoom) => {
                self.engine.set_viewport(Viewport {
                    zoom,
                    ..self.engine.viewport
                });
                should_render = true;
            }
            ParentToTabMessage::Shutdown => {
                return Ok((false, false));
            }
        }
        Ok((should_render, true))
    }

    async fn handle_shell_provider_message(&mut self, message: &ShellProviderMessage) -> io::Result<()> {
        match message {
            ShellProviderMessage::RequestRedraw => {
                self.redraw_request.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            _ => {}
        }
        Ok(())
    }

    /// Render a frame to the shared memory surface
    fn render_frame(&mut self) -> io::Result<()> {
        let animation_time = self.animation_time();
        if let Some(ref mut shared) = self.shared_surface {
            {
                let canvas = shared.renderer.surface.canvas();

                // Clear the canvas to prevent old frames from showing through
                canvas.restore_to_count(1);
                canvas.clear(skia_safe::Color::WHITE);

                let mut painter = ScenePainter {
                    inner: canvas,
                    cache: &mut self.scene_cache,
                };

                let engine = &mut self.engine;
                if engine.dom.is_some() {
                    engine.render(&mut painter, animation_time);

                    let dom = engine.dom.as_ref().unwrap();
                    // todo check if window is visible
                    if dom.animating() {
                        dom.shell_provider.request_redraw();
                    }
                }
            }

            shared.renderer.gr_context.flush_and_submit();

            let dst = unsafe { shared.shmem.as_slice_mut() };

            shared
                .renderer
                .readback_into_shmem(dst, shared.width, shared.height)?;

            self.scene_cache.next_gen();

            // Notify parent that frame is ready
            self.channel.send(&TabToParentMessage::FrameRendered {
                shmem_name: shared.shmem_name.clone(),
                width: shared.width,
                height: shared.height,
            })?;
        }
        Ok(())
    }
}

/// Entry point for tab process executable
pub async fn tab_process_main(tab_id: String, server_name: String) -> io::Result<()> {
    tracing_subscriber::fmt::fmt().with_max_level(LevelFilter::WARN).init();

    let mut process = TabProcess::new(tab_id, server_name)?;
    process.run().await
}