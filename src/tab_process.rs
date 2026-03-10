use crate::display_list::{DisplayFont, DisplayListRecorder};
use crate::dom::{AbstractDom, Dom};
use crate::engine::nav_provider::{NavigationProviderMessage, StokesNavigationProvider};
// Tab process module - runs the browser engine in a separate process
use crate::engine::{Engine, EngineConfig, ENGINE_REF, USER_AGENT_REF};
use crate::ipc::{connect, IpcChannel, ParentToTabMessage, TabToParentMessage};
use crate::networking;
use crate::renderer::painter::SkiaCache;
use blitz_traits::shell::{ShellProvider, Viewport};
use std::collections::HashSet;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use crate::shell_provider::{StokesShellProvider, ShellProviderMessage};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};

/// Tab process that runs in its own OS process
pub struct TabProcess {
    pub(crate) engine: Engine,
    scene_cache: SkiaCache,
    animation_time: Option<Instant>,
    channel: IpcChannel,
    tab_id: String,
    shell_receiver: UnboundedReceiver<ShellProviderMessage>,
    nav_receiver: UnboundedReceiver<NavigationProviderMessage>,
    redraw_request: AtomicBool,
    navigation_id: u64,
    frame_id: u64,
    sent_fonts: HashSet<DisplayFont>,
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
            shell_receiver: shell_rx,
            nav_receiver: nav_rx,
            redraw_request: AtomicBool::new(false),
            navigation_id: 0,
            frame_id: 0,
            sent_fonts: HashSet::new(),
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

    /// Main event loop for the tab process
    pub async fn run(&mut self) -> io::Result<()> {
        // Send ready message
        self.channel.send(&TabToParentMessage::Ready)?;

        loop {

            loop {
                match self.shell_receiver.try_recv() {
                    Ok(msg) => {
                        let _ = self.handle_shell_provider_message(&msg).await;
                        let _ = self.channel.send(&TabToParentMessage::ShellProvider(msg));
                    }
                    Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                    Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => break,
                }
            }

            loop {
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
                        }
                    }
                    Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                    Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => break,
                }
            }

            // Process all pending messages from parent (non-blocking)
            let mut should_render_after_messages = false;
            while let Some(msg) = self.channel.try_receive()? {
                let (should_render, should_continue) = self.handle_message(msg).await?;
                if !should_continue {
                    println!("Shutting down");
                    return Ok(());
                }
                if should_render {
                    should_render_after_messages = true;
                }
            }

            if self.redraw_request.swap(false, Ordering::Relaxed) {
                should_render_after_messages = true;
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

                let contents = networking::fetch(&url, &self.engine.config.user_agent).unwrap_or_else(|_| {
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
                    let contents = networking::fetch(&url, &self.engine.config.user_agent).unwrap_or_else(|_| {
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

    /// Render a frame to a recorded scene for the parent Vello renderer
    fn render_frame(&mut self) -> io::Result<()> {
        let animation_time = self.animation_time();
        self.frame_id = self.frame_id.wrapping_add(1);

        let engine = &mut self.engine;
        let (width, height) = engine.viewport.window_size;
        let mut recorder = DisplayListRecorder::new(width, height, self.frame_id);

        if engine.dom.is_some() {
            engine.render(&mut recorder, animation_time);

            let dom = engine.dom.as_ref().unwrap();
            // todo check if window is visible
            if dom.animating() {
                dom.shell_provider.request_redraw();
            }
        }

        let (frame, font_payloads) = recorder.into_frame_parts();
        let fonts = font_payloads
            .into_iter()
            .filter(|font| self.sent_fonts.insert(font.font.clone()))
            .collect();

        self.channel
            .send(&TabToParentMessage::DisplayListRendered { frame, fonts })?;
        Ok(())
    }
}

/// Entry point for tab process executable
pub async fn tab_process_main(tab_id: String, server_name: String) -> io::Result<()> {
    let mut process = TabProcess::new(tab_id, server_name)?;
    process.run().await
}