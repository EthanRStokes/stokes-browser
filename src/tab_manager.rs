use crate::display_list::DisplayListFrame;
use crate::fragment_tree::FragmentTree;
use crate::ipc::{IpcServer, ParentIpcChannel, ParentToTabMessage, TabToParentMessage};
use std::collections::HashMap;
use std::io;
use std::process::{Child, Command};
use std::thread;
use taffy::Point;

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
    /// Fragment tree received from the tab process for compositor-side rendering.
    pub fragment_tree: Option<FragmentTree>,
    pub fragment_tree_generation: u64,
    pub fragment_tree_frame_id: u64,
}

/// A compositable frame from a tab process.
pub struct RenderedFrame {
    pub frame: DisplayListFrame,
}

/// Manages all tab processes
pub struct TabManager {
    tabs: HashMap<String, ManagedTab>,
    next_tab_id: usize,
}

impl TabManager {
    /// Create a new tab manager
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            tabs: HashMap::new(),
            next_tab_id: 1,
        })
    }

    /// Create a new tab process
    pub fn create_tab(&mut self) -> io::Result<String> {
        let tab_id = format!("tab{}", self.next_tab_id);
        self.next_tab_id += 1;

        let server = IpcServer::new()?;
        let server_name = server.server_name().to_string();
        let exe_path = std::env::current_exe()?;

        let child = Command::new(exe_path)
            .arg("--tab-process")
            .arg(&tab_id)
            .arg(&server_name)
            .spawn()?;

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
            fragment_tree: None,
            fragment_tree_generation: 0,
            fragment_tree_frame_id: 0,
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
            tab.channel.send(message)?;
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
                TabToParentMessage::SyncFonts(_) => {}
                TabToParentMessage::FragmentTreeRendered {
                    generation,
                    frame_id,
                    tree,
                } => {
                    let should_replace = generation > tab.fragment_tree_generation
                        || (generation == tab.fragment_tree_generation
                            && frame_id >= tab.fragment_tree_frame_id);

                    if should_replace {
                        tab.fragment_tree_generation = generation;
                        tab.fragment_tree_frame_id = frame_id;
                        tab.fragment_tree = Some(tree);
                    }
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
                TabToParentMessage::ShellProvider(_msg) => {
                    // Shell provider messages are handled by the browser process, not the tab manager
                    // This is just here for exhaustive pattern matching
                },
                TabToParentMessage::UpdateButtons(_) => {},
                TabToParentMessage::Navigate { .. } => todo!(),
            }
        }
    }

    /// Close a tab
    pub fn close_tab(&mut self, tab_id: &str) -> io::Result<()> {
        if let Some(mut tab) = self.tabs.remove(tab_id) {
            let _ = tab.channel.send(ParentToTabMessage::Shutdown);
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
            let _ = tab.channel.send(ParentToTabMessage::Shutdown);
            let mut process = tab.process;
            let _ = process.kill();
        }
    }
}
