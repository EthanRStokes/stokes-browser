// Tab process module - runs the browser engine in a separate process
use crate::engine::{Engine, EngineConfig};
use crate::ipc::{connect, IpcChannel, ParentToTabMessage, TabToParentMessage};
use crate::js;
use shared_memory::{Shmem, ShmemConf};
use skia_safe::{AlphaType, ColorType, ImageInfo, Surface};
use std::cell::RefCell;
use std::io;
use std::path::PathBuf;
use std::rc::Rc;
use blitz_traits::shell::Viewport;

/// Tab process that runs in its own OS process
pub struct TabProcess {
    engine: Engine,
    channel: Rc<RefCell<IpcChannel>>,
    tab_id: String,
    shared_surface: Option<SharedSurface>,
    surface_generation: u32,
}

/// Shared memory surface for efficient rendering data transfer
struct SharedSurface {
    shmem: Shmem,
    surface: Surface,
    width: u32,
    height: u32,
    generation: u32,
}

impl TabProcess {
    /// Create a new tab process and connect to the parent
    pub fn new(tab_id: String, socket_path: PathBuf, config: EngineConfig) -> io::Result<Self> {
        let channel = connect(&socket_path)?;
        let engine = Engine::new(config, Viewport::default()); // Default viewport, will be resized later

        Ok(Self {
            engine,
            channel: Rc::new(RefCell::new(channel)),
            tab_id,
            shared_surface: None,
            surface_generation: 0,
        })
    }

    /// Initialize shared memory surface
    fn init_shared_surface(&mut self, width: u32, height: u32) -> io::Result<()> {
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
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        // Create a reusable raster surface
        let image_info = ImageInfo::new(
            (width as i32, height as i32),
            ColorType::RGBA8888,
            AlphaType::Premul,
            None,
        );

        let surface = skia_safe::surfaces::raster(&image_info, None, None)
            .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "Failed to create surface"))?;

        self.shared_surface = Some(SharedSurface {
            shmem,
            surface,
            width,
            height,
            generation: self.surface_generation,
        });

        Ok(())
    }

    /// Main event loop for the tab process
    pub async fn run(&mut self) -> io::Result<()> {
        // Set up alert callback to send alerts via IPC
        let channel_for_alert = self.channel.clone();
        js::set_alert_callback(move |message: String| {
            if let Ok(mut channel) = channel_for_alert.try_borrow_mut() {
                let _ = channel.send(&TabToParentMessage::Alert(message));
            }
        });

        // Send ready message
        self.channel.borrow_mut().send(&TabToParentMessage::Ready)?;

        loop {
            // Process all pending messages from parent (non-blocking)
            let mut has_messages = true;
            while has_messages {
                // Get the message first without holding the borrow
                let msg_option = self.channel.borrow_mut().try_receive::<ParentToTabMessage>()?;
                match msg_option {
                    Some(msg) => {
                        if !self.handle_message(msg).await? {
                            return Ok(()); // Shutdown requested
                        }
                    }
                    None => {
                        has_messages = false;
                    }
                }
            }

            // Process engine timers
            if self.engine.process_timers() {
                // If timers executed, render a new frame
                self.render_frame()?;
            }

            // Small sleep to prevent CPU spinning
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
    }

    /// Handle a message from the parent process
    async fn handle_message(&mut self, message: ParentToTabMessage) -> io::Result<bool> {
        match message {
            ParentToTabMessage::Navigate(url) => {
                self.channel.borrow_mut().send(&TabToParentMessage::NavigationStarted(url.clone()))?;
                self.engine.set_loading_state(true);

                match self.engine.navigate(&url).await {
                    Ok(_) => {
                        let title = self.engine.page_title().to_string();
                        let mut channel = self.channel.borrow_mut();
                        channel.send(&TabToParentMessage::NavigationCompleted {
                            url: url.clone(),
                            title: title.clone(),
                        })?;
                        channel.send(&TabToParentMessage::TitleChanged(title))?;
                        channel.send(&TabToParentMessage::LoadingStateChanged(false))?;

                        drop(channel);
                        self.render_frame()?;
                    }
                    Err(e) => {
                        self.channel.borrow_mut().send(&TabToParentMessage::NavigationFailed(e.to_string()))?;
                        self.channel.borrow_mut().send(&TabToParentMessage::LoadingStateChanged(false))?;
                    }
                }
            }
            ParentToTabMessage::Reload => {
                let url = self.engine.current_url().to_string();
                if !url.is_empty() {
                    self.channel.borrow_mut().send(&TabToParentMessage::NavigationStarted(url.clone()))?;
                    self.engine.set_loading_state(true);

                    match self.engine.navigate(&url).await {
                        Ok(_) => {
                            let title = self.engine.page_title().to_string();
                            let mut channel = self.channel.borrow_mut();
                            channel.send(&TabToParentMessage::NavigationCompleted {
                                url: url.clone(),
                                title,
                            })?;
                            channel.send(&TabToParentMessage::LoadingStateChanged(false))?;

                            drop(channel); // Release borrow before rendering
                            self.render_frame()?;
                        }
                        Err(e) => {
                            self.channel.borrow_mut().send(&TabToParentMessage::NavigationFailed(e.to_string()))?;
                            self.channel.borrow_mut().send(&TabToParentMessage::LoadingStateChanged(false))?;
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
                // Handle click and check if a link was clicked
                if let Some(href) = self.engine.handle_click(x, y) {
                    println!("[Tab Process] Link clicked: {}", href);
                    
                    // Resolve the href against the current page URL
                    match self.engine.resolve_url(&href) {
                        Ok(resolved_url) => {
                            println!("[Tab Process] Resolved to: {}", resolved_url);
                            // Send navigation request to parent
                            self.channel.borrow_mut().send(&TabToParentMessage::NavigateRequest(resolved_url))?;
                        }
                        Err(e) => {
                            eprintln!("[Tab Process] Failed to resolve URL '{}': {}", href, e);
                            // Try with the raw href as fallback
                            self.channel.borrow_mut().send(&TabToParentMessage::NavigateRequest(href))?;
                        }
                    }
                }
                self.render_frame()?;
            }
            ParentToTabMessage::MouseMove { x: _, y: _ } => {
                // Update cursor if hovering over interactive elements
                // TODO: Implement cursor detection and send CursorChanged message
            }
            ParentToTabMessage::KeyboardInput { key_type, modifiers } => {
                // Handle keyboard input in the engine
                use crate::ipc::{KeyInputType, ScrollDirection};

                match key_type {
                    KeyInputType::Scroll { direction, amount } => {
                        // Handle keyboard scrolling
                        match direction {
                            ScrollDirection::Up => {
                                self.engine.scroll_vertical(-amount);
                            }
                            ScrollDirection::Down => {
                                self.engine.scroll_vertical(amount);
                            }
                            ScrollDirection::Left => {
                                self.engine.scroll_horizontal(-amount);
                            }
                            ScrollDirection::Right => {
                                self.engine.scroll_horizontal(amount);
                            }
                        }
                    }
                    KeyInputType::Named(key_name) => {
                        // Handle named keys
                        match key_name.as_str() {
                            "Home" => {
                                self.engine.set_scroll_position(0.0, 0.0);
                            }
                            "End" => {
                                self.engine.set_scroll_position(0.0, f32::MAX);
                            }
                            "Enter" | "Escape" | "Tab" | "ShiftTab" | "Backspace" | "Delete" => {
                                // These keys might be handled by JavaScript or form elements
                                // For now, we just trigger a re-render
                                // TODO: Forward to focused element in DOM
                            }
                            _ => {}
                        }
                    }
                    KeyInputType::Character(text) => {
                        // Handle character input
                        // This could be for text input fields, keyboard shortcuts, etc.
                        // TODO: Forward to focused element in DOM

                        // Check for special keyboard shortcuts
                        if modifiers.ctrl {
                            match text.as_str() {
                                "ctrl+a" => {
                                    // Select all in page
                                    // TODO: Implement text selection
                                }
                                "ctrl+c" => {
                                    // Copy selected text
                                    // TODO: Implement copy from page content
                                }
                                "ctrl+f" => {
                                    // Find in page
                                    // TODO: Implement find functionality
                                }
                                _ => {}
                            }
                        }
                    }
                }

                self.render_frame()?;
            }
            ParentToTabMessage::RequestFrame => {
                self.render_frame()?;
            }
            ParentToTabMessage::SetScaleFactor(scale) => {
                self.engine.viewport.hidpi_scale = scale;
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

            // Clear the canvas to prevent old frames from showing through
            canvas.clear(skia_safe::Color::WHITE);

            let engine = &mut self.engine;
            if engine.dom.is_some() {
            let dom = engine.dom();
            engine.render(canvas, &dom, engine.viewport.hidpi_scale);
                }

            // Copy the pixel data to shared memory
            if let Some(pixmap) = shared.surface.peek_pixels() {
                if let Some(src) = pixmap.bytes() {
                    let dst = unsafe { shared.shmem.as_slice_mut() };

                    // Copy all pixel data at once
                    dst.copy_from_slice(src);
                } else {
                    return Err(io::Error::new(io::ErrorKind::Other, "Failed to get pixel bytes"));
                }
            } else {
                return Err(io::Error::new(io::ErrorKind::Other, "Failed to peek pixels"));
            }

            // Notify parent that frame is ready
            self.channel.borrow_mut().send(&TabToParentMessage::FrameRendered {
                shmem_name: format!("stokes_tab_{}_{}_{}", self.tab_id, std::process::id(), shared.generation),
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
