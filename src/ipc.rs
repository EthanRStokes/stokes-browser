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

use std::io;
use ipc_channel::ipc::{
    self, IpcOneShotServer, IpcReceiver, IpcSender,
};
use ipc_channel::TryRecvError;
use serde::{Deserialize, Serialize};
use crate::events::{MouseEventButtons, UiEvent};

// ── fd passing helpers (libc sendmsg / recvmsg with SCM_RIGHTS) ─────────────

/// Send a single file descriptor over a `UnixDatagram` socket using SCM_RIGHTS.
#[cfg(unix)]
fn send_fd_over_socket(sock: &std::os::unix::net::UnixDatagram, fd: std::os::unix::io::RawFd) -> io::Result<()> {
    use std::os::unix::io::AsRawFd;
    use libc::{
        msghdr, iovec, cmsghdr, sendmsg,
        CMSG_SPACE, CMSG_FIRSTHDR, CMSG_DATA, CMSG_LEN,
        SOL_SOCKET, SCM_RIGHTS,
        c_void, c_int,
    };
    use std::mem;

    let dummy: u8 = 0;
    let mut iov = iovec {
        iov_base: &dummy as *const u8 as *mut c_void,
        iov_len: 1,
    };

    let cmsg_space = unsafe { CMSG_SPACE(mem::size_of::<c_int>() as u32) } as usize;
    let mut cmsg_buf = vec![0u8; cmsg_space];

    let mut msg: msghdr = unsafe { mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = cmsg_buf.as_mut_ptr() as *mut c_void;
    msg.msg_controllen = cmsg_space as _;

    unsafe {
        let cmsg: *mut cmsghdr = CMSG_FIRSTHDR(&msg);
        (*cmsg).cmsg_level = SOL_SOCKET;
        (*cmsg).cmsg_type = SCM_RIGHTS;
        (*cmsg).cmsg_len = CMSG_LEN(mem::size_of::<c_int>() as u32) as _;
        let data_ptr = CMSG_DATA(cmsg) as *mut c_int;
        *data_ptr = fd;

        let ret = sendmsg(sock.as_raw_fd(), &msg, 0);
        if ret < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// Try to receive a single file descriptor from a `UnixDatagram` socket via SCM_RIGHTS.
/// Returns `Ok(None)` if the socket has no pending message (EAGAIN/EWOULDBLOCK).
#[cfg(unix)]
fn recv_fd_from_socket(sock: &std::os::unix::net::UnixDatagram) -> io::Result<Option<std::os::unix::io::RawFd>> {
    use std::os::unix::io::AsRawFd;
    use libc::{
        msghdr, iovec, cmsghdr, recvmsg,
        CMSG_SPACE, CMSG_FIRSTHDR, CMSG_DATA,
        SOL_SOCKET, SCM_RIGHTS,
        MSG_DONTWAIT, EAGAIN, EWOULDBLOCK,
        c_void, c_int,
    };
    use std::mem;

    let mut dummy = 0u8;
    let mut iov = iovec {
        iov_base: &mut dummy as *mut u8 as *mut c_void,
        iov_len: 1,
    };

    let cmsg_space = unsafe { CMSG_SPACE(mem::size_of::<c_int>() as u32) } as usize;
    let mut cmsg_buf = vec![0u8; cmsg_space];

    let mut msg: msghdr = unsafe { mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = cmsg_buf.as_mut_ptr() as *mut c_void;
    msg.msg_controllen = cmsg_space as _;

    let ret = unsafe { recvmsg(sock.as_raw_fd(), &mut msg, MSG_DONTWAIT) };
    if ret < 0 {
        let err = io::Error::last_os_error();
        let raw = err.raw_os_error().unwrap_or(0);
        if raw == EAGAIN || raw == EWOULDBLOCK {
            return Ok(None);
        }
        return Err(err);
    }

    // Extract the fd from the control message
    unsafe {
        let cmsg: *mut cmsghdr = CMSG_FIRSTHDR(&msg);
        if cmsg.is_null() {
            return Ok(None);
        }
        if (*cmsg).cmsg_level == SOL_SOCKET && (*cmsg).cmsg_type == SCM_RIGHTS {
            let data_ptr = CMSG_DATA(cmsg) as *const c_int;
            return Ok(Some(*data_ptr));
        }
    }
    Ok(None)
}

// ── Wire message types ────────────────────────────────────────────────────────

/// Messages sent from parent (browser UI) to child (tab process)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ParentToTabMessage {
    Navigate(String),
    Reload,
    GoBack,
    GoForward,
    Resize { width: f32, height: f32 },
    Scroll { delta_x: f32, delta_y: f32 },
    Click { x: f32, y: f32, modifiers: KeyModifiers },
    UI(UiEvent),
    KeyboardInput { key_type: KeyInputType, modifiers: KeyModifiers },
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
    /// memory has been exported as an opaque fd.
    ///
    /// The fd is sent out-of-band over the per-tab `fd_socket` (a `UnixDatagram`
    /// pair created during bootstrap) using `SCM_RIGHTS`.
    ///
    /// `vk_image_handle` is the raw `VkImage` (u64) in the **child** process;
    /// the parent uses it, together with the imported memory, to build a
    /// `skia_safe::Image` for compositing.
    FrameRendered {
        /// Raw VkImage handle (u64) in the tab process address space
        vk_image_handle: u64,
        width: u32,
        height: u32,
        /// VkFormat as raw integer (ash::vk::Format::as_raw())
        vk_format: i32,
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
    /// Name of the Unix datagram socket the tab should connect to for fd passing
    pub fd_socket_path: String,
}

// ── IpcChannel (tab/child side) ───────────────────────────────────────────────

pub struct IpcChannel {
    sender: IpcSender<TabToParentMessage>,
    receiver: IpcReceiver<ParentToTabMessage>,
    /// Unix datagram socket for sending fds to the parent via SCM_RIGHTS
    pub fd_socket: std::os::unix::net::UnixDatagram,
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

    /// Send a file descriptor to the parent process via SCM_RIGHTS.
    #[cfg(unix)]
    pub fn send_fd(&self, fd: std::os::unix::io::RawFd) -> io::Result<()> {
        send_fd_over_socket(&self.fd_socket, fd)
    }
}

// ── ParentIpcChannel (parent side) ────────────────────────────────────────────

pub struct ParentIpcChannel {
    pub sender: IpcSender<ParentToTabMessage>,
    pub receiver: IpcReceiver<TabToParentMessage>,
    /// Unix datagram socket for receiving fds from the tab via SCM_RIGHTS
    pub fd_socket: std::os::unix::net::UnixDatagram,
    /// Keep the temp dir alive so the socket path remains valid
    _fd_socket_dir: tempfile::TempDir,
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

    /// Attempt to receive a file descriptor sent by the tab via SCM_RIGHTS.
    /// Returns `Ok(None)` if no fd is waiting.
    #[cfg(unix)]
    pub fn try_recv_fd(&self) -> io::Result<Option<std::os::unix::io::RawFd>> {
        recv_fd_from_socket(&self.fd_socket)
    }
}

// ── IpcServer (parent side) ───────────────────────────────────────────────────

/// Listens for a single incoming bootstrap connection from a tab process.
pub struct IpcServer {
    server: IpcOneShotServer<ChannelBootstrap>,
    server_name: String,
    /// Temp dir holding the fd socket path (kept alive until accept())
    fd_socket_dir: tempfile::TempDir,
    pub fd_socket_path: String,
    /// The parent's half of the fd socket (bound, waiting for connection)
    fd_socket: std::os::unix::net::UnixDatagram,
}

impl IpcServer {
    pub fn new() -> io::Result<Self> {
        let (server, server_name) = IpcOneShotServer::<ChannelBootstrap>::new()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        // Create a temp dir + abstract/path datagram socket for fd passing
        let dir = tempfile::tempdir()?;
        let fd_socket_path = dir.path().join("fd.sock").to_string_lossy().into_owned();
        let fd_socket = std::os::unix::net::UnixDatagram::bind(&fd_socket_path)?;
        fd_socket.set_nonblocking(true)?;

        Ok(Self { server, server_name, fd_socket_dir: dir, fd_socket_path, fd_socket })
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
            fd_socket: self.fd_socket,
            _fd_socket_dir: self.fd_socket_dir,
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

    // The fd socket path is passed as the 5th CLI argument (index 4):
    // exe --tab-process <tab_id> <server_name> <fd_socket_path>
    let fd_socket_path = std::env::args().nth(4).unwrap_or_default();

    let fd_socket = if !fd_socket_path.is_empty() {
        let sock = std::os::unix::net::UnixDatagram::unbound()?;
        sock.connect(&fd_socket_path)?;
        sock
    } else {
        // Fallback: unbound (no fd passing possible, renders will fall back to pixel copy)
        std::os::unix::net::UnixDatagram::unbound()?
    };

    bootstrap_tx
        .send(ChannelBootstrap {
            tab_to_parent_rx,
            parent_to_tab_tx,
            fd_socket_path: fd_socket_path.clone(),
        })
        .map_err(|e| io::Error::new(io::ErrorKind::BrokenPipe, e))?;

    Ok(IpcChannel {
        sender: tab_to_parent_tx,
        receiver: parent_to_tab_rx,
        fd_socket,
    })
}
