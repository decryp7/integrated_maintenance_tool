//! The AI settings section: one serde struct with defaults and clamping, a versioned file that
//! tolerates corruption, and a JSON description of the fields for the Settings screen.
//! The Hugging Face access token is deliberately not a setting: it is never written to disk.

use crate::runtimes::builds::is_build_id;
use crate::web::{WebProvider, WebSearchSettings};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;

pub const SETTINGS_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuOffload {
    /// Decided from the free GPU memory at load time.
    Auto,
    /// A fixed number of layers on the GPU (0 = CPU only).
    Layers(u32),
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum GpuRepr {
    Word(String),
    Layers(u32),
}

impl From<GpuRepr> for GpuOffload {
    fn from(r: GpuRepr) -> Self {
        match r {
            GpuRepr::Layers(n) => GpuOffload::Layers(n),
            GpuRepr::Word(_) => GpuOffload::Auto,
        }
    }
}

impl From<GpuOffload> for GpuRepr {
    fn from(g: GpuOffload) -> Self {
        match g {
            GpuOffload::Auto => GpuRepr::Word("auto".into()),
            GpuOffload::Layers(n) => GpuRepr::Layers(n),
        }
    }
}

impl Serialize for GpuOffload {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        GpuRepr::from(*self).serialize(s)
    }
}

impl<'de> Deserialize<'de> for GpuOffload {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        GpuRepr::deserialize(d).map(Into::into)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AiSettings {
    pub version: u32,
    /// File name of the model in the models folder.
    pub active_model: Option<String>,
    /// Load the model in the background after startup when one is installed.
    pub autoload: bool,
    /// Unload the model after this many idle minutes (0 = keep loaded).
    pub idle_unload_minutes: u32,
    pub gpu_offload: GpuOffload,
    pub context_length: usize,
    pub max_tokens: usize,
    pub temperature: f32,
    /// CPU threads for inference (0 = automatic).
    pub threads: u32,
    /// Hugging Face address; set a mirror on corporate networks.
    pub hf_endpoint: String,
    pub download_proxy: Option<String>,
    pub max_download_gb: u32,
    /// Folder for downloaded models (default: `models` in the app data folder).
    pub models_dir: Option<String>,
    /// Web search for error codes and messages. Off by default; every search and page fetch is approved by the user.
    pub web_search: WebSearchSettings,
    /// Inference build of llama-server: `auto` (the best one that works here) or a build id such as
    /// `cpu`, `vulkan`, `cuda-13`, `metal` (see `imt-ai-worker --list-builds`).
    pub inference_build: String,
    /// A llama-server executable to use instead of the bundled builds (advanced).
    pub llama_server_path: Option<String>,
}

impl Default for AiSettings {
    fn default() -> Self {
        AiSettings {
            version: SETTINGS_VERSION,
            active_model: None,
            autoload: true,
            idle_unload_minutes: 15,
            gpu_offload: GpuOffload::Auto,
            context_length: 8192,
            max_tokens: 1024,
            temperature: 0.2,
            threads: 0,
            hf_endpoint: crate::models::hf::DEFAULT_ENDPOINT.into(),
            download_proxy: None,
            max_download_gb: 20,
            models_dir: None,
            web_search: WebSearchSettings::default(),
            inference_build: AUTO_BUILD.into(),
            llama_server_path: None,
        }
    }
}

/// The automatic choice of inference build.
pub const AUTO_BUILD: &str = "auto";

impl AiSettings {
    /// Brings every value into its allowed range (called after loading and before saving).
    pub fn clamp(mut self) -> Self {
        self.version = SETTINGS_VERSION;
        self.idle_unload_minutes = self.idle_unload_minutes.min(24 * 60);
        self.context_length = self.context_length.clamp(1024, 131_072);
        self.max_tokens = self.max_tokens.clamp(64, 16_384).min(self.context_length / 2);
        self.temperature = if self.temperature.is_finite() {
            self.temperature.clamp(0.0, 2.0)
        } else {
            0.2
        };
        self.threads = self.threads.min(256);
        self.max_download_gb = self.max_download_gb.clamp(1, 200);
        if let GpuOffload::Layers(n) = self.gpu_offload {
            self.gpu_offload = GpuOffload::Layers(n.min(1000));
        }
        let blank = |s: &Option<String>| s.as_deref().is_none_or(|v| v.trim().is_empty());
        if blank(&self.download_proxy) {
            self.download_proxy = None;
        }
        if blank(&self.models_dir) {
            self.models_dir = None;
        }
        if blank(&self.active_model) {
            self.active_model = None;
        }
        if self.hf_endpoint.trim().is_empty() {
            self.hf_endpoint = crate::models::hf::DEFAULT_ENDPOINT.into();
        }
        self.web_search = self.web_search.clone().clamp();
        self.inference_build = self.inference_build.trim().to_ascii_lowercase();
        if !is_build_id(&self.inference_build) {
            self.inference_build = AUTO_BUILD.into();
        }
        if blank(&self.llama_server_path) {
            self.llama_server_path = None;
        }
        self
    }

    /// The environment the inference worker needs to find and choose llama-server (ADR 0009):
    /// `builds_dir` holds the downloaded builds of the pinned release (`RuntimeStore::builds_dir`).
    /// Changing these takes effect when the worker restarts.
    pub fn worker_env(&self, builds_dir: &Path) -> Vec<(String, String)> {
        let mut env = vec![
            ("IMT_LLAMA_DIR".to_string(), builds_dir.to_string_lossy().into_owned()),
            ("IMT_LLAMA_BUILD".to_string(), self.inference_build.clone()),
        ];
        if let Some(p) = &self.llama_server_path {
            env.push(("IMT_LLAMA_SERVER".to_string(), p.trim().to_string()));
        }
        env
    }

    /// Problems the user should fix (clamping cannot repair them).
    pub fn validate(&self) -> Vec<String> {
        let mut p = Vec::new();
        if !self.hf_endpoint.starts_with("https://") {
            p.push("The model host address must start with https://".to_string());
        }
        if let Some(proxy) = &self.download_proxy {
            if !(proxy.starts_with("http://") || proxy.starts_with("https://")) {
                p.push("The download proxy must look like http://host:port".to_string());
            }
        }
        let w = &self.web_search;
        if w.provider == WebProvider::Searxng
            && !(w.searxng_url.starts_with("https://")
                || w.searxng_url.starts_with("http://localhost")
                || w.searxng_url.starts_with("http://127.0.0.1"))
        {
            p.push("The SearXNG address must start with https:// (http:// is allowed only for localhost)".to_string());
        }
        if let Some(m) = &self.active_model {
            if !crate::catalog::is_safe_file_name(m) {
                p.push("The active model must be a .gguf file name".to_string());
            }
        }
        if !is_build_id(&self.inference_build) {
            p.push("The inference build must be auto or a build name such as cpu, vulkan or cuda-13".to_string());
        }
        if let Some(path) = &self.llama_server_path {
            if !Path::new(path.trim()).is_absolute() {
                p.push("The llama-server path must be a full path to the executable".to_string());
            }
        }
        p
    }

    /// Loads settings; a missing file gives defaults, a damaged or newer one gives defaults plus a warning.
    pub fn load(path: &Path) -> (AiSettings, Option<String>) {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (AiSettings::default(), None),
            Err(e) => {
                return (
                    AiSettings::default(),
                    Some(format!("cannot read the AI settings ({e}); using defaults")),
                )
            }
        };
        match serde_json::from_str::<AiSettings>(&text) {
            Ok(s) if s.version <= SETTINGS_VERSION => (s.clamp(), None),
            Ok(_) => (
                AiSettings::default(),
                Some("the AI settings were saved by a newer IMT; using defaults".into()),
            ),
            Err(_) => (
                AiSettings::default(),
                Some("the AI settings file is damaged; using defaults".into()),
            ),
        }
    }

    /// Writes atomically (temp file, then rename).
    pub fn save(&self, path: &Path) -> crate::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        let text = serde_json::to_string_pretty(&self.clone().clamp()).map_err(|e| crate::AiError::Io(e.to_string()))?;
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}

/// Description of every field for the Settings screen: key, label, description, type, default,
/// min, max, options, restartRequired.
pub fn schema() -> Value {
    let d = AiSettings::default();
    let f = |key: &str, label: &str, desc: &str, ty: &str, default: Value, extra: Value, restart: bool| {
        let mut o = json!({"key": key, "label": label, "description": desc, "type": ty, "default": default, "restartRequired": restart});
        if let (Some(m), Some(e)) = (o.as_object_mut(), extra.as_object()) {
            m.extend(e.clone());
        }
        o
    };
    json!([
        f("active_model", "Model", "The model file to use. Choose or add one in the AI assistant dialog.", "string", Value::Null, json!({}), false),
        f("autoload", "Load in the background at startup", "Prepare the model after IMT starts so the first answer is quick.", "bool", json!(d.autoload), json!({}), false),
        f("idle_unload_minutes", "Unload when idle (minutes)", "Free the memory after this long without use. 0 keeps the model loaded.", "int", json!(d.idle_unload_minutes), json!({"min": 0, "max": 1440}), false),
        f("gpu_offload", "GPU use", "Automatic uses the free GPU memory; a number fixes the layers on the GPU (0 = CPU only).", "enum", json!("auto"), json!({"options": ["auto", "0", "8", "16", "24", "32", "all"], "min": 0, "max": 1000, "allowsNumber": true}), true),
        f("context_length", "Context length (tokens)", "How much text the model can consider at once. Larger needs more memory.", "int", json!(d.context_length), json!({"min": 1024, "max": 131072}), true),
        f("max_tokens", "Longest answer (tokens)", "Hard cap on the length of one answer.", "int", json!(d.max_tokens), json!({"min": 64, "max": 16384}), false),
        f("temperature", "Creativity", "0 is the most predictable. Low values suit analysis.", "float", json!(d.temperature), json!({"min": 0.0, "max": 2.0}), false),
        f("threads", "CPU threads", "0 chooses automatically.", "int", json!(d.threads), json!({"min": 0, "max": 256}), true),
        f("hf_endpoint", "Model host", "Where models are downloaded from. Use a mirror on corporate networks (https only).", "string", json!(d.hf_endpoint), json!({}), false),
        f("download_proxy", "Download proxy", "Optional http://host:port proxy, used for model downloads only.", "string", Value::Null, json!({}), false),
        f("max_download_gb", "Largest download (GB)", "Refuse model files larger than this.", "int", json!(d.max_download_gb), json!({"min": 1, "max": 200}), false),
        f("models_dir", "Models folder", "Where downloaded models are stored. Empty uses the app data folder.", "path", Value::Null, json!({}), true),
        f("inference_build", "Inference build", "Which llama-server build runs the model. Automatic picks the best one that works on this computer: CUDA with an NVIDIA GPU, else Vulkan, else the CPU (Metal on Apple Silicon). If the chosen build cannot start, IMT falls back to the next one and says so.", "enum", json!(AUTO_BUILD), json!({"options": ["auto", "cpu", "vulkan", "cuda-13", "cuda-12", "rocm", "sycl", "sycl-fp32", "openvino", "metal"], "optionsSource": "aiInferenceBuilds"}), true),
        f("llama_server_path", "Custom llama-server", "Advanced: a llama-server executable to use instead of the inference runtimes IMT downloads. Empty uses the downloaded builds.", "file", Value::Null, json!({}), true),
        f("web_search.provider", "Web search", "Look up error codes and messages on the web. Off by default. Only a cleaned-up query leaves this computer, and only after you approve it. Manual shows the query and lets you search yourself.", "enum", json!("none"), json!({"options": ["none", "searxng", "brave", "manual"]}), false),
        f("web_search.searxng_url", "SearXNG address", "Your SearXNG instance, for example https://search.example.org. It must allow the JSON format.", "string", json!(""), json!({}), false),
        f("web_search.brave_key", "Brave Search API key", "Stored in the system keychain, never in a file. Write-only: it cannot be read back.", "secret", Value::Null, json!({"writeOnly": true, "storage": "keychain"}), false),
        f("web_search.max_results", "Results per search", "How many results the assistant sees.", "int", json!(5), json!({"min": 1, "max": 10}), false),
        f("web_search.allow_fetch", "Allow reading result pages", "Lets the assistant read a page that a search returned, after you approve that page.", "bool", json!(true), json!({}), false),
        f("web_search.require_approval", "Ask before every search and page", "Always on: every web search and page read needs your approval.", "bool", json!(true), json!({"readOnly": true}), false),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_roundtrip_and_gpu_offload_forms() {
        let s = AiSettings::default();
        let text = serde_json::to_string(&s).unwrap();
        assert!(text.contains("\"gpu_offload\":\"auto\""));
        assert_eq!(serde_json::from_str::<AiSettings>(&text).unwrap(), s);
        let n: AiSettings = serde_json::from_str("{\"gpu_offload\": 12}").unwrap();
        assert_eq!(n.gpu_offload, GpuOffload::Layers(12));
        assert!(serde_json::to_string(&n).unwrap().contains("\"gpu_offload\":12"));
        let w: AiSettings = serde_json::from_str("{\"gpu_offload\": \"whatever\"}").unwrap();
        assert_eq!(w.gpu_offload, GpuOffload::Auto);
        // missing keys take defaults, unknown keys are ignored
        let p: AiSettings = serde_json::from_str("{\"autoload\": false, \"future\": 1}").unwrap();
        assert!(!p.autoload && p.context_length == 8192);
    }

    #[test]
    fn clamping() {
        let s = AiSettings {
            context_length: 10,
            max_tokens: 99_999,
            temperature: f32::NAN,
            threads: 9999,
            idle_unload_minutes: 99999,
            max_download_gb: 0,
            gpu_offload: GpuOffload::Layers(5000),
            download_proxy: Some("  ".into()),
            models_dir: Some("".into()),
            active_model: Some(" ".into()),
            hf_endpoint: " ".into(),
            inference_build: "../x".into(),
            llama_server_path: Some(" ".into()),
            ..AiSettings::default()
        }
        .clamp();
        assert_eq!(s.inference_build, "auto");
        assert!(s.llama_server_path.is_none());
        assert_eq!(
            (s.context_length, s.max_tokens, s.threads, s.idle_unload_minutes, s.max_download_gb),
            (1024, 512, 256, 1440, 1)
        );
        assert_eq!(s.temperature, 0.2);
        assert_eq!(s.gpu_offload, GpuOffload::Layers(1000));
        assert!(s.download_proxy.is_none() && s.models_dir.is_none() && s.active_model.is_none());
        assert_eq!(s.hf_endpoint, "https://huggingface.co");
        assert_eq!(
            AiSettings {
                temperature: 9.0,
                ..AiSettings::default()
            }
            .clamp()
            .temperature,
            2.0
        );
    }

    #[test]
    fn inference_build_settings_reach_the_worker() {
        let d = AiSettings::default();
        assert_eq!(d.inference_build, "auto");
        let dir = Path::new("/data/ai/runtimes/b1");
        assert_eq!(
            d.worker_env(dir),
            vec![
                ("IMT_LLAMA_DIR".to_string(), dir.to_string_lossy().into_owned()),
                ("IMT_LLAMA_BUILD".to_string(), "auto".to_string())
            ]
        );
        let server = if cfg!(windows) {
            r"C:\llama\llama-server.exe"
        } else {
            "/opt/llama/llama-server"
        };
        let s = AiSettings {
            inference_build: " Vulkan ".into(),
            llama_server_path: Some(server.into()),
            ..AiSettings::default()
        }
        .clamp();
        assert_eq!(s.inference_build, "vulkan");
        assert!(s.validate().is_empty());
        let env = s.worker_env(dir);
        assert_eq!(env[1], ("IMT_LLAMA_BUILD".to_string(), "vulkan".to_string()));
        assert_eq!(env[2], ("IMT_LLAMA_SERVER".to_string(), server.to_string()));
        let bad = AiSettings {
            inference_build: "Not Valid!".into(),
            llama_server_path: Some("relative/llama-server".into()),
            ..AiSettings::default()
        };
        assert_eq!(bad.validate().len(), 2);
        // older settings files without the keys load with the defaults
        let old: AiSettings = serde_json::from_str("{\"autoload\": true}").unwrap();
        assert_eq!((old.inference_build.as_str(), old.llama_server_path), ("auto", None));
    }

    #[test]
    fn validation() {
        assert!(AiSettings::default().validate().is_empty());
        let bad = AiSettings {
            hf_endpoint: "http://x".into(),
            download_proxy: Some("socks5://x".into()),
            active_model: Some("../a.gguf".into()),
            ..AiSettings::default()
        };
        assert_eq!(bad.validate().len(), 3);
        let w = |provider, url: &str| AiSettings {
            web_search: WebSearchSettings {
                provider,
                searxng_url: url.into(),
                ..Default::default()
            },
            ..AiSettings::default()
        };
        assert_eq!(w(WebProvider::Searxng, "http://example.com").validate().len(), 1);
        assert!(w(WebProvider::Searxng, "https://s.example.org").validate().is_empty());
        assert!(w(WebProvider::Searxng, "http://localhost:8080").validate().is_empty());
        assert!(w(WebProvider::None, "").validate().is_empty());
        assert_eq!(AiSettings::default().web_search.provider, WebProvider::None);
        assert_eq!(
            AiSettings {
                web_search: WebSearchSettings {
                    max_results: 99,
                    ..Default::default()
                },
                ..AiSettings::default()
            }
            .clamp()
            .web_search
            .max_results,
            10
        );
        assert!(AiSettings {
            download_proxy: Some("http://p:1".into()),
            ..AiSettings::default()
        }
        .validate()
        .is_empty());
    }

    #[test]
    fn load_and_save_tolerate_problems() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("sub/ai.json");
        assert_eq!(AiSettings::load(&p), (AiSettings::default(), None));
        let s = AiSettings {
            active_model: Some("a.gguf".into()),
            temperature: 0.7,
            ..AiSettings::default()
        };
        s.save(&p).unwrap();
        assert_eq!(AiSettings::load(&p), (s, None));
        std::fs::write(&p, "{ broken").unwrap();
        assert!(AiSettings::load(&p).1.unwrap().contains("damaged"));
        std::fs::write(&p, "{\"version\": 99}").unwrap();
        assert!(AiSettings::load(&p).1.unwrap().contains("newer"));
        assert!(AiSettings::load(dir.path()).1.unwrap().contains("cannot read"));
    }

    #[test]
    fn schema_describes_every_field() {
        let v = schema();
        let keys: Vec<&str> = v.as_array().unwrap().iter().map(|f| f["key"].as_str().unwrap()).collect();
        let settings = serde_json::to_value(AiSettings::default()).unwrap();
        for k in settings
            .as_object()
            .unwrap()
            .keys()
            .filter(|k| *k != "version" && *k != "web_search")
        {
            assert!(keys.contains(&k.as_str()), "{k} missing from the schema");
        }
        for k in settings["web_search"].as_object().unwrap().keys() {
            assert!(
                keys.contains(&format!("web_search.{k}").as_str()),
                "web_search.{k} missing from the schema"
            );
        }
        let brave = v.as_array().unwrap().iter().find(|f| f["key"] == "web_search.brave_key").unwrap();
        assert_eq!((brave["type"].as_str(), brave["writeOnly"].as_bool()), (Some("secret"), Some(true)));
        assert!(
            !settings.to_string().contains("brave_key"),
            "the key never lives in the settings file"
        );
        for f in v.as_array().unwrap() {
            for k in ["key", "label", "description", "type", "default", "restartRequired"] {
                assert!(f.get(k).is_some(), "{f}");
            }
        }
    }
}
