//! Windows: the worker's named pipe, restricted to the current user, as a stream of connections for tonic.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::windows::named_pipe::{NamedPipeClient, NamedPipeServer, ServerOptions};
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::server::Connected;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{LocalFree, HLOCAL};
use windows::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};

/// One accepted pipe connection.
pub struct PipeIo(NamedPipeServer);

impl Connected for PipeIo {
    type ConnectInfo = ();
    fn connect_info(&self) {}
}

impl AsyncRead for PipeIo {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_read(cx, buf)
    }
}

impl AsyncWrite for PipeIo {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(cx)
    }
}

/// Owner-only access: the DACL grants full control to the owner (the user running IMT) and to nobody else.
const OWNER_ONLY_SDDL: &str = "D:P(A;;GA;;;OW)";

fn create_instance(name: &str, first: bool) -> io::Result<NamedPipeServer> {
    let wide: Vec<u16> = OWNER_ONLY_SDDL.encode_utf16().chain(std::iter::once(0)).collect();
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
    options.first_pipe_instance(first).reject_remote_clients(true);
    // SAFETY: `attributes` points at a valid descriptor for the duration of the call.
    let result =
        unsafe { options.create_with_security_attributes_raw(name, &mut attributes as *mut SECURITY_ATTRIBUTES as *mut std::ffi::c_void) };
    // SAFETY: allocated by the conversion call above and no longer used.
    unsafe {
        let _ = LocalFree(Some(HLOCAL(descriptor.0)));
    }
    result
}

/// Accepts connections on `name` forever (until the stream is dropped).
pub(crate) fn incoming(name: &str) -> io::Result<ReceiverStream<io::Result<PipeIo>>> {
    let name = name.to_string();
    let mut server = create_instance(&name, true)?;
    let (tx, rx) = tokio::sync::mpsc::channel(4);
    tokio::spawn(async move {
        loop {
            let connected = server.connect().await;
            let next = create_instance(&name, false);
            let item = connected.map(|()| PipeIo(server));
            if tx.send(item).await.is_err() {
                return;
            }
            match next {
                Ok(s) => server = s,
                Err(e) => {
                    let _ = tx.send(Err(e)).await;
                    return;
                }
            }
        }
    });
    Ok(ReceiverStream::new(rx))
}

/// Opens the pipe, waiting briefly while the server is busy or not yet created.
pub(crate) async fn connect(name: &str) -> io::Result<NamedPipeClient> {
    use tokio::net::windows::named_pipe::ClientOptions;
    const ERROR_PIPE_BUSY: i32 = 231;
    loop {
        match ClientOptions::new().open(name) {
            Ok(c) => return Ok(c),
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) || e.kind() == io::ErrorKind::NotFound => {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await
            }
            Err(e) => return Err(e),
        }
    }
}
