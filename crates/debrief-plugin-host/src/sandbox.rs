//! The sandbox of a plugin's process: the policy lives in the SDK ([`debrief_plugin_api::sandbox`]), so the test
//! harness launches plugins exactly as Debrief does.

pub use debrief_plugin_api::sandbox::{container_name, sandbox_for, sandbox_for_third_party, PluginHome, DEFAULT_MEMORY_LIMIT};
