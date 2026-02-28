// Inter-Process Communication module for browser processes
//
// Uses the `ipc-channel` crate which is built on OS-native IPC primitives
// (Unix domain sockets on Linux/macOS, named pipes on Windows).
//
// Cross-process Vulkan memory handles are transmitted inline in the
// `FrameRendered` IPC message as a raw u64:
//   • Windows – a Win32 HANDLE already duplicated into the parent process.
//   • Linux   – a dup'd file descriptor number.
//
// This avoids the need for any out-of-band SCM_RIGHTS socket infrastructure.

use crate::events::{MouseEventButtons, UiEvent};
use ipc_channel::ipc::{
    self, IpcOneShotServer, IpcReceiver, IpcSender,
};
use ipc_channel::TryRecvError;
use serde::{Deserialize, Serialize};
use std::io;


// ── Wire message types ────────────────────────────────────────────────────────

/// Messages sent from parent (browser UI) to child (tab process)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ParentToTabMessage {
    Navigate(String),
    Reload,
    GoBack,
    GoForward,
    Resize { width: f32, height: f32 },
    UI(UiEvent),
    RequestFrame,
    SetScaleFactor(f32),
    SetZoom(f32),
    Shutdown,
}

/// Type of keyboard input
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum KeyInputType {
    Character(String),
    Named(String),
    Scroll { direction: ScrollDirection, amount: f32 },
}

/// Scroll direction for keyboard scrolling
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ScrollDirection {
    Up,
    Down,
    Left,
    Right,
}

/// Messages sent from child (tab process) to parent (browser UI)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TabToParentMessage {
    Navigate { url: String, retain_scroll_position: bool, is_md: bool },
    NavigationStarted(String),
    NavigationCompleted { url: String, title: String },
    NavigationFailed(String),
    TitleChanged(String),
    LoadingStateChanged(bool),
    /// A frame has been rendered into a Vulkan image whose backing device
    /// memory has been exported as a platform-specific handle.
    ///
    /// The handle is sent inline in this message:
    ///   • Windows – a Win32 HANDLE already duplicated into the parent process
    ///               via `DuplicateHandle`.  The parent must close it after importing.
    ///   • Linux   – a dup'd file-descriptor number. The parent must close it
    ///               after importing.
    FrameRendered {
        /// Platform memory handle (Win32 HANDLE on Windows, fd on Linux)
        mem_handle: u64,
        width: u32,
        height: u32,
        /// VkFormat as raw integer (ash::vk::Format::as_raw())
        vk_format: i32,
        /// Exact allocation size in bytes from the tab's vkAllocateMemory
        alloc_size: u64,
        /// Exportable semaphore handle that is signaled when the tab's GPU
        /// rendering into this image is complete.
        ///
        ///   • Linux   – a sync_fd (SYNC_FD handle type) from vkGetSemaphoreFdKHR.
        ///               The parent imports and waits on it before reading the image.
        ///               The fd is consumed by the import; do not close it manually.
        ///   • Windows – a Win32 HANDLE from vkGetSemaphoreWin32HandleKHR, already
        ///               duplicated into the parent process.
        ///
        /// A value of -1 (Linux) or 0 (Windows) means no semaphore is available
        /// and the parent must fall back to a CPU-side wait.
        sem_handle: i64,
    },
    Ready,
    NavigateRequest(String),
    NavigateRequestInNewTab(String),
    Alert(String),
    ShellProvider(crate::shell_provider::ShellProviderMessage),
    UpdateButtons(MouseEventButtons),
}

/// Keyboard modifier key state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyModifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub meta: bool,
}

// ── Bootstrap handshake ───────────────────────────────────────────────────────
//
// The parent creates an `IpcOneShotServer` and passes its opaque name string
// to the child via a CLI argument.  The child connects and sends back a
// `ChannelBootstrap` so the parent gets both halves of the bidirectional pair.
// The parent also embeds `VulkanDeviceInfo` so the child can attach to the
// same physical device and create its own Skia Vulkan context.

/// Sent once by the child over the one-shot bootstrap channel.
#[derive(Debug, Serialize, Deserialize)]
pub struct ChannelBootstrap {
    /// The parent's receiver for tab→parent messages
    pub tab_to_parent_rx: IpcReceiver<TabToParentMessage>,
    /// The parent's sender for parent→tab messages
    pub parent_to_tab_tx: IpcSender<ParentToTabMessage>,
}

// ── IpcChannel (tab/child side) ───────────────────────────────────────────────

pub struct IpcChannel {
    sender: IpcSender<TabToParentMessage>,
    receiver: IpcReceiver<ParentToTabMessage>,
}

impl IpcChannel {
    pub fn send(&self, message: &TabToParentMessage) -> io::Result<()> {
        self.sender
            .send(message.clone())
            .map_err(|e| io::Error::new(io::ErrorKind::BrokenPipe, e))
    }

    pub fn try_receive(&self) -> io::Result<Option<ParentToTabMessage>> {
        match self.receiver.try_recv() {
            Ok(msg) => Ok(Some(msg)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(e) => Err(io::Error::new(io::ErrorKind::BrokenPipe, e)),
        }
    }

    pub fn receive(&self) -> io::Result<ParentToTabMessage> {
        self.receiver
            .recv()
            .map_err(|e| io::Error::new(io::ErrorKind::BrokenPipe, e))
    }
}

// ── ParentIpcChannel (parent side) ────────────────────────────────────────────

pub struct ParentIpcChannel {
    pub sender: IpcSender<ParentToTabMessage>,
    pub receiver: IpcReceiver<TabToParentMessage>,
}

impl ParentIpcChannel {
    pub fn send(&self, message: &ParentToTabMessage) -> io::Result<()> {
        self.sender
            .send(message.clone())
            .map_err(|e| io::Error::new(io::ErrorKind::BrokenPipe, e))
    }

    pub fn try_receive(&self) -> io::Result<Option<TabToParentMessage>> {
        match self.receiver.try_recv() {
            Ok(msg) => Ok(Some(msg)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(e) => Err(io::Error::new(io::ErrorKind::BrokenPipe, e)),
        }
    }
}

// ── IpcServer (parent side) ───────────────────────────────────────────────────

/// Listens for a single incoming bootstrap connection from a tab process.
pub struct IpcServer {
    server: IpcOneShotServer<ChannelBootstrap>,
    server_name: String,
}

impl IpcServer {
    pub fn new() -> io::Result<Self> {
        let (server, server_name) = IpcOneShotServer::<ChannelBootstrap>::new()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        Ok(Self { server, server_name })
    }

    /// The opaque name to pass to the child process as a CLI argument.
    pub fn server_name(&self) -> &str {
        &self.server_name
    }

    /// Also expose as socket_path() for compatibility with existing call sites.
    pub fn socket_path(&self) -> &str {
        &self.server_name
    }

    /// Block until the tab process connects and completes the handshake.
    /// Consumes `self` because the one-shot server can only be used once.
    pub fn accept(self) -> io::Result<ParentIpcChannel> {
        let (_, bootstrap) = self.server
            .accept()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        Ok(ParentIpcChannel {
            sender: bootstrap.parent_to_tab_tx,
            receiver: bootstrap.tab_to_parent_rx,
        })
    }
}

// ── connect (child/tab side) ──────────────────────────────────────────────────

/// Called from the tab process.  Connects to the parent's bootstrap server,
/// creates the bidirectional channel pair and returns the child's end.
pub fn connect(server_name: &str) -> io::Result<IpcChannel> {
    // Channel for tab → parent messages: child keeps tx, sends rx to parent.
    let (tab_to_parent_tx, tab_to_parent_rx) =
        ipc::channel::<TabToParentMessage>()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

    // Channel for parent → tab messages: child keeps rx, sends tx to parent.
    let (parent_to_tab_tx, parent_to_tab_rx) =
        ipc::channel::<ParentToTabMessage>()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

    // Send the parent's halves over the one-shot bootstrap channel.
    let bootstrap_tx = IpcSender::<ChannelBootstrap>::connect(server_name.to_string())
        .map_err(|e| io::Error::new(io::ErrorKind::ConnectionRefused, e))?;

    bootstrap_tx
        .send(ChannelBootstrap {
            tab_to_parent_rx,
            parent_to_tab_tx,
        })
        .map_err(|e| io::Error::new(io::ErrorKind::BrokenPipe, e))?;

    Ok(IpcChannel {
        sender: tab_to_parent_tx,
        receiver: parent_to_tab_rx,
    })
}
