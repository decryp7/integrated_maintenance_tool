//! Establishing a connection (ADR 0017 §2): **the host listens, the child connects.**
//!
//! The host creates a [`Listener`] (a private endpoint and a fresh 256-bit token) *before* it starts the child,
//! so nobody can squat the address; the child learns the address and token, connects with [`connect`] and sends
//! [`Frame::Hello`]. The host's [`Listener::accept`] checks the token in constant time and the peer's process
//! id against the process it started, answers [`Frame::Welcome`], and both sides start a [`Peer`].

use crate::endpoint::PrivateEndpoint;
use crate::frame::{self, Frame, FrameError, TOKEN_BYTES, VERSION};
use crate::peer::{Duplex, Handler, Peer, PeerOptions};
use crate::IpcError;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long either side waits for the other's handshake frame once connected.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// Largest handshake frame (they are tiny).
const HANDSHAKE_MAX: usize = 1024;

/// Who may open the host's endpoint (the token is checked either way).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Only the user running IMT.
    Owner,
    /// The user and the Administrators group: for an elevated child, which may run as another account
    /// (an over-the-shoulder administrator). Unix: same as [`Access::Owner`].
    OwnerAndAdmins,
}

/// A per-launch token.
#[derive(Clone, PartialEq, Eq)]
pub struct Token(pub [u8; TOKEN_BYTES]);

impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Token(..)")
    }
}

impl Token {
    pub fn random() -> std::io::Result<Token> {
        let mut t = [0u8; TOKEN_BYTES];
        getrandom::fill(&mut t).map_err(|e| std::io::Error::other(format!("no random numbers available: {e}")))?;
        Ok(Token(t))
    }

    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }

    pub fn from_hex(s: &str) -> Option<Token> {
        let s = s.trim();
        if s.len() != TOKEN_BYTES * 2 || !s.is_ascii() {
            return None;
        }
        let mut t = [0u8; TOKEN_BYTES];
        for (i, b) in t.iter_mut().enumerate() {
            *b = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()?;
        }
        Some(Token(t))
    }

    /// Constant-time comparison.
    pub fn matches(&self, other: &[u8; TOKEN_BYTES]) -> bool {
        let mut diff = 0u8;
        for (a, b) in self.0.iter().zip(other) {
            diff |= a ^ b;
        }
        diff == 0
    }
}

/// The host's side before the child connected.
pub struct Listener {
    endpoint: PrivateEndpoint,
    token: Token,
    inner: platform::Listening,
}

impl Listener {
    /// A private endpoint named `<prefix>-<random>` and a fresh token. The address exists from now on.
    pub fn create(prefix: &str, access: Access) -> std::io::Result<Listener> {
        let endpoint = PrivateEndpoint::create_named(prefix)?;
        let inner = platform::listen(&endpoint.address, access)?;
        Ok(Listener {
            endpoint,
            token: Token::random()?,
            inner,
        })
    }

    pub fn address(&self) -> &str {
        &self.endpoint.address
    }

    pub fn token(&self) -> &Token {
        &self.token
    }

    /// The private directory of the endpoint (Unix), where a token file for an elevated child can go.
    pub fn private_dir(&self) -> Option<&std::path::Path> {
        self.endpoint.dir()
    }

    /// Waits (at most `window`) for the child to connect and authenticate, then starts the peer. `expected_pid`
    /// is the process the host started (checked where the platform reports the peer's pid); `exited` is asked
    /// while waiting and ends the wait with its description when the child died first.
    pub async fn accept(
        self,
        expected_pid: Option<u32>,
        window: Duration,
        mut exited: impl FnMut() -> Option<String>,
        handler: Arc<dyn Handler>,
        opts: PeerOptions,
    ) -> Result<(Peer, PrivateEndpoint), IpcError> {
        let label = opts.label.clone();
        let started = Instant::now();
        let mut accepting = std::pin::pin!(self.inner.accept());
        let mut stream = loop {
            tokio::select! {
                r = &mut accepting => break r.map_err(|e| IpcError::Spawn(format!("cannot accept {label}'s connection: {e}")))?,
                _ = tokio::time::sleep(Duration::from_millis(25)) => {
                    if let Some(why) = exited() {
                        return Err(IpcError::Died(why));
                    }
                    if started.elapsed() > window {
                        return Err(IpcError::Died(format!("{label} did not connect within {} s", window.as_secs())));
                    }
                }
            }
        };
        let hello = tokio::time::timeout(HANDSHAKE_TIMEOUT, frame::read_frame(&mut stream.io, HANDSHAKE_MAX)).await;
        let (version, token, pid) = match hello {
            Err(_) => return Err(IpcError::Protocol(format!("{label} connected but sent no handshake"))),
            Ok(Err(e)) => return Err(handshake_error(&label, e)),
            Ok(Ok(Frame::Hello { version, token, pid })) => (version, token, pid),
            Ok(Ok(other)) => return Err(IpcError::Protocol(format!("{label} sent {other:?} instead of its handshake"))),
        };
        if !self.token.matches(&token) {
            return Err(IpcError::Protocol(format!("{label} presented a wrong session token")));
        }
        if version != VERSION {
            return Err(IpcError::Protocol(format!(
                "{label} speaks protocol version {version}, this IMT speaks {VERSION}; reinstall IMT"
            )));
        }
        if let Some(expected) = expected_pid {
            // The peer's pid as the OS reports it, else what it claims; both must be the started process.
            let actual = stream.peer_pid.unwrap_or(pid);
            if actual != expected || pid != expected {
                return Err(IpcError::Protocol(format!(
                    "the process that connected (pid {actual}) is not {label} (pid {expected})"
                )));
            }
        }
        frame::write_frame(&mut stream.io, &Frame::Welcome { version: VERSION }, HANDSHAKE_MAX)
            .await
            .map_err(|e| handshake_error(&label, e))?;
        let peer = Peer::start(stream.io, handler, opts, &tokio::runtime::Handle::current());
        Ok((peer, self.endpoint))
    }
}

fn handshake_error(label: &str, e: FrameError) -> IpcError {
    match e {
        FrameError::Closed | FrameError::Io(_) => IpcError::Died(format!("{label} closed the connection during the handshake ({e})")),
        other => IpcError::Protocol(format!("{label}: {other}")),
    }
}

/// The child's side of the handshake: connects to `address` and authenticates with `token`; returns the stream
/// to start a [`Peer`] on (after the child locked itself down).
pub async fn handshake(address: &str, token: &Token, label: &str) -> Result<Box<dyn Duplex>, IpcError> {
    Ok(Box::new(platform_handshake(address, token, label).await?))
}

/// [`handshake`] on this platform's stream type (Unix: so the child can move it to another runtime).
pub(crate) async fn platform_handshake(address: &str, token: &Token, label: &str) -> Result<platform::Stream, IpcError> {
    let mut io = platform::connect(address)
        .await
        .map_err(|e| IpcError::Spawn(format!("cannot connect to {label}: {e}")))?;
    let hello = Frame::Hello {
        version: VERSION,
        token: token.0,
        pid: std::process::id(),
    };
    frame::write_frame(&mut io, &hello, HANDSHAKE_MAX)
        .await
        .map_err(|e| handshake_error(label, e))?;
    match tokio::time::timeout(HANDSHAKE_TIMEOUT, frame::read_frame(&mut io, HANDSHAKE_MAX)).await {
        Err(_) => Err(IpcError::Protocol(format!("{label} did not answer the handshake"))),
        Ok(Err(e)) => Err(handshake_error(label, e)),
        Ok(Ok(Frame::Welcome { version: VERSION })) => Ok(io),
        Ok(Ok(other)) => Err(IpcError::Protocol(format!("{label} answered the handshake with {other:?}"))),
    }
}

/// [`handshake`], then a [`Peer`] on the current runtime.
pub async fn connect(address: &str, token: &Token, handler: Arc<dyn Handler>, opts: PeerOptions) -> Result<Peer, IpcError> {
    let io = handshake(address, token, &opts.label).await?;
    Ok(Peer::start(io, handler, opts, &tokio::runtime::Handle::current()))
}

/// A connected stream and the peer's process id, when the OS reports it.
pub(crate) struct Accepted {
    pub io: Box<dyn Duplex>,
    pub peer_pid: Option<u32>,
}

#[cfg(unix)]
mod platform {
    use super::{Accepted, Access};
    use std::io;
    use tokio::net::{UnixListener, UnixStream};

    pub struct Listening(UnixListener);

    pub fn listen(address: &str, _access: Access) -> io::Result<Listening> {
        use std::os::unix::fs::PermissionsExt;
        // Binding needs a runtime context for tokio; a std listener converted later does not.
        let std_listener = std::os::unix::net::UnixListener::bind(address)?;
        std::fs::set_permissions(address, std::fs::Permissions::from_mode(0o600))?;
        std_listener.set_nonblocking(true)?;
        // `from_std` registers with a reactor: the shared runtime's.
        let _g = crate::runtime::shared().map_err(io::Error::other)?.enter();
        Ok(Listening(UnixListener::from_std(std_listener)?))
    }

    impl Listening {
        pub async fn accept(self) -> io::Result<Accepted> {
            let (s, _) = self.0.accept().await?;
            let peer_pid = s.peer_cred().ok().and_then(|c| c.pid()).and_then(|p| u32::try_from(p).ok());
            Ok(Accepted { io: Box::new(s), peer_pid })
        }
    }

    pub type Stream = UnixStream;

    pub async fn connect(address: &str) -> io::Result<UnixStream> {
        UnixStream::connect(address).await
    }
}

#[cfg(windows)]
mod platform {
    use super::{Accepted, Access};
    use std::io;
    use std::os::windows::io::AsRawHandle;
    use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions};
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{LocalFree, HANDLE, HLOCAL};
    use windows::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
    use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
    use windows::Win32::System::Pipes::GetNamedPipeClientProcessId;

    /// Full control for the owner (the user running IMT) and nobody else.
    const OWNER_ONLY: &str = "D:P(A;;GA;;;OW)";
    /// Also the Administrators group (an elevated child may run as another administrator account).
    const OWNER_AND_ADMINS: &str = "D:P(A;;GA;;;OW)(A;;GA;;;BA)";

    pub struct Listening(NamedPipeServer);

    pub fn listen(address: &str, access: Access) -> io::Result<Listening> {
        let sddl = match access {
            Access::Owner => OWNER_ONLY,
            Access::OwnerAndAdmins => OWNER_AND_ADMINS,
        };
        let wide: Vec<u16> = sddl.encode_utf16().chain(std::iter::once(0)).collect();
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        // SAFETY: `wide` is a valid NUL-terminated string that outlives the call; the descriptor it allocates is
        // freed below after the pipe instance (which copies it) has been created.
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(PCWSTR(wide.as_ptr()), SDDL_REVISION_1, &mut descriptor, None)
                .map_err(|e| io::Error::other(e.to_string()))?;
        }
        let mut attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: false.into(),
        };
        let mut options = ServerOptions::new();
        // The first instance: nobody can have created this name before (it is random, and we are first).
        options.first_pipe_instance(true).reject_remote_clients(true).max_instances(1);
        let _g = crate::runtime::shared().map_err(io::Error::other)?.enter();
        // SAFETY: `attributes` points at a valid descriptor for the duration of the call.
        let result = unsafe {
            options.create_with_security_attributes_raw(address, &mut attributes as *mut SECURITY_ATTRIBUTES as *mut std::ffi::c_void)
        };
        // SAFETY: allocated by the conversion call above and no longer used.
        unsafe {
            let _ = LocalFree(Some(HLOCAL(descriptor.0)));
        }
        result.map(Listening)
    }

    impl Listening {
        pub async fn accept(self) -> io::Result<Accepted> {
            self.0.connect().await?;
            let mut pid = 0u32;
            // SAFETY: the handle is a connected pipe server instance owned by `self.0`; `pid` is a valid out pointer.
            let peer_pid = unsafe { GetNamedPipeClientProcessId(HANDLE(self.0.as_raw_handle()), &mut pid) }
                .ok()
                .map(|_| pid);
            Ok(Accepted {
                io: Box::new(self.0),
                peer_pid,
            })
        }
    }

    pub type Stream = NamedPipeClient;

    pub async fn connect(address: &str) -> io::Result<NamedPipeClient> {
        const ERROR_PIPE_BUSY: i32 = 231;
        let started = std::time::Instant::now();
        loop {
            match ClientOptions::new().open(address) {
                Ok(c) => return Ok(c),
                Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) && started.elapsed() < std::time::Duration::from_secs(2) => {
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await
                }
                Err(e) => return Err(e),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_roundtrip_through_hex_and_compare() {
        let t = Token::random().unwrap();
        assert_eq!(Token::from_hex(&t.to_hex()), Some(t.clone()));
        assert!(t.matches(&t.0));
        let mut other = t.0;
        other[31] ^= 1;
        assert!(!t.matches(&other));
        assert_eq!(Token::from_hex("zz"), None);
        assert_eq!(Token::from_hex(&"g".repeat(64)), None);
        assert_eq!(format!("{t:?}"), "Token(..)");
    }
}
