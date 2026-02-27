// Tab Manager - manages tab processes from the parent process
use crate::ipc::{IpcServer, ParentIpcChannel, ParentToTabMessage, TabToParentMessage};
use crate::vk_shared::{VulkanDeviceInfo, import_skia_image};
use ash::vk::{self, Handle};
use skia_safe::gpu::DirectContext;
use skia_safe::Image;
use std::collections::HashMap;
use std::io;
use std::process::{Child, Command};
use std::thread;

/// Represents a managed tab process
pub struct ManagedTab {
    pub id: String,
    pub title: String,
    pub url: String,
    pub is_loading: bool,
    pub zoom: f32,
    process: Child,
    channel: ParentIpcChannel,
    pub rendered_frame: Option<RenderedFrame>,
}

/// A rendered frame from a tab process
pub struct RenderedFrame {
    pub image: Image,
    pub width: u32,
    pub height: u32,
}

/// Manages all tab processes
pub struct TabManager {
    tabs: HashMap<String, ManagedTab>,
    next_tab_id: usize,
    vk_device_info: Option<VulkanDeviceInfo>,
    /// ash Instance / PhysicalDevice / Device for importing VkImages
    ash_instance: Option<ash::Instance>,
    ash_physical_device: Option<vk::PhysicalDevice>,
    ash_device: Option<ash::Device>,
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
        })
    }

    /// Supply the Vulkan context from the parent window so the tab manager can
    /// import VkImages from tab processes.
    pub fn set_vulkan_context(
        &mut self,
        device_info: VulkanDeviceInfo,
        ash_instance: ash::Instance,
        ash_physical_device: vk::PhysicalDevice,
        ash_device: ash::Device,
    ) {
        self.vk_device_info = Some(device_info);
        self.ash_instance = Some(ash_instance);
        self.ash_physical_device = Some(ash_physical_device);
        self.ash_device = Some(ash_device);
    }

    /// Create a new tab process
    pub fn create_tab(&mut self) -> io::Result<String> {
        let tab_id = format!("tab{}", self.next_tab_id);
        self.next_tab_id += 1;

        // Create a fresh one-shot server for this tab.
        let server = IpcServer::new()?;
        let server_name = server.server_name().to_string();
        let fd_socket_path = server.fd_socket_path.clone();

        // Get the current executable path
        let exe_path = std::env::current_exe()?;

        // Spawn the tab process, passing the server name and fd socket path.
        let mut cmd = Command::new(exe_path);
        cmd.arg("--tab-process")
            .arg(&tab_id)
            .arg(&server_name)
            .arg(&fd_socket_path);

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
            println!("Sending message to tab {}: {:?}", tab_id, message);
            tab.channel.send(&message)?;
        }
        Ok(())
    }

    /// Poll messages from all tabs (non-blocking)
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
        gr_context: &mut DirectContext,
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
                TabToParentMessage::FrameRendered { vk_image_handle, width, height, vk_format } => {
                    // Try to receive the fd from the fd socket
                    if let Ok(Some(mem_fd)) = tab.channel.try_recv_fd() {
                        let format = vk::Format::from_raw(vk_format);
                        if let (Some(inst), Some(phys), Some(dev)) = (
                            self.ash_instance.as_ref(),
                            self.ash_physical_device.as_ref(),
                            self.ash_device.as_ref(),
                        ) {
                            match unsafe {
                                import_skia_image(
                                    inst,
                                    *phys,
                                    dev,
                                    gr_context,
                                    mem_fd,
                                    vk_image_handle,
                                    width,
                                    height,
                                    format,
                                )
                            } {
                                Ok(image) => {
                                    tab.rendered_frame = Some(RenderedFrame { image, width, height });
                                }
                                Err(e) => {
                                    eprintln!("[TabManager] Failed to import VkImage: {}", e);
                                }
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
