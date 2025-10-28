// Tab Manager - manages tab processes from the parent process
use crate::ipc::{IpcChannel, IpcServer, ParentToTabMessage, TabToParentMessage};
use shared_memory::ShmemConf;
use skia_safe::{AlphaType, ColorType, Data, Image, ImageInfo};
use std::collections::HashMap;
use std::io;
use std::process::{Child, Command};
use std::sync::Arc;
use std::thread;

/// Represents a managed tab process
pub struct ManagedTab {
    pub id: String,
    pub title: String,
    pub url: String,
    pub is_loading: bool,
    process: Child,
    channel: IpcChannel,
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
    ipc_server: Arc<IpcServer>,
    next_tab_id: usize,
}

impl TabManager {
    /// Create a new tab manager
    pub fn new() -> io::Result<Self> {
        let ipc_server = Arc::new(IpcServer::new()?);

        Ok(Self {
            tabs: HashMap::new(),
            ipc_server,
            next_tab_id: 1,
        })
    }

    /// Create a new tab process
    pub fn create_tab(&mut self) -> io::Result<String> {
        let tab_id = format!("tab{}", self.next_tab_id);
        self.next_tab_id += 1;

        // Get the current executable path
        let exe_path = std::env::current_exe()?;

        // Spawn the tab process
        let child = Command::new(exe_path)
            .arg("--tab-process")
            .arg(&tab_id)
            .arg(self.ipc_server.socket_path().to_str().unwrap())
            .spawn()?;

        // Accept the connection from the tab process
        let channel = self.ipc_server.accept()?;

        let managed_tab = ManagedTab {
            id: tab_id.clone(),
            title: "New Tab".to_string(),
            url: String::new(),
            is_loading: false,
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
        if let Some(tab) = self.tabs.get_mut(tab_id) {
            tab.channel.send(&message)?;
        }
        Ok(())
    }

    /// Poll messages from all tabs (non-blocking)
    pub fn poll_messages(&mut self) -> Vec<(String, TabToParentMessage)> {
        let mut messages = Vec::new();

        for (tab_id, tab) in self.tabs.iter_mut() {
            // Try to receive messages without blocking
            while let Ok(Some(msg)) = tab.channel.try_receive::<TabToParentMessage>() {
                messages.push((tab_id.clone(), msg));
            }
        }

        messages
    }

    /// Process a message from a tab and update state
    pub fn process_tab_message(&mut self, tab_id: &str, message: TabToParentMessage) {
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
                TabToParentMessage::FrameRendered { shmem_name, width, height } => {
                    // Load the frame from shared memory
                    if let Ok(frame) = Self::load_frame_from_shmem(&shmem_name, width, height) {
                        tab.rendered_frame = Some(frame);
                    }
                }
                TabToParentMessage::CursorChanged(_cursor_type) => {
                    // TODO: Handle cursor changes
                }
                TabToParentMessage::Ready => {
                    println!("Tab {} is ready", tab_id);
                }
                TabToParentMessage::NavigateRequest(url) => {
                    // Handle navigation request from web content (e.g., link clicks)
                    println!("Navigation request from tab {}: {}", tab_id, url);
                    tab.url = url.clone();
                    // The actual navigation will be handled by sending Navigate message back to the tab
                }
                TabToParentMessage::NavigateRequestInNewTab(_url) => {
                    // Navigate in new tab request is handled by the browser process, not the tab manager
                    // This is just here for exhaustive pattern matching
                }
                TabToParentMessage::Alert(_message) => {
                    // Alert is handled by the browser process, not the tab manager
                    // This is just here for exhaustive pattern matching
                }
            }
        }
    }

    /// Load a rendered frame from shared memory
    fn load_frame_from_shmem(shmem_name: &str, width: u32, height: u32) -> io::Result<RenderedFrame> {
        let shmem = ShmemConf::new()
            .os_id(shmem_name)
            .open()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        let size = (width * height * 4) as usize;

        // Copy the data from shared memory
        let data = unsafe {
            let slice = std::slice::from_raw_parts(shmem.as_ptr() as *const u8, size);
            Data::new_copy(slice)
        };

        // Create an image from the data
        let image_info = ImageInfo::new(
            (width as i32, height as i32),
            ColorType::RGBA8888,
            AlphaType::Premul,
            None,
        );

        let row_bytes = width as usize * 4;
        let image = Image::from_raster_data(
            &image_info,
            data,
            row_bytes,
        ).ok_or_else(|| io::Error::new(io::ErrorKind::Other, "Failed to create image"))?;

        Ok(RenderedFrame {
            image,
            width,
            height,
        })
    }

    /// Close a tab
    pub fn close_tab(&mut self, tab_id: &str) -> io::Result<()> {
        if let Some(mut tab) = self.tabs.remove(tab_id) {
            // Send shutdown message
            let _ = tab.channel.send(&ParentToTabMessage::Shutdown);

            // Wait for process to exit (with timeout)
            thread::sleep(std::time::Duration::from_millis(100));

            // Kill if still running
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
        // Clean up all tab processes
        for (_, mut tab) in self.tabs.drain() {
            let _ = tab.channel.send(&ParentToTabMessage::Shutdown);
            let _ = tab.process.kill();
        }
    }
}
