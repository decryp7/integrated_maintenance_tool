//! SDK for IMT process plugins: the `imt.plugin.v1` contract (ADR 0004).
//!
//! A process plugin is an ordinary IMT [`Plugin`](imt_core::Plugin) (importers, transforms, exporters,
//! marker kinds, annotators, settings) compiled into its own executable whose `main` calls [`serve`]:
//!
//! ```no_run
//! # struct MyPlugin;
//! # impl imt_core::Plugin for MyPlugin {
//! #     fn id(&self) -> &'static str { "com.example.my" }
//! #     fn name(&self) -> &'static str { "My plugin" }
//! #     fn version(&self) -> &'static str { "1.0.0" }
//! #     fn register(&self, _: &mut imt_core::PluginRegistry) -> anyhow::Result<()> { Ok(()) }
//! # }
//! fn main() {
//!     if let Err(e) = imt_plugin_sdk::serve(std::sync::Arc::new(MyPlugin)) {
//!         eprintln!("{e}");
//!         std::process::exit(1);
//!     }
//! }
//! ```
//!
//! IMT starts the executable, tells it a private address and a one-time token on stdin, and calls it over
//! gRPC; records travel as Arrow record batches ([`arrow`]). The plugin process exits when IMT closes its
//! stdin. `imt-plugin-check <executable>` ([`conformance`]) runs the contract tests a plugin must pass.
//!
//! Modules: [`pb`] (generated code), [`arrow`] (the record schema), [`convert`] (messages to IMT types),
//! [`manifest`] (the parsed handshake reply), [`server`] (serving a plugin), [`client`] (calling one; used
//! by IMT's plugin host and the conformance kit), [`conformance`].

pub mod arrow;
pub mod client;
pub mod conformance;
pub mod convert;
pub mod manifest;
pub mod server;

/// Generated code of the `imt.plugin.v1` package (`proto/imt/plugin/v1/plugin.proto`).
#[allow(clippy::all, missing_docs)]
pub mod pb {
    tonic::include_proto!("imt.plugin.v1");
}

pub use convert::{Permission, PermissionRequest};
pub use server::{serve, serve_with, ByteService, ServeOptions};

/// The contract version this SDK implements.
pub const CONTRACT_MAJOR: u32 = 1;
pub const CONTRACT_MINOR: u32 = 0;

/// Largest gRPC message in either direction (a record batch is at most about 24 MiB of text).
pub const MAX_MESSAGE_BYTES: usize = 64 * 1024 * 1024;
/// File content is streamed in chunks of this size.
pub const CHUNK_BYTES: usize = 1024 * 1024;
/// Largest file content one `Import` call accepts.
pub const MAX_IMPORT_BYTES: u64 = 4 * 1024 * 1024 * 1024;

/// Codes of the typed errors (`x-imt-error-bin`) of the contract.
pub mod code {
    /// The plugin's code returned an error (the plugin itself is fine).
    pub const FAILED: i32 = -32000;
    /// The request is malformed (unknown id, bad JSON, wrong order of messages).
    pub const INVALID: i32 = -32602;
    /// The plugin does not implement this call.
    pub const UNIMPLEMENTED: i32 = -32601;
    /// The plugin's code panicked while handling the call; IMT counts it like an in-process crash.
    pub const CRASHED: i32 = -32010;
    /// The contract versions are incompatible.
    pub const VERSION: i32 = -32030;
    /// Call before `Handshake`.
    pub const NO_HANDSHAKE: i32 = -32031;
    /// A limit was exceeded (content too large, too many rows).
    pub const TOO_LARGE: i32 = -32040;
}
