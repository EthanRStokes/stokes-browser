use crate::dom::{AbstractDom, Dom};
use crate::engine::nav_provider::{NavigationProviderMessage, StokesNavigationProvider};
// Tab process module - runs the browser engine in a separate process
use crate::engine::{Engine, EngineConfig, ENGINE_REF, USER_AGENT_REF};
use crate::ipc::{connect, IpcChannel, ParentToTabMessage, TabToParentMessage};
use crate::networking;
use crate::renderer::painter::{ScenePainter, SkiaCache};
use crate::shell_provider::{ShellProviderMessage, StokesShellProvider};
use crate::vk_shared::{TabVkImage, VulkanDeviceInfo};
use ash::vk::{self, Handle};
use blitz_traits::shell::{ShellProvider, Viewport};
use skia_safe::gpu::{self as sk_gpu, DirectContext};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};
use crate::engine::js_provider::{JsProviderMessage, StokesJsProvider};

/// Tab process that runs in its own OS process
pub struct TabProcess {
    pub(crate) engine: Engine,
    scene_cache: SkiaCache,
    animation_time: Option<Instant>,
    channel: IpcChannel,
    tab_id: String,
    /// Skia DirectContext backed by our own Vulkan device
    gr_context: Option<DirectContext>,
    /// Current Vulkan image + Skia surface used for rendering
    vk_image: Option<TabVkImage>,
    /// ash Entry – must outlive Instance/Device to keep libvulkan loaded
    ash_entry: Option<ash::Entry>,
    /// ash handles for our private Vulkan device
    ash_instance: Option<ash::Instance>,
    ash_physical_device: Option<vk::PhysicalDevice>,
    ash_device: Option<ash::Device>,
    queue_family_index: u32,
    /// Preferred image format taken from the parent's swapchain
    vk_format: vk::Format,
    shell_receiver: UnboundedReceiver<ShellProviderMessage>,
    nav_receiver: UnboundedReceiver<NavigationProviderMessage>,
    redraw_request: AtomicBool,
    navigation_id: u64,
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

        // Parse VulkanDeviceInfo from the environment variable set by the parent
        let vk_device_info: Option<VulkanDeviceInfo> = std::env::var("STOKES_VK_DEVICE_INFO")
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok());

        // Determine preferred format (fall back to R8G8B8A8_UNORM)
        let vk_format = vk_device_info.as_ref()
            .map(|i| vk::Format::from_raw(i.image_format))
            .unwrap_or(vk::Format::R8G8B8A8_UNORM);

        // Initialise our private Vulkan device so we can create exportable VkImages.
        let (ash_entry, ash_instance, ash_physical_device, ash_device, queue_family_index, gr_context) =
            match unsafe { Self::init_vulkan(vk_device_info.as_ref()) } {
                Ok(handles) => {
                    let (entry, inst, phys, dev, qfi, ctx) = handles;
                    (Some(entry), Some(inst), Some(phys), Some(dev), qfi, Some(ctx))
                }
                Err(e) => {
                    eprintln!("[Tab {}] Vulkan init failed (will use CPU fallback): {}", tab_id, e);
                    (None, None, None, None, 0, None)
                }
            };

        Ok(Self {
            engine,
            scene_cache: SkiaCache::default(),
            animation_time: None,
            channel,
            tab_id,
            gr_context,
            vk_image: None,
            ash_entry,
            ash_instance,
            ash_physical_device,
            ash_device,
            queue_family_index,
            vk_format,
            shell_receiver: shell_rx,
            nav_receiver: nav_rx,
            redraw_request: AtomicBool::new(false),
            navigation_id: 0,
        })
    }

    /// Initialise a private Vulkan instance + device suitable for offscreen rendering.
    /// Tries to select the same physical device the parent is using (by index/handles from
    /// `VulkanDeviceInfo`), falling back to the first GPU if not found.
    unsafe fn init_vulkan(
        parent_info: Option<&VulkanDeviceInfo>,
    ) -> Result<(ash::Entry, ash::Instance, vk::PhysicalDevice, ash::Device, u32, DirectContext), String> {
        let entry = ash::Entry::load().map_err(|e| format!("ash Entry::load: {:?}", e))?;

        let app_info = vk::ApplicationInfo::default()
            .application_name(c"stokes-tab")
            .api_version(vk::API_VERSION_1_1);

        let instance_ci = vk::InstanceCreateInfo::default()
            .application_info(&app_info);

        let instance = entry.create_instance(&instance_ci, None)
            .map_err(|e| format!("vkCreateInstance: {:?}", e))?;

        let physical_devices = instance.enumerate_physical_devices()
            .map_err(|e| format!("enumerate_physical_devices: {:?}", e))?;

        if physical_devices.is_empty() {
            return Err("No Vulkan physical devices found".into());
        }

        // Try to match the parent's physical device by UUID; fall back to first device.
        let physical_device = parent_info
            .and_then(|info| {
                physical_devices.iter().find(|&&d| {
                    let uuid = crate::vk_shared::physical_device_uuid(&instance, d);
                    uuid == info.device_uuid
                }).copied()
            })
            .unwrap_or(physical_devices[0]);

        // Use the parent's queue family index if provided, otherwise find a graphics family.
        let queue_family_index = parent_info
            .map(|info| info.queue_family_index)
            .unwrap_or_else(|| {
                let queue_families = instance.get_physical_device_queue_family_properties(physical_device);
                queue_families.iter().enumerate()
                    .find(|(_, q)| q.queue_flags.contains(vk::QueueFlags::GRAPHICS))
                    .map(|(i, _)| i as u32)
                    .unwrap_or(0)
            });

        let queue_priority = [1.0f32];
        let queue_ci = vk::DeviceQueueCreateInfo::default()
            .queue_family_index(queue_family_index)
            .queue_priorities(&queue_priority);

        // Enable the correct external memory extension for the platform.
        #[cfg(windows)]
        let ext_names: Vec<*const std::ffi::c_char> = vec![
            ash::khr::external_memory::NAME.as_ptr(),
            ash::khr::external_memory_win32::NAME.as_ptr(),
            ash::khr::external_semaphore::NAME.as_ptr(),
            ash::khr::external_semaphore_win32::NAME.as_ptr(),
        ];
        #[cfg(not(windows))]
        let ext_names: Vec<*const std::ffi::c_char> = vec![
            ash::khr::external_memory::NAME.as_ptr(),
            ash::khr::external_memory_fd::NAME.as_ptr(),
            ash::vk::EXT_EXTERNAL_MEMORY_DMA_BUF_NAME.as_ptr(),
            ash::khr::external_semaphore::NAME.as_ptr(),
            ash::khr::external_semaphore_fd::NAME.as_ptr(),
        ];

        let device_ci = vk::DeviceCreateInfo::default()
            .queue_create_infos(std::slice::from_ref(&queue_ci))
            .enabled_extension_names(&ext_names);

        let device = instance.create_device(physical_device, &device_ci, None)
            .map_err(|e| format!("vkCreateDevice: {:?}", e))?;

        // Build a Skia DirectContext against this device
        let queue = device.get_device_queue(queue_family_index, 0);
        let get_device_proc_addr = instance.fp_v1_0().get_device_proc_addr;
        let get_proc = |gpo: skia_safe::gpu::vk::GetProcOf| {
            match gpo {
                skia_safe::gpu::vk::GetProcOf::Instance(raw_instance, name) => {
                    let vk_instance = vk::Instance::from_raw(raw_instance as _);
                    entry.get_instance_proc_addr(vk_instance, name)
                }
                skia_safe::gpu::vk::GetProcOf::Device(raw_device, name) => {
                    let vk_device = vk::Device::from_raw(raw_device as _);
                    get_device_proc_addr(vk_device, name)
                }
            }
            .map(|f| f as _)
            .unwrap_or(std::ptr::null())
        };

        let gr_context = sk_gpu::direct_contexts::make_vulkan(
            &skia_safe::gpu::vk::BackendContext::new(
                instance.handle().as_raw() as _,
                physical_device.as_raw() as _,
                device.handle().as_raw() as _,
                (queue.as_raw() as _, queue_family_index as usize),
                &get_proc,
            ),
            None,
        ).ok_or_else(|| "Failed to create Skia Vulkan DirectContext in tab".to_string())?;

        Ok((entry, instance, physical_device, device, queue_family_index, gr_context))
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
    /// Returns `Ok(false)` if Vulkan is not available (tab continues without GPU rendering).
    fn ensure_vk_image(&mut self, width: u32, height: u32) -> io::Result<bool> {
        // Check if we need to (re)create the image
        let needs_create = match &self.vk_image {
            None => true,
            Some(img) => img.width != width || img.height != height,
        };

        if !needs_create {
            return Ok(true);
        }

        // Drop the old image first
        self.vk_image = None;

        let (inst, phys, dev, ctx) = match (
            self.ash_instance.as_ref(),
            self.ash_physical_device.as_ref(),
            self.ash_device.as_ref(),
            self.gr_context.as_mut(),
        ) {
            (Some(i), Some(p), Some(d), Some(c)) => (i, p, d, c),
            _ => {
                eprintln!("[Tab {}] Vulkan not available — skipping VkImage creation", self.tab_id);
                return Ok(false);
            }
        };

        let queue = unsafe { dev.get_device_queue(self.queue_family_index, 0) };

        let img = unsafe {
            TabVkImage::new(
                inst,
                *phys,
                dev,
                ctx,
                width,
                height,
                self.vk_format,
                self.queue_family_index,
                queue,
            )
        }.map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        self.vk_image = Some(img);
        Ok(true)
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
        if let Some(ctx) = self.gr_context.as_mut() {
            ctx.flush_and_submit();
        }

        self.scene_cache.next_gen();

        // After Skia flush, submit a barrier (COLOR_ATTACHMENT_OPTIMAL → GENERAL)
        // and signal the exportable semaphore.  The parent will import that
        // semaphore and wait on it before reading the image, giving us a
        // true GPU-side synchronization fence across the process boundary.
        // Falls back to a CPU queue_wait_idle when semaphores are unavailable.
        let parent_pid = std::env::var("STOKES_VK_DEVICE_INFO")
            .ok()
            .and_then(|s| serde_json::from_str::<crate::vk_shared::VulkanDeviceInfo>(&s).ok())
            .map(|info| info.parent_pid)
            .unwrap_or(0);

        let vk_image = self.vk_image.as_ref().unwrap();
        let sem_handle: i64 = unsafe { vk_image.signal_and_export_semaphore(parent_pid) };

        // If we couldn't get a semaphore, fall back to a CPU wait so the
        // parent still sees a complete frame.
        if sem_handle == -1 || sem_handle == 0 {
            if let Some(device) = self.ash_device.as_ref() {
                let queue = unsafe { device.get_device_queue(self.queue_family_index, 0) };
                unsafe { device.queue_wait_idle(queue).ok() };
            }
        }

        let vk_image = self.vk_image.as_ref().unwrap();
        let width = vk_image.width;
        let height = vk_image.height;
        let vk_format = vk_image.format.as_raw();
        let alloc_size = vk_image.alloc_size;

        // Retrieve the parent PID from VulkanDeviceInfo (set via env at startup).
        let parent_pid = std::env::var("STOKES_VK_DEVICE_INFO")
            .ok()
            .and_then(|s| serde_json::from_str::<VulkanDeviceInfo>(&s).ok())
            .map(|info| info.parent_pid)
            .unwrap_or(0);

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
    let mut process = TabProcess::new(tab_id, server_name)?;
    process.run().await
}