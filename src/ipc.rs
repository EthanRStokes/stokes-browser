// Inter-Process Communication module for browser processes
//
// Uses the `ipc-channel` crate which is built on OS-native IPC primitives
// (Unix domain sockets with SCM_RIGHTS fd-passing on Linux/macOS, named pipes
// on Windows).  Compared to the previous length-prefixed bincode-over-
// UnixStream approach this gives us:
//
//   • Zero nonblocking-mode toggling – `IpcReceiver::try_recv` never calls
//     fcntl; the kernel notifies readiness through the OS IPC mechanism.
//   • Efficient large-message handling – the OS can use kernel buffers and
//     avoids user-space copies for messages that fit in the socket buffer.
//   • A clean `IpcReceiverSet` API for polling *many* tab receivers at once
//     without spawning per-tab threads.

use crate::display_list::{DisplayFontData, DisplayListFrame};
use crate::events::{MouseEventButtons, UiEvent};
use crate::fragment_tree::FragmentTree;
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
    DisplayListRendered {
        frame: DisplayListFrame,
        fonts: Vec<DisplayFontData>,
    },
    /// New: Fragment tree with pre-rendered display commands.
    /// The main process composites from this instead of the raw display list.
    FragmentTreeRendered {
        tree: FragmentTree,
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
//
// The child creates both channel pairs and sends the parent's halves:
//   - tab_to_parent_rx: the parent's receiver for messages from the tab
//   - parent_to_tab_tx: the parent's sender for messages to the tab

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
