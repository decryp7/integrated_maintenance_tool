//! Starting a plugin process and keeping a handshaken connection to it.

use imt_ipc::child_log::{ChildKind, ChildLogSpec};
use imt_ipc::process::{launch, Launched, SpawnSpec};
use imt_ipc::supervisor::{Connection, SupervisorConfig};
use imt_ipc::IpcError;
use imt_plugin_sdk::client::{self, Client};
use imt_plugin_sdk::manifest::RemoteManifest;
use imt_plugin_sdk::PermissionRequest;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

/// The runtime every plugin connection of this process uses (two I/O threads; plugin calls themselves
/// block the calling thread, see [`client::block_on`]).
pub fn runtime() -> Result<&'static tokio::runtime::Runtime, IpcError> {
    static RT: OnceLock<Result<tokio::runtime::Runtime, String>> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("imt-plugin-io")
            .enable_all()
            .build()
            .map_err(|e| e.to_string())
    })
    .as_ref()
    .map_err(|e| IpcError::Spawn(format!("cannot start the plugin runtime: {e}")))
}

/// How to start one plugin process.
#[derive(Clone)]
pub struct PluginSpec {
    pub exe: PathBuf,
    pub args: Vec<String>,
    /// Names the plugin in messages before its manifest is known.
    pub label: String,
    /// The plugin id the process must report (a built-in served by the host binary).
    pub expected_id: Option<String>,
    /// Permissions the user granted; offered in the handshake.
    pub grants: Vec<PermissionRequest>,
    pub supervisor: SupervisorConfig,
    pub connect_window: Duration,
    pub handshake_timeout: Duration,
    /// Where the process's standard error and start/exit lines go (see `imt_ipc::child_log`).
    pub log: Option<ChildLogSpec>,
}

impl PluginSpec {
    pub fn new(exe: impl Into<PathBuf>, label: impl Into<String>) -> PluginSpec {
        let label = label.into();
        let log = ChildLogSpec::new(
            format!("plugin-{}", label.strip_prefix("plugin ").unwrap_or(&label)),
            ChildKind::Plugin,
        );
        PluginSpec {
            exe: exe.into(),
            args: Vec::new(),
            supervisor: SupervisorConfig {
                log: Some(log.clone()),
                ..SupervisorConfig::new(label.clone())
            },
            log: Some(log),
            label,
            expected_id: None,
            grants: Vec::new(),
            connect_window: Duration::from_secs(15),
            handshake_timeout: Duration::from_secs(15),
        }
    }
}

/// A live, handshaken plugin process.
pub struct PluginConnection {
    launched: Launched,
    pub client: Client,
    pub manifest: RemoteManifest,
}

impl PluginConnection {
    /// Starts the process (empty environment, temporary working directory, below-normal priority, killed
    /// with IMT) and performs the handshake.
    pub fn launch(spec: &PluginSpec) -> Result<PluginConnection, IpcError> {
        let rt = runtime()?;
        let mut s = SpawnSpec::new(spec.exe.clone(), spec.label.clone());
        s.args = spec.args.clone();
        s.clear_env = true;
        s.current_dir = Some(std::env::temp_dir());
        s.endpoint_prefix = "imt-plugin".into();
        s.connect_window = spec.connect_window;
        s.log = spec.log.clone();
        let launched = client::block_on(rt.handle(), launch(&s))?;
        let mut c = client::client(launched.channel.clone());
        let hs = client::block_on(rt.handle(), client::handshake(&mut c, &spec.grants, Some(spec.handshake_timeout)));
        let manifest = match hs {
            Ok(m) => m,
            Err(e) => {
                launched.process.kill();
                return Err(match e {
                    IpcError::Died(_) => IpcError::Died(launched.process.exit_text()),
                    other => other,
                });
            }
        };
        if let Some(log) = launched.process.log() {
            log.set_version(&manifest.version);
        }
        if let Some(id) = &spec.expected_id {
            if &manifest.id != id {
                launched.process.kill();
                return Err(IpcError::Protocol(format!(
                    "expected plugin \"{id}\", the process is \"{}\"",
                    manifest.id
                )));
            }
        }
        Ok(PluginConnection {
            launched,
            client: c,
            manifest,
        })
    }

    /// Describes how the process ended (for messages after a broken call).
    pub fn exit_text(&self) -> String {
        self.launched.process.exit_text()
    }
}

impl Connection for PluginConnection {
    fn pid(&self) -> Option<u32> {
        Some(self.launched.process.pid())
    }

    fn kill(&self) {
        self.launched.process.kill();
    }
}
