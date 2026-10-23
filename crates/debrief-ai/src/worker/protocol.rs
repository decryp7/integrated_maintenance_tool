//! The typed vocabulary of the worker protocol: the requests and replies that cross the wire (`frames`), as the
//! supervisor and the backends work with them, and the worker's typed error.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// 2: the model's image projector (`LoadParams::mmproj_path`, `LoadResult::vision`) and images in chats
/// (`ChatParams::images`, ADR 0025).
pub const PROTOCOL_VERSION: u32 = 2;
/// Largest frame accepted in either direction.
pub const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;

/// The calls of the worker, by the names the transport trait uses.
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

/// A typed error of the worker (`code`, `message`, optional `data`); the shared `debrief_ipc::RemoteError`.
pub type RpcError = debrief_ipc::RemoteError;

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
    /// The model's multimodal projector, loaded into the same server (`--mmproj`) so the one model reads images.
    #[serde(default)]
    pub mmproj_path: Option<String>,
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
    /// The server loaded the projector and accepts images.
    #[serde(default)]
    pub vision: bool,
    /// What the running llama-server reported about itself (`/props`), when it answered.
    #[serde(default)]
    pub server: Option<ServerInfo>,
}

/// What a running llama-server reports about itself (`GET /props`), read once after it starts. Every field is
/// optional: builds differ, and the worker never assumes what a build does not say (ADR 0009).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerInfo {
    /// The context the server actually allocated.
    #[serde(default)]
    pub n_ctx: Option<usize>,
    /// `modalities.vision`: the server reads images (its projector loaded).
    #[serde(default)]
    pub vision: Option<bool>,
    /// `modalities.audio`.
    #[serde(default)]
    pub audio: Option<bool>,
    /// The marker the server looks for in a multimodal prompt; `None` means llama.cpp's default `<__media__>`.
    #[serde(default)]
    pub media_marker: Option<String>,
    /// The model the server says it loaded (alias or file name, never a full path).
    #[serde(default)]
    pub model: Option<String>,
    /// The server's build (`build_info`, e.g. "b6800-abc1234").
    #[serde(default)]
    pub build: Option<String>,
    /// The server has a chat template for the model.
    #[serde(default)]
    pub chat_template: bool,
}

/// Most images in one chat request, and most bytes of their (base64) data together.
pub const MAX_IMAGES: usize = 4;
pub const MAX_IMAGE_DATA: usize = 8 * 1024 * 1024;

/// An image of a chat request: Debrief's sanitised re-encoding (never the user's file), base64.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WireImage {
    /// `image/png` or `image/jpeg`.
    pub mime: String,
    /// Standard base64 of the encoded image.
    pub data: String,
    pub width: u32,
    pub height: u32,
}

impl WireImage {
    /// Checks a request's images against the limits (the worker re-checks what it is sent).
    pub fn check_all(images: &[WireImage]) -> Result<(), RpcError> {
        let bytes: usize = images.iter().map(|i| i.data.len()).sum();
        let bad = images.iter().any(|i| {
            !matches!(i.mime.as_str(), "image/png" | "image/jpeg")
                || i.width == 0
                || i.height == 0
                || i.data.is_empty()
                || !i.data.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='))
        });
        if images.len() > MAX_IMAGES || bytes > MAX_IMAGE_DATA || bad {
            return Err(RpcError::new(
                code::INVALID_PARAMS,
                format!(
                    "at most {MAX_IMAGES} PNG or JPEG images of {} MB together",
                    MAX_IMAGE_DATA / (1024 * 1024)
                ),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatParams {
    /// The prompt, already rendered for the model's family by Debrief.
    pub prompt: String,
    pub max_tokens: usize,
    pub temperature: f32,
    #[serde(default)]
    pub json_schema: Option<Value>,
    /// The images of the prompt, in the order of its media markers (`llm::prompted::MEDIA_MARKER`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<WireImage>,
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
        data: Some(json!({"prompt": prompt, "limit": limit}).into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_params_roundtrip() {
        let l = LoadParams {
            model_path: "/m.gguf".into(),
            mmproj_path: Some("/m.mmproj.gguf".into()),
            gpu_layers: -1,
            context: 8192,
            threads: 0,
        };
        assert_eq!(serde_json::from_value::<LoadParams>(serde_json::to_value(&l).unwrap()).unwrap(), l);
        let c: ChatParams = serde_json::from_value(json!({"prompt": "p", "max_tokens": 5, "temperature": 0.5})).unwrap();
        assert!(c.json_schema.is_none() && c.images.is_empty());
        let old: LoadParams = serde_json::from_value(json!({"model_path": "m", "gpu_layers": 0, "context": 1, "threads": 0})).unwrap();
        assert_eq!(old.mmproj_path, None);
        // A worker that does not report the server (older protocol) still parses.
        let l: LoadResult =
            serde_json::from_value(json!({"backend": "CPU", "context": 4096, "name": "m", "rss_bytes": 1, "vision": true})).unwrap();
        assert_eq!(l.server, None);
        let partial: ServerInfo = serde_json::from_value(json!({"n_ctx": 2048})).unwrap();
        assert_eq!((partial.n_ctx, partial.vision, partial.chat_template), (Some(2048), None, false));
        let img = |mime: &str, data: &str| WireImage {
            mime: mime.into(),
            data: data.into(),
            width: 2,
            height: 2,
        };
        assert!(WireImage::check_all(&[img("image/png", "iVBORw0KGgo=")]).is_ok());
        for bad in [
            vec![img("image/gif", "AAAA")],
            vec![img("image/png", "")],
            vec![img("image/png", "not base64!")],
            vec![img("image/png", "AAAA"); MAX_IMAGES + 1],
            vec![img("image/png", &"A".repeat(MAX_IMAGE_DATA + 4))],
            vec![WireImage {
                width: 0,
                ..img("image/png", "AAAA")
            }],
        ] {
            assert_eq!(WireImage::check_all(&bad).unwrap_err().code, code::INVALID_PARAMS);
        }
        assert_eq!(serde_json::to_value(WireFinish::Cancelled).unwrap(), "cancelled");
        let o = context_overflow(10, 5);
        assert_eq!(o.code, code::CONTEXT_OVERFLOW);
        assert_eq!(o.data.unwrap()["limit"], 5);
        let e = RpcError::new(code::ENGINE, "x");
        assert_eq!(serde_json::from_str::<RpcError>(&serde_json::to_string(&e).unwrap()).unwrap(), e);
    }
}
