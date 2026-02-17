// Inter-Process Communication module for browser processes
use bincode::{Decode, Encode};
use std::io::{self, Read, Write};
use std::path::PathBuf;

#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};

#[cfg(target_os = "windows")]
use uds_windows::{UnixListener, UnixStream};

/// Messages sent from parent (browser UI) to child (tab process)
#[derive(Debug, Clone, Encode, Decode)]
pub enum ParentToTabMessage {
    /// Navigate to a URL
    Navigate(String),
    /// Reload the current page
    Reload,
    /// Go back in history
    GoBack,
    /// Go forward in history
    GoForward,
    /// Resize the rendering area
    Resize { width: f32, height: f32 },
    /// Scroll the page
    Scroll { delta_x: f32, delta_y: f32 },
    /// Click at position
    Click { x: f32, y: f32, modifiers: KeyModifiers },
    /// Mouse move
    MouseMove { x: f32, y: f32 },
    /// Keyboard input (character or named key)
    KeyboardInput {
        key_type: KeyInputType,
        modifiers: KeyModifiers
    },
    /// Request a frame render
    RequestFrame,
    /// Update scale factor
    SetScaleFactor(f32),
    /// Shutdown the tab process
    Shutdown,
}

/// Type of keyboard input
#[derive(Debug, Clone, Encode, Decode)]
pub enum KeyInputType {
    /// Regular character input
    Character(String),
    /// Named key (Enter, Escape, Tab, etc.)
    Named(String),
    /// Scroll command from keyboard
    Scroll { direction: ScrollDirection, amount: f32 },
}

/// Scroll direction for keyboard scrolling
#[derive(Debug, Clone, Encode, Decode)]
pub enum ScrollDirection {
    Up,
    Down,
    Left,
    Right,
}

/// Messages sent from child (tab process) to parent (browser UI)
#[derive(Debug, Clone, Encode, Decode)]
pub enum TabToParentMessage {
    /// Navigation started
    NavigationStarted(String),
    /// Navigation completed
    NavigationCompleted { url: String, title: String },
    /// Navigation failed
    NavigationFailed(String),
    /// Page title changed
    TitleChanged(String),
    /// Loading state changed
    LoadingStateChanged(bool),
    /// Frame rendered (contains shared memory key)
    FrameRendered {
        shmem_name: String,
        width: u32,
        height: u32,
    },
    /// Cursor should change
    CursorChanged(CursorType),
    /// Tab process is ready
    Ready,
    /// Request navigation to a URL (e.g., from clicking a link)
    NavigateRequest(String),
    /// Request navigation to a URL in a new tab (e.g., from Ctrl+clicking a link)
    NavigateRequestInNewTab(String),
    /// Show an alert dialog
    Alert(String),
    /// Shell provider message (for shell operations like cursor changes, redraws, etc.)
    ShellProvider(crate::shell_provider::ShellProviderMessage),
}

#[derive(Debug, Clone, Encode, Decode)]
pub struct KeyModifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub meta: bool,
}

#[derive(Debug, Clone, Encode, Decode)]
pub enum CursorType {
    Default,
    Pointer,
    Text,
}

/// IPC channel for bidirectional communication
pub struct IpcChannel {
    stream: UnixStream,
    config: bincode::config::Configuration,
}

impl IpcChannel {
    /// Create a new IPC channel from a Unix stream
    pub fn new(stream: UnixStream) -> Self {
        Self {
            stream,
            config: bincode::config::standard(),
        }
    }

    /// Send a message through the channel
    pub fn send<T: Encode>(&mut self, message: &T) -> io::Result<()> {
        let encoded = bincode::encode_to_vec(message, self.config)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        // Send length prefix (4 bytes)
        let len = encoded.len() as u32;
        self.stream.write_all(&len.to_le_bytes())?;

        // Send the actual message
        self.stream.write_all(&encoded)?;
        self.stream.flush()?;
        Ok(())
    }

    /// Receive a message from the channel
    pub fn receive<T>(&mut self) -> io::Result<T>
    where
        T: bincode::Decode<()>,
    {
        // Read length prefix
        let mut len_bytes = [0u8; 4];
        self.stream.read_exact(&mut len_bytes)?;
        let len = u32::from_le_bytes(len_bytes) as usize;

        // Read the message
        let mut buffer = vec![0u8; len];
        self.stream.read_exact(&mut buffer)?;

        let (decoded, _) = bincode::decode_from_slice(&buffer, self.config)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        Ok(decoded)
    }

    /// Try to receive a message without blocking
    pub fn try_receive<T: for<'de> Encode + bincode::Decode<()>>(&mut self) -> io::Result<Option<T>> {
        self.stream.set_nonblocking(true)?;
        let result = self.receive();
        self.stream.set_nonblocking(false)?;

        match result {
            Ok(msg) => Ok(Some(msg)),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(e),
        }
    }
}

/// IPC server for the parent process to accept connections from tab processes
pub struct IpcServer {
    listener: UnixListener,
    socket_path: PathBuf,
}

impl IpcServer {
    /// Create a new IPC server
    pub fn new() -> io::Result<Self> {
        let socket_path = std::env::temp_dir().join(format!("stokes_browser_{}.sock", std::process::id()));

        // Remove the socket file if it already exists
        let _ = std::fs::remove_file(&socket_path);

        let listener = UnixListener::bind(&socket_path)?;
        Ok(Self {
            listener,
            socket_path,
        })
    }

    /// Get the socket path
    #[inline]
    pub fn socket_path(&self) -> &PathBuf {
        &self.socket_path
    }

    /// Accept a new connection
    pub fn accept(&self) -> io::Result<IpcChannel> {
        let (stream, _) = self.listener.accept()?;
        Ok(IpcChannel::new(stream))
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

/// Connect to an IPC server
pub fn connect(socket_path: &PathBuf) -> io::Result<IpcChannel> {
    let stream = UnixStream::connect(socket_path)?;
    Ok(IpcChannel::new(stream))
}
