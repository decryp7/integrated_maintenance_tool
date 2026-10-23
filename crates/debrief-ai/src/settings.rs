//! The AI settings section: one serde struct with defaults and clamping, a versioned file that
//! tolerates corruption, and a JSON description of the fields for the Settings screen.
//! The Hugging Face access token is deliberately not a setting: it is never written to disk.

use crate::runtimes::builds::is_build_id;
use crate::updates::UpdatePolicy;
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
    /// Parallel connections for large downloads (1 turns segmented downloads off, at most 8).
    pub download_connections: u32,
    /// Folder for downloaded models (default: `models` in the app data folder).
    pub models_dir: Option<String>,
    /// Inference build of llama-server: `auto` (the best one that works here) or a build id such as
    /// `cpu`, `vulkan`, `cuda-13`, `metal` (see `debrief-ai-worker --list-builds`).
    pub inference_build: String,
    /// A llama-server executable to use instead of the bundled builds (advanced).
    pub llama_server_path: Option<String>,
    /// What to do when a newer pinned inference runtime is available.
    pub runtime_updates: UpdatePolicy,
    /// What to do when a newer revision of an installed catalog model is available.
    pub model_updates: UpdatePolicy,
    /// Hours between automatic update checks.
    pub update_check_hours: u32,
}

/// The default hours between update checks (daily).
pub const DEFAULT_UPDATE_CHECK_HOURS: u32 = 24;

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
            download_connections: 4,
            models_dir: None,
            inference_build: AUTO_BUILD.into(),
            llama_server_path: None,
            runtime_updates: UpdatePolicy::Ask,
            model_updates: UpdatePolicy::Ask,
            update_check_hours: DEFAULT_UPDATE_CHECK_HOURS,
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
        self.update_check_hours = self.update_check_hours.clamp(1, 24 * 30);
        self.max_download_gb = self.max_download_gb.clamp(1, 200);
        if let GpuOffload::Layers(n) = self.gpu_offload {
            self.gpu_offload = GpuOffload::Layers(n.min(1000));
        }
        let blank = |s: &Option<String>| s.as_deref().is_none_or(|v| v.trim().is_empty());
        self.download_connections = self.download_connections.clamp(1, 8);
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
            ("DEBRIEF_LLAMA_DIR".to_string(), builds_dir.to_string_lossy().into_owned()),
            ("DEBRIEF_LLAMA_BUILD".to_string(), self.inference_build.clone()),
        ];
        if let Some(p) = &self.llama_server_path {
            env.push(("DEBRIEF_LLAMA_SERVER".to_string(), p.trim().to_string()));
        }
        env
    }

    /// Seconds between automatic update checks.
    pub fn update_interval_secs(&self) -> u64 {
        u64::from(self.update_check_hours.clamp(1, 24 * 30)) * 3600
    }

    /// The update policy for `kind`.
    pub fn update_policy(&self, kind: crate::updates::UpdateKind) -> UpdatePolicy {
        match kind {
            crate::updates::UpdateKind::Runtime => self.runtime_updates,
            crate::updates::UpdateKind::Model => self.model_updates,
        }
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
                Some("the AI settings were saved by a newer Debrief; using defaults".into()),
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
        f("autoload", "Load in the background at startup", "Prepare the model after Debrief starts so the first answer is quick.", "bool", json!(d.autoload), json!({}), false),
        f("idle_unload_minutes", "Unload when idle (minutes)", "Free the memory after this long without use. 0 keeps the model loaded.", "int", json!(d.idle_unload_minutes), json!({"min": 0, "max": 1440}), false),
        f("gpu_offload", "GPU use", "Automatic uses the free GPU memory; a number fixes the layers on the GPU (0 = CPU only).", "enum", json!("auto"), json!({"options": ["auto", "0", "8", "16", "24", "32", "all"], "min": 0, "max": 1000, "allowsNumber": true}), true),
        f("context_length", "Context length (tokens)", "How much text the model can consider at once. Larger needs more memory.", "int", json!(d.context_length), json!({"min": 1024, "max": 131072}), true),
        f("max_tokens", "Longest answer (tokens)", "Hard cap on the length of one answer.", "int", json!(d.max_tokens), json!({"min": 64, "max": 16384}), false),
        f("temperature", "Creativity", "0 is the most predictable. Low values suit analysis.", "float", json!(d.temperature), json!({"min": 0.0, "max": 2.0}), false),
        f("threads", "CPU threads", "0 chooses automatically.", "int", json!(d.threads), json!({"min": 0, "max": 256}), true),
        f("hf_endpoint", "Model host", "Where models are downloaded from. Use a mirror on corporate networks (https only).", "string", json!(d.hf_endpoint), json!({}), false),
        f("download_proxy", "Download proxy", "Optional http://host:port proxy, used for model downloads only.", "string", Value::Null, json!({}), false),
        f("max_download_gb", "Largest download (GB)", "Refuse model files larger than this.", "int", json!(d.max_download_gb), json!({"min": 1, "max": 200}), false),
        f("download_connections", "Download connections", "Large downloads (models, runtimes, updates) use this many parallel connections when the host allows it. 1 turns it off; 8 is the most. Fewer are used automatically if the host asks to slow down.", "int", json!(d.download_connections), json!({"min": 1, "max": 8}), false),
        f("models_dir", "Models folder", "Where downloaded models are stored. Empty uses the app data folder.", "path", Value::Null, json!({}), true),
        f("inference_build", "Inference build", "Which llama-server build runs the model. Automatic picks the best one that works on this computer: CUDA with an NVIDIA GPU, else Vulkan, else the CPU (Metal on Apple Silicon). If the chosen build cannot start, Debrief falls back to the next one and says so.", "enum", json!(AUTO_BUILD), json!({"options": ["auto", "cpu", "vulkan", "cuda-13", "cuda-12", "rocm", "sycl", "sycl-fp32", "openvino", "metal"], "optionsSource": "aiInferenceBuilds"}), true),
        f("runtime_updates", "Inference runtime updates", "When Debrief pins a newer llama.cpp build than the one installed. Automatic downloads and verifies it in the background (the installed build keeps working until the new one is verified, and a running chat is never interrupted); Ask tells you and offers Update, Later and Skip this version; Manual only looks when you choose Check now.", "enum", json!(d.runtime_updates.as_str()), json!({"options": ["automatic", "ask", "manual"]}), false),
        f("model_updates", "Model updates", "When the model list has a newer revision or quantisation of an installed model. Automatic downloads and verifies it in the background and switches to it only when no chat is running; Ask tells you; Manual only looks when you choose Check now.", "enum", json!(d.model_updates.as_str()), json!({"options": ["automatic", "ask", "manual"]}), false),
        f("update_check_hours", "Check for AI updates every (hours)", "How often Debrief checks for runtime and model updates (not with Manual). A check that fails because the computer is offline or a proxy blocks it is retried later without interrupting you.", "int", json!(d.update_check_hours), json!({"min": 1, "max": 720}), false),
        f("llama_server_path", "Custom llama-server", "Advanced: a llama-server executable to use instead of the inference runtimes Debrief downloads. Empty uses the downloaded builds.", "file", Value::Null, json!({}), true),
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
            download_connections: 99,
            gpu_offload: GpuOffload::Layers(5000),
            download_proxy: Some("  ".into()),
            models_dir: Some("".into()),
            active_model: Some(" ".into()),
            hf_endpoint: " ".into(),
            inference_build: "../x".into(),
            llama_server_path: Some(" ".into()),
            update_check_hours: 0,
            ..AiSettings::default()
        }
        .clamp();
        assert_eq!(s.inference_build, "auto");
        assert!(s.llama_server_path.is_none());
        assert_eq!(s.update_check_hours, 1, "0 hours becomes 1");
        assert_eq!(
            (s.context_length, s.max_tokens, s.threads, s.idle_unload_minutes, s.max_download_gb),
            (1024, 512, 256, 1440, 1)
        );
        assert_eq!(s.temperature, 0.2);
        assert_eq!(s.download_connections, 8);
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
                ("DEBRIEF_LLAMA_DIR".to_string(), dir.to_string_lossy().into_owned()),
                ("DEBRIEF_LLAMA_BUILD".to_string(), "auto".to_string())
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
        assert_eq!(env[1], ("DEBRIEF_LLAMA_BUILD".to_string(), "vulkan".to_string()));
        assert_eq!(env[2], ("DEBRIEF_LLAMA_SERVER".to_string(), server.to_string()));
        let bad = AiSettings {
            inference_build: "Not Valid!".into(),
            llama_server_path: Some("relative/llama-server".into()),
            ..AiSettings::default()
        };
        assert_eq!(bad.validate().len(), 2);
    }

    #[test]
    fn update_policies_and_interval() {
        use crate::updates::UpdateKind;
        let d = AiSettings::default();
        assert_eq!(d.update_policy(UpdateKind::Runtime), UpdatePolicy::Ask);
        assert_eq!(d.update_interval_secs(), 86_400);
        let s: AiSettings =
            serde_json::from_str(r#"{"runtime_updates":"automatic","model_updates":"bogus","update_check_hours":99999}"#).unwrap();
        let s = s.clamp();
        assert_eq!(s.update_policy(UpdateKind::Runtime), UpdatePolicy::Automatic);
        assert_eq!(
            s.update_policy(UpdateKind::Model),
            UpdatePolicy::Ask,
            "unknown words mean the default"
        );
        assert_eq!(s.update_interval_secs(), 720 * 3600);
        assert!(serde_json::to_string(&s).unwrap().contains(r#""runtime_updates":"automatic""#));
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
        for k in settings.as_object().unwrap().keys().filter(|k| *k != "version") {
            assert!(keys.contains(&k.as_str()), "{k} missing from the schema");
        }
        for f in v.as_array().unwrap() {
            for k in ["key", "label", "description", "type", "default", "restartRequired"] {
                assert!(f.get(k).is_some(), "{f}");
            }
        }
    }
}
