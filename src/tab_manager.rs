// Tab Manager - manages tab processes from the parent process
use crate::ipc::{IpcServer, ParentIpcChannel, ParentToTabMessage, TabToParentMessage};
use crate::vk_shared::{import_vk_image_raw, ImportedVkImage, VulkanDeviceInfo};
use ash::vk::{self, Handle};
use std::collections::HashMap;
use std::io;
use std::process::{Child, Command};
use std::thread;
use std::sync::Arc;
use taffy::Point;
use vulkano::device::{Device, Queue};
use vulkano::device::physical::PhysicalDevice;
use vulkano::format::Format;
use vulkano::instance::Instance;
use vulkano::memory::allocator::MemoryAllocator;

/// Represents a managed tab process
pub struct ManagedTab {
    pub id: String,
    pub title: String,
    pub url: String,
    pub is_loading: bool,
    pub zoom: f32,
    pub viewport_scroll: Point<f64>,
    process: Child,
    channel: ParentIpcChannel,
    pub rendered_frame: Option<RenderedFrame>,
}

/// A rendered frame from a tab process
pub struct RenderedFrame {
    /// The raw VkImage imported from the tab process.  Blitted directly to the
    /// swapchain via `vkCmdBlitImage` — no Skia texture wrapping needed.
    pub width: u32,
    pub height: u32,
    /// Shared owner of the imported VkImage + VkDeviceMemory.
    /// The renderer keeps an extra clone while a frame is in flight.
    pub vk_guard: Arc<ImportedVkImage>,
    /// Exported render-complete semaphore handle from the tab frame.
    /// Linux: local duplicated fd (or -1 when unavailable). Windows: HANDLE value (or 0).
    pub sem_handle: i64,
}

/// Manages all tab processes
pub struct TabManager {
    tabs: HashMap<String, ManagedTab>,
    next_tab_id: usize,
    vk_device_info: Option<VulkanDeviceInfo>,
    /// ash Instance / PhysicalDevice / Device for importing VkImages
    ash_instance: Option<ash::Instance>,
    ash_physical_device: Option<ash::vk::PhysicalDevice>,
    ash_device: Option<ash::Device>,
    /// Queue used for semaphore-wait submits and image layout transitions.
    ash_queue: Option<ash::vk::Queue>,
    vk_instance: Option<Arc<Instance>>,
    vk_physical_device: Option<Arc<PhysicalDevice>>,
    vk_device: Option<Arc<Device>>,
    vk_allocator: Option<Arc<dyn MemoryAllocator>>,
    vk_queue: Option<Arc<Queue>>,
    ash_queue_family_index: u32,
}

impl TabManager {
    /// Create a new tab manager (without Vulkan context yet).
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            tabs: HashMap::new(),
            next_tab_id: 1,
            vk_device_info: None,
            ash_instance: None,
            ash_physical_device: None,
            ash_device: None,
            ash_queue: None,
            vk_instance: None,
            vk_physical_device: None,
            vk_device: None,
            vk_allocator: None,
            vk_queue: None,
            ash_queue_family_index: 0,
        })
    }

    /// Supply the Vulkan context from the parent window so the tab manager can
    /// import VkImages from tab processes.
    pub fn set_vulkan_context(
        &mut self,
        device_info: VulkanDeviceInfo,
        ash_instance: ash::Instance,
        ash_physical_device: ash::vk::PhysicalDevice,
        ash_device: ash::Device,
        ash_queue: ash::vk::Queue,
        vk_instance: Arc<Instance>,
        vk_physical_device: Arc<PhysicalDevice>,
        vk_device: Arc<Device>,
        vk_allocator: Arc<dyn MemoryAllocator>,
        vk_queue: Arc<Queue>,
        ash_queue_family_index: u32,
    ) {
        self.vk_device_info = Some(device_info);
        self.ash_instance = Some(ash_instance);
        self.ash_physical_device = Some(ash_physical_device);
        self.ash_device = Some(ash_device);
        self.ash_queue = Some(ash_queue);
        self.vk_instance = Some(vk_instance);
        self.vk_physical_device = Some(vk_physical_device);
        self.vk_device = Some(vk_device);
        self.vk_allocator = Some(vk_allocator);
        self.vk_queue = Some(vk_queue);
        self.ash_queue_family_index = ash_queue_family_index;
    }

    /// Create a new tab process
    pub fn create_tab(&mut self) -> io::Result<String> {
        let tab_id = format!("tab{}", self.next_tab_id);
        self.next_tab_id += 1;

        // Create a fresh one-shot server for this tab.
        let server = IpcServer::new()?;
        let server_name = server.server_name().to_string();

        // Get the current executable path
        let exe_path = std::env::current_exe()?;

        // Spawn the tab process, passing the server name.
        let mut cmd = Command::new(exe_path);
        cmd.arg("--tab-process")
            .arg(&tab_id)
            .arg(&server_name);

        // Pass VulkanDeviceInfo if available
        if let Some(ref info) = self.vk_device_info {
            let info_json = serde_json::to_string(info).unwrap_or_default();
            cmd.env("STOKES_VK_DEVICE_INFO", info_json);
        }

        let child = cmd.spawn()?;

        // Block until the tab process completes the bootstrap handshake.
        let channel = server.accept()?;

        let managed_tab = ManagedTab {
            id: tab_id.clone(),
            title: "New Tab".to_string(),
            url: String::new(),
            is_loading: false,
            zoom: 1.0,
            viewport_scroll: Point { x: 0.0, y: 0.0 },
            process: child,
            channel,
            rendered_frame: None,
        };

        self.tabs.insert(tab_id.clone(), managed_tab);
        Ok(tab_id)
    }

    /// Get a tab by ID
    #[inline]
    pub fn get_tab(&self, tab_id: &str) -> Option<&ManagedTab> {
        self.tabs.get(tab_id)
    }

    /// Get a mutable tab by ID
    #[inline]
    pub fn get_tab_mut(&mut self, tab_id: &str) -> Option<&mut ManagedTab> {
        self.tabs.get_mut(tab_id)
    }

    /// Send a message to a tab
    pub fn send_to_tab(&mut self, tab_id: &str, message: ParentToTabMessage) -> io::Result<()> {
        if let Some(tab) = self.tabs.get(tab_id) {
            tab.channel.send(&message)?;
        }
        Ok(())
    }

    pub fn poll_messages(&mut self) -> Vec<(String, TabToParentMessage)> {
        let mut messages = Vec::new();

        for (tab_id, tab) in self.tabs.iter() {
            while let Ok(Some(msg)) = tab.channel.try_receive() {
                messages.push((tab_id.clone(), msg));
            }
        }

        messages
    }

    /// Process a message from a tab and update state
    pub fn process_tab_message(
        &mut self,
        tab_id: &str,
        message: TabToParentMessage,
    ) {
        if let Some(tab) = self.tabs.get_mut(tab_id) {
            match message {
                TabToParentMessage::NavigationStarted(url) => {
                    tab.is_loading = true;
                    tab.url = url;
                }
                TabToParentMessage::NavigationCompleted { url, title } => {
                    tab.is_loading = false;
                    tab.url = url;
                    tab.title = title;

                    // todo conditional reset scroll
                    tab.viewport_scroll = Point::default();
                }
                TabToParentMessage::NavigationFailed(error) => {
                    tab.is_loading = false;
                    eprintln!("Navigation failed in tab {}: {}", tab_id, error);
                }
                TabToParentMessage::TitleChanged(title) => {
                    tab.title = title;
                }
                TabToParentMessage::LoadingStateChanged(is_loading) => {
                    tab.is_loading = is_loading;
                }
                TabToParentMessage::FrameRendered { mem_handle, width, height, vk_format, alloc_size, sem_handle } => {
                    let format = Format::try_from(ash::vk::Format::from_raw(vk_format)).expect("Invalid VkFormat from tab process");
                    if let (Some(inst), Some(phys), Some(dev), Some(allocator)) = (
                        self.vk_instance.as_ref(),
                        self.vk_physical_device.as_ref(),
                        self.vk_device.as_ref(),
                        self.vk_allocator.as_ref(),
                    ) {
                        // On Linux, the mem_handle is a raw fd number from the
                        // child process — it's not valid in our fd table.  Use
                        // pidfd_getfd() (Linux 5.6+) to duplicate the child's
                        // DMA-BUF fd into our process.
                        #[cfg(not(windows))]
                        let local_handle = {
                            let child_pid = tab.process.id() as libc::pid_t;
                            let target_fd = mem_handle as libc::c_int;

                            const SYS_PIDFD_OPEN: libc::c_long = 434;
                            const SYS_PIDFD_GETFD: libc::c_long = 438;

                            // pidfd_open(pid, flags) → pidfd
                            let pidfd = unsafe { libc::syscall(SYS_PIDFD_OPEN, child_pid, 0 as libc::c_uint) } as libc::c_int;
                            if pidfd < 0 {
                                let err = std::io::Error::last_os_error();
                                eprintln!("[TabManager] pidfd_open({}) failed: {}", child_pid, err);
                                return;
                            }

                            // pidfd_getfd(pidfd, targetfd, flags) → local fd
                            let local_fd = unsafe { libc::syscall(SYS_PIDFD_GETFD, pidfd, target_fd, 0 as libc::c_uint) } as libc::c_int;
                            unsafe { libc::close(pidfd) };

                            if local_fd < 0 {
                                let err = std::io::Error::last_os_error();
                                eprintln!(
                                    "[TabManager] pidfd_getfd(pid={}, fd={}) failed: {}",
                                    child_pid, target_fd, err
                                );
                                return;
                            }

                            local_fd as u64
                        };
                        #[cfg(windows)]
                        let local_handle = mem_handle;

                        #[cfg(not(windows))]
                        let local_sem_handle: i64 = {
                            if sem_handle == -1 {
                                -1
                            } else {
                                let child_pid = tab.process.id() as libc::pid_t;
                                let target_fd = sem_handle as libc::c_int;

                                const SYS_PIDFD_OPEN: libc::c_long = 434;
                                const SYS_PIDFD_GETFD: libc::c_long = 438;

                                let pidfd = unsafe { libc::syscall(SYS_PIDFD_OPEN, child_pid, 0 as libc::c_uint) } as libc::c_int;
                                if pidfd < 0 {
                                    let err = std::io::Error::last_os_error();
                                    eprintln!("[TabManager] pidfd_open({}) for semaphore failed: {}", child_pid, err);
                                    -1
                                } else {
                                    let local_fd = unsafe { libc::syscall(SYS_PIDFD_GETFD, pidfd, target_fd, 0 as libc::c_uint) } as libc::c_int;
                                    unsafe { libc::close(pidfd) };
                                    if local_fd < 0 {
                                        let err = std::io::Error::last_os_error();
                                        eprintln!(
                                            "[TabManager] pidfd_getfd(pid={}, sem_fd={}) failed: {}",
                                            child_pid, target_fd, err
                                        );
                                        -1
                                    } else {
                                        local_fd as i64
                                    }
                                }
                            }
                        };
                        #[cfg(windows)]
                        let local_sem_handle: i64 = sem_handle;

                        match unsafe {
                            import_vk_image_raw(
                                inst.clone(),
                                phys.clone(),
                                dev.clone(),
                                allocator.clone(),
                                local_handle,
                                width,
                                height,
                                format,
                                alloc_size,
                            )
                        } {
                            Ok(vk_guard) => {
                                // The semaphore is intentionally NOT waited on here.
                                // It is stored on the frame and consumed by the blit
                                // path (vk_blit_tab_then_present) as an extra wait
                                // semaphore on the same vkQueueSubmit that does the
                                // blit — so the GPU handles synchronization without
                                // any CPU stall on the main thread.
                                tab.rendered_frame = Some(RenderedFrame {
                                    width,
                                    height,
                                    vk_guard: Arc::new(vk_guard),
                                    sem_handle: local_sem_handle,
                                });
                            }
                            Err(e) => {
                                eprintln!("[TabManager] Failed to import VkImage: {}", e);
                            }
                        }
                    }
                }
                TabToParentMessage::Ready => {
                    println!("Tab {} is ready", tab_id);
                }
                TabToParentMessage::NavigateRequest(url) => {
                    println!("Navigation request from tab {}: {}", tab_id, url);
                    tab.url = url.clone();
                }
                TabToParentMessage::NavigateRequestInNewTab(_url) => {
                    // Handled by the browser process
                }
                TabToParentMessage::Alert(_message) => {
                    // Handled by the browser process
                }
                TabToParentMessage::ShellProvider(_msg) => {
                    // Handled by the browser process
                },
                TabToParentMessage::UpdateButtons(_) => {},
                TabToParentMessage::Navigate { .. } => todo!(),
            }
        }
    }


    /// Close a tab
    pub fn close_tab(&mut self, tab_id: &str) -> io::Result<()> {
        if let Some(mut tab) = self.tabs.remove(tab_id) {
            let _ = tab.channel.send(&ParentToTabMessage::Shutdown);
            thread::sleep(std::time::Duration::from_millis(100));
            let _ = tab.process.kill();
        }
        Ok(())
    }

    /// Get all tab IDs
    pub fn tab_ids(&self) -> Vec<String> {
        self.tabs.keys().cloned().collect()
    }

    /// Get tab count
    #[inline]
    pub fn tab_count(&self) -> usize {
        self.tabs.len()
    }
}

impl Drop for TabManager {
    fn drop(&mut self) {
        for (_, tab) in self.tabs.drain() {
            let _ = tab.channel.send(&ParentToTabMessage::Shutdown);
            let mut process = tab.process;
            let _ = process.kill();
        }
    }
}
