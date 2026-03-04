use crate::dom::{AbstractDom, Dom};
use crate::engine::nav_provider::{NavigationProviderMessage, StokesNavigationProvider};
use crate::engine::{Engine, EngineConfig, ENGINE_REF, USER_AGENT_REF};
use crate::ipc::{connect, IpcChannel, ParentToTabMessage, TabToParentMessage};
use crate::networking;
use crate::renderer::painter::{ScenePainter, SkiaCache};
use crate::shell_provider::{ShellProviderMessage, StokesShellProvider};
use crate::vk_shared::{SkiaGetProc, TabVkImage, VulkanDeviceInfo};
use ash::vk::{self, Handle};
use blitz_traits::shell::{ShellProvider, Viewport};
use skia_safe::gpu::{self as sk_gpu, DirectContext};
use std::ffi::CStr;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use skia_safe::gpu::vk::GetProcOf;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};

// ── Grouped Vulkan state for the tab process ────────────────────────────────

/// All Vulkan handles owned by the tab process, grouped into a single struct.
/// Stored as `Option<TabVulkanState>` on `TabProcess` — if `None`, Vulkan is
/// unavailable and the tab falls back to CPU rendering.
struct TabVulkanState {
    _entry: ash::Entry,
    instance: ash::Instance,
    physical_device: vk::PhysicalDevice,
    device: ash::Device,
    queue_family_index: u32,
    gr_context: DirectContext,
    /// Parent PID, cached at init to avoid re-parsing the env var each frame.
    parent_pid: u32,
    /// Preferred image format (from the parent's swapchain).
    vk_format: vk::Format,
}

impl Drop for TabVulkanState {
    fn drop(&mut self) {
        // DirectContext must be dropped before the device.
        // (Rust drops fields in declaration order, so gr_context is dropped
        //  before device — but let's be explicit.)
        unsafe {
            self.device.device_wait_idle().ok();
        }
    }
}

/// Tab process that runs in its own OS process.
///
/// **Field ordering matters for drop safety.**  `vk_image` must be declared
/// (and therefore dropped) *before* `vk_state`, because the image holds a
/// cloned `ash::Device` and calls Vulkan destroy functions in its `Drop` impl.
/// If `vk_state` were dropped first its `Drop` would call `vkDestroyDevice`,
/// leaving `vk_image` with a dangling device handle.
pub struct TabProcess {
    pub(crate) engine: Engine,
    scene_cache: SkiaCache,
    animation_time: Option<Instant>,
    channel: IpcChannel,
    tab_id: String,
    /// Current Vulkan image + Skia surface used for rendering.
    /// ⚠ Must be declared before `vk_state` so it is dropped first.
    vk_image: Option<TabVkImage>,
    /// Vulkan state — `None` if Vulkan init failed (CPU fallback).
    vk_state: Option<TabVulkanState>,
    shell_receiver: UnboundedReceiver<ShellProviderMessage>,
    nav_receiver: UnboundedReceiver<NavigationProviderMessage>,
    redraw_request: AtomicBool,
}

impl TabProcess {
    /// Create a new tab process and connect to the parent.
    pub fn new(tab_id: String, server_name: String) -> io::Result<Self> {
        let channel = connect(&server_name)?;

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

        ENGINE_REF.with(|engine_ref| {
            *engine_ref.borrow_mut() = Some(&mut engine as *mut Engine);
        });
        USER_AGENT_REF.with(|agent_ref| {
            *agent_ref.borrow_mut() = Some(engine.config.user_agent.clone());
        });

        // Parse VulkanDeviceInfo from the environment variable set by the parent.
        let vk_device_info: Option<VulkanDeviceInfo> = std::env::var("STOKES_VK_DEVICE_INFO")
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok());

        let parent_pid = vk_device_info.as_ref().map(|i| i.parent_pid).unwrap_or(0);
        let vk_format = vk_device_info
            .as_ref()
            .map(|i| vk::Format::from_raw(i.image_format))
            .unwrap_or(vk::Format::R8G8B8A8_UNORM);

        // Initialise our private Vulkan device.
        let vk_state = match unsafe { Self::init_vulkan(vk_device_info.as_ref(), parent_pid, vk_format) } {
            Ok(state) => Some(state),
            Err(e) => {
                eprintln!("[Tab {}] Vulkan init failed (CPU fallback): {}", tab_id, e);
                None
            }
        };

        Ok(Self {
            engine,
            scene_cache: SkiaCache::default(),
            animation_time: None,
            channel,
            tab_id,
            vk_image: None,
            vk_state,
            shell_receiver: shell_rx,
            nav_receiver: nav_rx,
            redraw_request: AtomicBool::new(false),
        })
    }

    /// Initialise a private Vulkan instance + device suitable for offscreen rendering.
    unsafe fn init_vulkan(
        parent_info: Option<&VulkanDeviceInfo>,
        parent_pid: u32,
        vk_format: vk::Format,
    ) -> Result<TabVulkanState, String> {
        let entry = ash::Entry::load()
            .map_err(|e| format!("ash Entry::load: {:?}", e))?;

        let instance_api_version = entry
            .try_enumerate_instance_version()
            .map_err(|e| format!("vkEnumerateInstanceVersion: {:?}", e))?
            .unwrap_or(vk::API_VERSION_1_0);

        let app_info = vk::ApplicationInfo::default()
            .application_name(c"stokes-tab")
            .api_version(instance_api_version);

        let instance_ci = vk::InstanceCreateInfo::default()
            .application_info(&app_info);

        let instance = entry.create_instance(&instance_ci, None)
            .map_err(|e| format!("vkCreateInstance: {:?}", e))?;

        let physical_devices = instance.enumerate_physical_devices()
            .map_err(|e| format!("enumerate_physical_devices: {:?}", e))?;

        if physical_devices.is_empty() {
            return Err("No Vulkan physical devices found".into());
        }

        // Try to match the same GPU the parent is using (by device UUID).
        let physical_device = parent_info
            .and_then(|info| {
                physical_devices.iter().find(|&&d| {
                    crate::vk_shared::physical_device_uuid(&instance, d) == info.device_uuid
                }).copied()
            })
            .unwrap_or(physical_devices[0]);

        let queue_families = instance.get_physical_device_queue_family_properties(physical_device);
        let fallback_qfi = queue_families
            .iter()
            .enumerate()
            .find(|(_, q)| q.queue_flags.contains(vk::QueueFlags::GRAPHICS))
            .map(|(i, _)| i as u32)
            .ok_or("No graphics queue family found")?;

        // Reuse parent's queue family index when we matched the same GPU.
        let matched_parent = parent_info
            .is_some_and(|info| {
                crate::vk_shared::physical_device_uuid(&instance, physical_device) == info.device_uuid
            });
        let queue_family_index = if matched_parent {
            parent_info
                .filter(|info| {
                    queue_families
                        .get(info.queue_family_index as usize)
                        .is_some_and(|q| q.queue_flags.contains(vk::QueueFlags::GRAPHICS))
                })
                .map(|info| info.queue_family_index)
                .unwrap_or(fallback_qfi)
        } else {
            fallback_qfi
        };

        // Validate required device extensions.
        let ext_names = crate::vk_shared::tab_device_extension_names();
        let available_exts = instance.enumerate_device_extension_properties(physical_device)
            .map_err(|e| format!("enumerate_device_extension_properties: {:?}", e))?;
        let missing_exts: Vec<String> = ext_names
            .iter()
            .map(|&name| CStr::from_ptr(name).to_string_lossy().into_owned())
            .filter(|name| {
                !available_exts.iter().any(|ext| {
                    CStr::from_ptr(ext.extension_name.as_ptr())
                        .to_string_lossy()
                        .as_ref()
                        == name
                })
            })
            .collect();
        if !missing_exts.is_empty() {
            let driver_name = CStr::from_ptr(
                instance.get_physical_device_properties(physical_device).device_name.as_ptr(),
            )
            .to_string_lossy()
            .into_owned();
            return Err(format!(
                "Missing required Vulkan tab extensions: {:?}. GPU: {}",
                missing_exts, driver_name
            ));
        }

        let queue_priority = [1.0f32];
        let queue_ci = vk::DeviceQueueCreateInfo::default()
            .queue_family_index(queue_family_index)
            .queue_priorities(&queue_priority);

        let device_ci = vk::DeviceCreateInfo::default()
            .queue_create_infos(std::slice::from_ref(&queue_ci))
            .enabled_extension_names(&ext_names);

        let device = instance.create_device(physical_device, &device_ci, None)
            .map_err(|e| format!("vkCreateDevice: {:?}", e))?;

        let queue = device.get_device_queue(queue_family_index, 0);

        // Negotiate API version to match what window.rs does.
        let device_api_version = instance
            .get_physical_device_properties(physical_device)
            .api_version;
        let negotiated_api_version = instance_api_version.min(device_api_version);

        // Build Skia DirectContext using the shared proc loader.
        let get_proc = SkiaGetProc::new(&entry, &instance);
        let get_proc_fn = |of: GetProcOf| get_proc.resolve(of);

        let mut backend_ctx = skia_safe::gpu::vk::BackendContext::new(
            instance.handle().as_raw() as _,
            physical_device.as_raw() as _,
            device.handle().as_raw() as _,
            (queue.as_raw() as _, queue_family_index as usize),
            &get_proc_fn,
        );
        backend_ctx.set_max_api_version(negotiated_api_version);

        let gr_context = sk_gpu::direct_contexts::make_vulkan(&backend_ctx, None)
            .ok_or("Failed to create Skia Vulkan DirectContext in tab")?;

        Ok(TabVulkanState {
            _entry: entry,
            instance,
            physical_device,
            device,
            queue_family_index,
            gr_context,
            parent_pid,
            vk_format,
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

    /// Ensure the VkImage is allocated at the given dimensions.
    /// Returns `Ok(false)` if Vulkan is not available.
    fn ensure_vk_image(&mut self, width: u32, height: u32) -> io::Result<bool> {
        let needs_create = match &self.vk_image {
            None => true,
            Some(img) => img.width != width || img.height != height,
        };

        if !needs_create {
            return Ok(true);
        }

        // Wait for all GPU work to finish before dropping the old image.
        // On Windows, pending semaphore signals or Skia submissions can cause
        // access violations if the image/memory is destroyed while still in use.
        if let Some(vk) = self.vk_state.as_mut() {
            vk.gr_context.flush_and_submit();
            unsafe { vk.device.device_wait_idle().ok(); }
        }

        // Drop the old image (now safe — GPU is idle).
        self.vk_image = None;

        let vk = match self.vk_state.as_mut() {
            Some(v) => v,
            None => return Ok(false),
        };

        let queue = unsafe { vk.device.get_device_queue(vk.queue_family_index, 0) };

        let img = unsafe {
            TabVkImage::new(
                &vk.instance,
                vk.physical_device,
                &vk.device,
                &mut vk.gr_context,
                width,
                height,
                vk.vk_format,
                vk.queue_family_index,
                queue,
            )
        };

        match img {
            Ok(created) => {
                self.vk_image = Some(created);
                Ok(true)
            }
            Err(e) => {
                eprintln!("[Tab:ensure_vk_image] TabVkImage::new failed: {}", e);
                Err(io::Error::new(io::ErrorKind::Other, e))
            }
        }
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
                                        url,
                                        contents,
                                        is_md: false,
                                        retain_scroll_position: false,
                                    });
                                })
                            );
                        }
                        NavigationProviderMessage::Navigate {
                            url,
                            contents,
                            retain_scroll_position: _,
                            is_md: _,
                        } => {
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
                // (Re)create the VkImage at the new size; non-fatal if Vulkan unavailable
                let _ = self.ensure_vk_image(width as u32, height as u32);
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

    /// Render a frame into the shared Vulkan image and notify the parent.
    fn render_frame(&mut self) -> io::Result<()> {
        let animation_time = self.animation_time();

        let vk_image = match self.vk_image.as_mut() {
            Some(img) => img,
            None => return Ok(()), // Not yet initialised or Vulkan unavailable
        };

        let canvas = vk_image.surface_mut().canvas();
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
            if dom.animating() {
                dom.shell_provider.request_redraw();
            }
        }

        // Flush the Skia GPU commands so the image memory is ready to export
        if let Some(ctx) = self.vk_state.as_mut().map(|s| &mut s.gr_context) {
            ctx.flush_and_submit();
        }

        self.scene_cache.next_gen();

        // Use cached parent_pid from TabVulkanState.
        let parent_pid = self.vk_state.as_ref().map(|s| s.parent_pid).unwrap_or(0);

        // After Skia flush, submit a barrier (COLOR_ATTACHMENT_OPTIMAL → GENERAL)
        // and signal the exportable semaphore for cross-process GPU sync.
        let sem_handle: i64 = {
            let vk_image = self.vk_image.as_mut().unwrap();
            unsafe { vk_image.signal_and_export_semaphore(parent_pid) }
        };

        // If we couldn't get a semaphore, fall back to a CPU wait.
        if sem_handle == -1 || sem_handle == 0 {
            if let Some(vk) = self.vk_state.as_ref() {
                let queue = unsafe { vk.device.get_device_queue(vk.queue_family_index, 0) };
                unsafe { vk.device.queue_wait_idle(queue).ok() };
            }
        }

        let vk_image = self.vk_image.as_ref().unwrap();
        let width = vk_image.width;
        let height = vk_image.height;
        let vk_format = vk_image.format.as_raw();
        let alloc_size = vk_image.alloc_size;

        // Export the backing memory as a cross-process handle.
        let mem_handle = match unsafe { vk_image.export_handle(parent_pid) } {
            Ok(h) => h,
            Err(e) => {
                eprintln!("[Tab {}] export_handle failed: {}", self.tab_id, e);
                return Ok(());
            }
        };

        // Send the FrameRendered metadata message with the handle embedded.
        self.channel.send(&TabToParentMessage::FrameRendered {
            mem_handle,
            width,
            height,
            vk_format,
            alloc_size,
            sem_handle,
        })?;

        Ok(())
    }
}

/// Entry point for tab process executable
pub async fn tab_process_main(tab_id: String, server_name: String) -> io::Result<()> {
    tokio::time::sleep(Duration::from_millis(10000)).await;
    let mut process = TabProcess::new(tab_id, server_name)?;
    process.run().await
}
