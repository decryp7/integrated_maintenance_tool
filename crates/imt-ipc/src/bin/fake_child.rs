//! A fake child for the imt-ipc tests (never bundled). Flags: `--exit-now` (dies before listening),
//! `--no-listen` (reads the handshake and never binds), `--write-pid=<file>` (records its pid, for the
//! orphan test), `--say=<text>` (writes a line to stderr first). It serves the test `Echo` service, whose request picks crashes, panics, delays and
//! (`env`) reports an environment variable.

use imt_ipc::server::{child_main, RuntimeKind, ServeOptions};
use imt_ipc::testing::echo_server::{Echo, EchoServer};
use imt_ipc::testing::{EchoReply, EchoRequest};
use imt_ipc::RemoteError;
use std::time::Duration;
use tonic::{Request, Response, Status};

struct Svc;

#[tonic::async_trait]
impl Echo for Svc {
    async fn echo(&self, req: Request<EchoRequest>) -> Result<Response<EchoReply>, Status> {
        let r = req.into_inner();
        tokio::time::sleep(Duration::from_millis(r.delay_ms)).await;
        match r.action.as_str() {
            "abort" => crash(),
            "panic" => panic!("handler panicked"),
            "error" => Err(imt_ipc::error::to_status(
                tonic::Code::FailedPrecondition,
                &RemoteError::new(7, "typed failure"),
            )),
            // Reports one of its environment variables (the environment tests).
            "env" => Ok(Response::new(EchoReply {
                text: std::env::var(&r.text).unwrap_or_else(|_| "<unset>".into()),
            })),
            _ => Ok(Response::new(EchoReply { text: r.text })),
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(file) = args.iter().find_map(|a| a.strip_prefix("--write-pid=")) {
        let _ = std::fs::write(file, std::process::id().to_string());
    }
    for say in args.iter().filter_map(|a| a.strip_prefix("--say=")) {
        eprintln!("{say}");
    }
    if args.iter().any(|a| a == "--exit-now") {
        std::process::exit(3);
    }
    if args.iter().any(|a| a == "--no-listen") {
        loop {
            std::thread::sleep(Duration::from_secs(3600));
        }
    }
    let opts = ServeOptions {
        name: "fake child".into(),
        ..ServeOptions::default()
    };
    let r = child_main(RuntimeKind::MultiThread, opts, |_| {
        tonic::service::Routes::new(EchoServer::new(Svc))
    });
    if let Err(e) = r {
        eprintln!("fake child: {e}");
        std::process::exit(1);
    }
}

/// Dies like a crash. On Windows `abort` goes through Windows Error Reporting, which can keep a crashing process
/// alive for seconds on a busy machine (and the crash then looks like a hang); exit at once instead, with the
/// status an abort leaves (STATUS_STACK_BUFFER_OVERRUN).
fn crash() -> ! {
    #[cfg(windows)]
    std::process::exit(0xC000_0409_u32 as i32);
    #[cfg(not(windows))]
    std::process::abort();
}
