//! Inference runs in a separate worker process so a crash, hang or out-of-memory in native code cannot take
//! IMT down. The wire is IMT's one IPC (ADR 0017, `imt_ipc`): postcard frames on a private local stream (a Unix
//! socket in a 0700 directory or an owner-only named pipe), IMT listening and the worker connecting with a
//! random per-launch token. `WorkerTransport` is the seam the supervisor depends on.

pub mod backend;
pub mod frames;
pub mod protocol;
pub mod supervisor;
pub mod transport;

pub use backend::{current_rss_bytes, WorkerBackend};
pub use frames::{
    default_worker_path, fixed_env, run_worker, worker_file_name, CappedLauncher, FramesLauncher, FramesTransport, WorkerEnv,
    WorkerMemoryCap,
};
pub use supervisor::{EngineState, EngineStatus, Health, Supervisor, WorkerConfig, WorkerEngine, WorkerRuntime};
pub use transport::{CallOpts, TransportError, WorkerLauncher, WorkerTransport};

/// The only variables of IMT's environment the worker inherits (it starts from an empty environment plus
/// these and its `IMT_LLAMA_*` settings): what the OS, the GPU drivers and their loaders need for
/// llama-server, and the coverage profile path for instrumented test builds.
pub const WORKER_ENV: &[&str] = &[
    "PATH",
    "SystemRoot",
    "SYSTEMROOT",
    "windir",
    "SystemDrive",
    "TEMP",
    "TMP",
    "TMPDIR",
    "HOME",
    "USERPROFILE",
    "LOCALAPPDATA",
    "APPDATA",
    "ProgramData",
    "ProgramFiles",
    "LANG",
    "LC_ALL",
    "XDG_RUNTIME_DIR",
    "XDG_CACHE_HOME",
    "XDG_DATA_DIRS",
    "XDG_CONFIG_DIRS",
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "LD_LIBRARY_PATH",
    "VK_ICD_FILENAMES",
    "VK_DRIVER_FILES",
    "CUDA_VISIBLE_DEVICES",
    "HIP_VISIBLE_DEVICES",
    "ROCR_VISIBLE_DEVICES",
    "LLVM_PROFILE_FILE",
    "RUST_BACKTRACE",
    "CARGO_LLVM_COV",
];

#[cfg(test)]
mod tests {
    #[test]
    fn the_worker_keeps_what_every_child_keeps_and_nothing_secret() {
        for k in imt_ipc::process::KEPT_ENV {
            assert!(super::WORKER_ENV.contains(k), "{k}");
        }
        for secret in ["HF_TOKEN", "HTTPS_PROXY", "IMT_DATA_DIR", "USERNAME"] {
            assert!(!super::WORKER_ENV.contains(&secret), "{secret}");
        }
    }
}
