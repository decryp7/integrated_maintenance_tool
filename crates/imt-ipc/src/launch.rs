//! The one launcher (ADR 0017 §5): creates the host's private endpoint, starts the child (empty environment,
//! below-normal priority, a job object with the sandbox's limits on Windows, killed with IMT), tells it the
//! endpoint, the token and its sandbox, and accepts its authenticated connection.
//!
//! The child side of the same contract is [`crate::child::child_main`].

use crate::child_log::ChildLog;
use crate::endpoint::PrivateEndpoint;
use crate::link::{Access, Listener};
use crate::peer::{Handler, Peer, PeerOptions};
use crate::process::{ChildProcess, SpawnSpec};
use crate::sandbox::SandboxSpec;
use crate::supervisor::Connection;
use crate::IpcError;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// What a child reads first: one JSON line on stdin (or, for an elevated child, a file named on its command
/// line, see [`crate::elevation`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bootstrap {
    /// The host's endpoint.
    pub address: String,
    /// The per-launch token, hex.
    pub token: String,
    /// What the child applies to itself after connecting.
    #[serde(default)]
    pub sandbox: SandboxSpec,
    /// Largest frame on the connection.
    pub max_frame: usize,
    /// Free-form role the host asks for (e.g. a plugin's `elevated` role); empty for the default.
    #[serde(default)]
    pub role: String,
}

/// How to start one child.
#[derive(Debug, Clone)]
pub struct ChildSpec {
    /// The program, its arguments, environment, log and job limits (`spawn.sandbox` is the sandbox).
    pub spawn: SpawnSpec,
    /// How the connection runs (label, frame limit).
    pub peer: PeerOptions,
    /// Free-form role passed in the bootstrap.
    pub role: String,
    /// Whether the child must run with administrator rights (see [`crate::elevation`]).
    pub elevation: crate::elevation::Elevation,
}

impl ChildSpec {
    pub fn new(spawn: SpawnSpec) -> ChildSpec {
        let peer = PeerOptions::new(spawn.label.clone());
        ChildSpec {
            spawn,
            peer,
            role: String::new(),
            elevation: crate::elevation::Elevation::None,
        }
    }

    pub fn sandbox(mut self, sandbox: SandboxSpec) -> ChildSpec {
        self.spawn.sandbox = sandbox;
        self
    }
}

/// The child's process, however it was started.
pub(crate) enum Process {
    Spawned(Arc<ChildProcess>),
    #[cfg_attr(not(windows), allow(dead_code))]
    Elevated(crate::elevation::ElevatedProcess),
}

/// A started child with its authenticated connection. Killed on drop.
pub struct Child {
    process: Process,
    peer: Peer,
    /// Kept so the private directory lives as long as the connection.
    _endpoint: PrivateEndpoint,
}

impl Child {
    pub fn peer(&self) -> &Peer {
        &self.peer
    }

    pub fn pid(&self) -> u32 {
        match &self.process {
            Process::Spawned(p) => p.pid(),
            Process::Elevated(p) => p.pid(),
        }
    }

    /// The child's log, when its output is captured (not for elevated children, whose output IMT cannot pipe).
    pub fn log(&self) -> Option<&Arc<ChildLog>> {
        match &self.process {
            Process::Spawned(p) => p.log(),
            Process::Elevated(_) => None,
        }
    }

    /// What the launcher could not enforce of the child's sandbox, as the child reported it before it ran (Linux:
    /// landlock); empty for children that lock themselves down.
    pub fn sandbox_notes(&self) -> &[crate::sandbox::Note] {
        match &self.process {
            Process::Spawned(p) => p.sandbox_notes(),
            Process::Elevated(_) => &[],
        }
    }

    /// The most memory the host's watcher saw the child use, in bytes (0 when it is not watched: no memory cap, or
    /// an elevated child, which caps itself).
    pub fn memory_peak(&self) -> u64 {
        match &self.process {
            Process::Spawned(p) => p.memory_peak(),
            Process::Elevated(_) => 0,
        }
    }

    /// How the child ended (for messages after a broken call).
    pub fn exit_text(&self) -> String {
        match &self.process {
            Process::Spawned(p) => p.exit_text(),
            Process::Elevated(p) => p.exit_text(),
        }
    }

    /// Whether the process has exited.
    pub fn exited(&self) -> bool {
        match &self.process {
            Process::Spawned(p) => p.exited(),
            Process::Elevated(p) => p.exited(),
        }
    }

    /// Closes the connection (a well-behaved child exits at once) and kills the process if it did not.
    pub fn stop(&self) {
        self.peer.close();
        match &self.process {
            Process::Spawned(p) => p.kill(),
            Process::Elevated(p) => p.kill(),
        }
    }
}

impl Connection for Child {
    fn pid(&self) -> Option<u32> {
        Some(Child::pid(self))
    }

    fn kill(&self) {
        self.stop();
    }
}

impl Drop for Child {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Who may open the endpoint of a child started without elevation: its AppContainer too when it is contained.
fn container_access(spawn: &SpawnSpec) -> Result<Access, IpcError> {
    #[cfg(windows)]
    if let crate::sandbox::Containment::Launcher { name, .. } = &spawn.sandbox.containment {
        let c = crate::contain::Container::open(name)
            .map_err(|e| IpcError::Spawn(format!("cannot prepare the sandbox of {}: {e}", spawn.label)))?;
        return Ok(Access::OwnerInContainer(c.sid_string));
    }
    #[cfg(not(windows))]
    let _ = spawn;
    Ok(Access::Owner)
}

/// Starts the child described by `spec` and accepts its connection; `handler` answers the calls the child
/// makes back (host services; [`crate::peer::NoCalls`] when there are none).
pub fn launch(spec: &ChildSpec, handler: Arc<dyn Handler>) -> Result<Child, IpcError> {
    let mode = crate::elevation::decide(&spec.elevation, crate::elevation::is_elevated())?;
    let access = match mode {
        crate::elevation::LaunchMode::RunAs => Access::OwnerAndAdmins,
        crate::elevation::LaunchMode::Plain => container_access(&spec.spawn)?,
    };
    let listener = Listener::create(&spec.spawn.endpoint_prefix, access)
        .map_err(|e| IpcError::Spawn(format!("cannot prepare a private connection for {}: {e}", spec.spawn.label)))?;
    let bootstrap = Bootstrap {
        address: listener.address().to_string(),
        token: listener.token().to_hex(),
        sandbox: spec.spawn.sandbox.clone(),
        max_frame: spec.peer.max_frame,
        role: spec.role.clone(),
    };
    let (process, pid) = match mode {
        crate::elevation::LaunchMode::Plain => {
            let p = Arc::new(ChildProcess::spawn(&spec.spawn)?);
            if let Err(e) = p.send_line(&bootstrap) {
                p.kill();
                return Err(e);
            }
            let pid = p.pid();
            (Process::Spawned(p), pid)
        }
        crate::elevation::LaunchMode::RunAs => {
            let p = crate::elevation::run_as(&spec.spawn, &bootstrap)?;
            let pid = p.pid();
            (Process::Elevated(p), pid)
        }
    };
    let exited: Box<dyn FnMut() -> Option<String> + Send> = match &process {
        Process::Spawned(p) => {
            let (p, label) = (p.clone(), spec.spawn.label.clone());
            Box::new(move || p.try_exit().map(|s| format!("{label} exited while starting ({s})")))
        }
        Process::Elevated(p) => {
            let (probe, label) = (p.probe(), spec.spawn.label.clone());
            Box::new(move || probe().map(|s| format!("{label} exited while starting ({s})")))
        }
    };
    let (window, opts) = (spec.spawn.connect_window, spec.peer.clone());
    let accepted = crate::runtime::run(async move { listener.accept(Some(pid), window, exited, handler, opts).await })
        .map_err(IpcError::Spawn)
        .and_then(|r| r);
    match accepted {
        Ok((peer, endpoint)) => Ok(Child {
            process,
            peer,
            _endpoint: endpoint,
        }),
        Err(e) => {
            match &process {
                Process::Spawned(p) => p.kill(),
                Process::Elevated(p) => p.kill(),
            }
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bootstrap_is_one_json_line() {
        let b = Bootstrap {
            address: "a".into(),
            token: "t".into(),
            sandbox: SandboxSpec::locked(),
            max_frame: 1024,
            role: String::new(),
        };
        let line = serde_json::to_string(&b).unwrap();
        assert!(!line.contains('\n'));
        assert_eq!(serde_json::from_str::<Bootstrap>(&line).unwrap(), b);
        let spec = ChildSpec::new(SpawnSpec::new("x", "the x")).sandbox(SandboxSpec::locked());
        assert_eq!(spec.peer.label, "the x");
        assert!(spec.spawn.sandbox.low_integrity);
    }

    #[test]
    fn a_missing_program_is_a_spawn_error() {
        let spec = ChildSpec::new(SpawnSpec::new("/definitely/not/here", "the missing child"));
        assert!(matches!(launch(&spec, Arc::new(crate::peer::NoCalls)), Err(IpcError::Spawn(_))));
    }
}
