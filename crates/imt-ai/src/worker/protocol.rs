//! The typed vocabulary of the worker protocol. The wire is gRPC (`proto/llm.proto`, see `grpc_client` and
//! `grpc_server`); these are the plain-Rust forms the supervisor and the backends work with, and the
//! typed error that travels in the `x-imt-error-bin` trailer.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const PROTOCOL_VERSION: u32 = 1;
/// Largest gRPC message accepted in either direction.
pub const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;
/// Metadata key carrying the per-launch token on every call.
pub const TOKEN_HEADER: &str = imt_ipc::auth::TOKEN_HEADER;
/// Binary trailer carrying the typed error (JSON).
pub const ERROR_TRAILER: &str = imt_ipc::error::ERROR_TRAILER;

/// The calls of the `Worker` service, by the names the transport trait uses.
pub mod method {
    pub const HELLO: &str = "hello";
    pub const LOAD: &str = "load";
    pub const CHAT: &str = "chat";
    pub const UNLOAD: &str = "unload";
    pub const STATUS: &str = "status";
    pub const SHUTDOWN: &str = "shutdown";
}

/// Typed error codes (the numbers are kept from the JSON-RPC convention for readability).
pub mod code {
    pub const INVALID_REQUEST: i32 = -32600;
    pub const METHOD_NOT_FOUND: i32 = -32601;
    pub const INVALID_PARAMS: i32 = -32602;
    pub const ENGINE: i32 = -32000;
    pub const LOAD_FAILED: i32 = -32001;
    pub const CONTEXT_OVERFLOW: i32 = -32002;
    pub const NOT_LOADED: i32 = -32003;
    pub const BUSY: i32 = -32004;
}

/// A typed error of the worker (`code`, `message`, optional `data`); the shared `imt_ipc::RemoteError`.
pub type RpcError = imt_ipc::RemoteError;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HelloParams {
    pub protocol_version: u32,
    #[serde(default)]
    pub client: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HelloResult {
    pub protocol_version: u32,
    pub worker_version: String,
    pub pid: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoadParams {
    pub model_path: String,
    /// -1 decide from the free GPU memory, 0 CPU only, n layers on the GPU.
    pub gpu_layers: i32,
    pub context: usize,
    /// 0 = automatic.
    pub threads: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoadResult {
    /// "Metal", "Vulkan", "CUDA", "CPU".
    pub backend: String,
    pub context: usize,
    pub name: String,
    pub rss_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatParams {
    /// The prompt, already rendered for the model's family by IMT.
    pub prompt: String,
    pub max_tokens: usize,
    pub temperature: f32,
    #[serde(default)]
    pub json_schema: Option<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireFinish {
    Stop,
    Length,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatResult {
    pub text: String,
    pub prompt_tokens: usize,
    pub completion_tokens: usize,
    pub finish: WireFinish,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatusResult {
    pub loaded: bool,
    pub backend: String,
    pub rss_bytes: u64,
    pub pid: u32,
    pub uptime_secs: u64,
}

/// `data.reason` of a `LOAD_FAILED` error when no llama-server build is installed (the worker's `locate::NO_RUNTIME`).
pub const NO_RUNTIME_REASON: &str = "no_runtime";

/// The `data.reason` of a typed error, if it carries one.
pub fn reason_of(e: &RpcError) -> Option<&str> {
    e.data.as_ref()?.get("reason")?.as_str()
}

pub fn context_overflow(prompt: usize, limit: usize) -> RpcError {
    RpcError {
        code: code::CONTEXT_OVERFLOW,
        message: format!("the prompt ({prompt} tokens) does not fit the context ({limit})"),
        data: Some(json!({"prompt": prompt, "limit": limit})),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_params_roundtrip() {
        let l = LoadParams {
            model_path: "/m.gguf".into(),
            gpu_layers: -1,
            context: 8192,
            threads: 0,
        };
        assert_eq!(serde_json::from_value::<LoadParams>(serde_json::to_value(&l).unwrap()).unwrap(), l);
        let c: ChatParams = serde_json::from_value(json!({"prompt": "p", "max_tokens": 5, "temperature": 0.5})).unwrap();
        assert!(c.json_schema.is_none());
        assert_eq!(serde_json::to_value(WireFinish::Cancelled).unwrap(), "cancelled");
        let o = context_overflow(10, 5);
        assert_eq!(o.code, code::CONTEXT_OVERFLOW);
        assert_eq!(o.data.unwrap()["limit"], 5);
        let e = RpcError::new(code::ENGINE, "x");
        assert_eq!(serde_json::from_str::<RpcError>(&serde_json::to_string(&e).unwrap()).unwrap(), e);
    }
}
