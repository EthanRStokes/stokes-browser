// Tab process module - runs the browser engine in a separate process
use crate::engine::{Engine, EngineConfig};
use crate::ipc::{IpcChannel, ParentToTabMessage, TabToParentMessage, connect};
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use parking_lot::Mutex;
use skia_safe::{Surface, Canvas, ColorType, AlphaType, ImageInfo};
use shared_memory::{ShmemConf, Shmem};

/// Tab process that runs in its own OS process
pub struct TabProcess {
    engine: Engine,
    channel: IpcChannel,
    tab_id: String,
    shared_surface: Option<SharedSurface>,
}

/// Shared memory surface for efficient rendering data transfer
struct SharedSurface {
    shmem: Shmem,
    surface: Surface,
    width: u32,
    height: u32,
}

impl TabProcess {
    /// Create a new tab process and connect to the parent
    pub fn new(tab_id: String, socket_path: PathBuf, config: EngineConfig) -> io::Result<Self> {
        let channel = connect(&socket_path)?;
        let engine = Engine::new(config, 1.0); // Default scale factor

        Ok(Self {
            engine,
            channel,
            tab_id,
            shared_surface: None,
        })
    }

    /// Initialize shared memory surface
    fn init_shared_surface(&mut self, width: u32, height: u32) -> io::Result<()> {
        let shmem_name = format!("stokes_tab_{}_{}", self.tab_id, std::process::id());

        // Calculate required size (RGBA8888 = 4 bytes per pixel)
        let size = (width * height * 4) as usize;

        let shmem = ShmemConf::new()
            .size(size)
            .os_id(&shmem_name)
            .create()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        // Create Skia surface from shared memory
        let image_info = ImageInfo::new(
            (width as i32, height as i32),
            ColorType::RGBA8888,
            AlphaType::Premul,
            None,
        );

        let row_bytes = width as usize * 4;
        let surface = Surface::new_raster(
            &image_info,
            Some(row_bytes),
            None,
        ).ok_or_else(|| io::Error::new(io::ErrorKind::Other, "Failed to create surface"))?;

        self.shared_surface = Some(SharedSurface {
            shmem,
            surface,
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
            // Process any pending messages from parent
            match self.channel.receive::<ParentToTabMessage>() {
                Ok(msg) => {
                    if !self.handle_message(msg).await? {
                        break; // Shutdown requested
                    }
                }
                Err(e) => {
                    eprintln!("Tab process {} error receiving message: {}", self.tab_id, e);
                    break;
                }
            }

            // Process engine timers
            if self.engine.process_timers() {
                // If timers executed, render a new frame
                self.render_frame()?;
            }
        }

        Ok(())
    }

    /// Handle a message from the parent process
    async fn handle_message(&mut self, message: ParentToTabMessage) -> io::Result<bool> {
        match message {
            ParentToTabMessage::Navigate(url) => {
                self.channel.send(&TabToParentMessage::NavigationStarted(url.clone()))?;
                self.engine.set_loading_state(true);

                match self.engine.navigate(&url).await {
                    Ok(_) => {
                        let title = self.engine.page_title().to_string();
                        self.channel.send(&TabToParentMessage::NavigationCompleted {
                            url: url.clone(),
                            title: title.clone(),
                        })?;
                        self.channel.send(&TabToParentMessage::TitleChanged(title))?;
                        self.channel.send(&TabToParentMessage::LoadingStateChanged(false))?;
                        self.render_frame()?;
                    }
                    Err(e) => {
                        self.channel.send(&TabToParentMessage::NavigationFailed(e.to_string()))?;
                        self.channel.send(&TabToParentMessage::LoadingStateChanged(false))?;
                    }
                }
            }
            ParentToTabMessage::Reload => {
                let url = self.engine.current_url().to_string();
                if !url.is_empty() {
                    self.channel.send(&TabToParentMessage::NavigationStarted(url.clone()))?;
                    self.engine.set_loading_state(true);

                    match self.engine.navigate(&url).await {
                        Ok(_) => {
                            let title = self.engine.page_title().to_string();
                            self.channel.send(&TabToParentMessage::NavigationCompleted {
                                url: url.clone(),
                                title,
                            })?;
                            self.channel.send(&TabToParentMessage::LoadingStateChanged(false))?;
                            self.render_frame()?;
                        }
                        Err(e) => {
                            self.channel.send(&TabToParentMessage::NavigationFailed(e.to_string()))?;
                            self.channel.send(&TabToParentMessage::LoadingStateChanged(false))?;
                        }
                    }
                }
            }
            ParentToTabMessage::Resize { width, height } => {
                self.engine.resize(width, height);
                self.init_shared_surface(width as u32, height as u32)?;
                self.render_frame()?;
            }
            ParentToTabMessage::Scroll { delta_x, delta_y } => {
                self.engine.scroll(delta_x, delta_y);
                self.render_frame()?;
            }
            ParentToTabMessage::Click { x, y } => {
                self.engine.handle_click(x, y);
                self.render_frame()?;
            }
            ParentToTabMessage::MouseMove { x, y } => {
                // Update cursor if hovering over interactive elements
                // TODO: Implement cursor detection and send CursorChanged message
            }
            ParentToTabMessage::KeyboardInput { key, modifiers } => {
                // TODO: Handle keyboard input in engine
                self.render_frame()?;
            }
            ParentToTabMessage::RequestFrame => {
                self.render_frame()?;
            }
            ParentToTabMessage::SetScaleFactor(scale) => {
                self.engine.scale_factor = scale;
                self.engine.recalculate_layout();
                self.render_frame()?;
            }
            ParentToTabMessage::Shutdown => {
                return Ok(false); // Signal to exit the loop
            }
        }
        Ok(true) // Continue running
    }

    /// Render a frame to the shared memory surface
    fn render_frame(&mut self) -> io::Result<()> {
        if let Some(ref mut shared) = self.shared_surface {
            let canvas = shared.surface.canvas();

            // Render the engine content
            self.engine.render(canvas, self.engine.scale_factor);

            // Notify parent that frame is ready
            self.channel.send(&TabToParentMessage::FrameRendered {
                shmem_name: format!("stokes_tab_{}_{}", self.tab_id, std::process::id()),
                width: shared.width,
                height: shared.height,
            })?;
        }
        Ok(())
    }
}

/// Entry point for tab process executable
pub async fn tab_process_main(tab_id: String, socket_path: PathBuf) -> io::Result<()> {
    let config = EngineConfig::default();
    let mut process = TabProcess::new(tab_id, socket_path, config)?;
    process.run().await
}
