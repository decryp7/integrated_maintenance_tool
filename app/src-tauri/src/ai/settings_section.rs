//! Settings → AI: the fields of `imt_ai::settings::schema()` as a section of the app's settings system.
//! The active model is chosen in the model manager, so it is not a field here.
//! The section's values live in `settings.json` like every other section and are applied to the runtime
//! (and written to `ai.json`) at startup and on every change, so `settings.json` is their source of truth;
//! `ai.json` adds what is chosen elsewhere (the active model).

use super::core::AiCore;
use crate::settings::AI_SECTION;
use imt_ai::runtimes::builds::label as build_label;
use imt_ai::settings::{AiSettings, GpuOffload};
use imt_ai::updates::UpdatePolicy;
use imt_core::settings::{Configurable, EnumOption, FieldKind, SectionOwner, SettingField, SettingsSection, ValidValues};
use serde_json::Value;
use std::sync::Arc;

/// Schema keys that are not settings fields (see the module doc).
const NOT_FIELDS: &[&str] = &["active_model"];
const MAX_TEXT: usize = 2048;
/// "all layers on the GPU".
const ALL_LAYERS: u32 = 1000;
/// The automatic inference build.
const AUTO: &str = "auto";

pub struct AiSettingsSection {
    pub core: Arc<AiCore>,
}

fn num(v: &Value, k: &str) -> Option<f64> {
    v.get(k).and_then(Value::as_f64)
}

fn label_of(option: &str) -> String {
    match option {
        "auto" => "Automatic".into(),
        "0" => "CPU only".into(),
        "all" => "All layers on the GPU".into(),
        "automatic" => "Automatic: download and install in the background".into(),
        "ask" => "Ask before updating".into(),
        "manual" => "Manual: only when I choose Check now".into(),
        n if n.parse::<u32>().is_ok() => format!("{n} layers on the GPU"),
        other => other.into(),
    }
}

/// One schema entry as a field, or `None` for entries that are not settings fields.
fn field_of(f: &Value) -> Option<SettingField> {
    let key = f.get("key")?.as_str()?;
    if NOT_FIELDS.contains(&key) {
        return None;
    }
    let label = f.get("label").and_then(Value::as_str).unwrap_or(key);
    let desc = f.get("description").and_then(Value::as_str).unwrap_or("");
    let default = f.get("default").cloned().unwrap_or(Value::Null);
    let (kind, default) = match f.get("type")?.as_str()? {
        "bool" => (FieldKind::Bool, default),
        "int" => (
            FieldKind::Int {
                min: num(f, "min")? as i64,
                max: num(f, "max")? as i64,
                step: 1,
                unit: None,
            },
            default,
        ),
        "float" => (
            FieldKind::Float {
                min: num(f, "min")?,
                max: num(f, "max")?,
                step: 0.1,
                unit: None,
            },
            default,
        ),
        "enum" => {
            let options = f
                .get("options")?
                .as_array()?
                .iter()
                .filter_map(Value::as_str)
                .map(|o| EnumOption::new(o, label_of(o)))
                .collect();
            (FieldKind::Enum { options }, default)
        }
        "path" => (FieldKind::Path { dir: true }, Value::from(default.as_str().unwrap_or(""))),
        "file" => (FieldKind::Path { dir: false }, Value::from(default.as_str().unwrap_or(""))),
        "string" => (
            FieldKind::Text {
                max_len: MAX_TEXT,
                pattern: None,
            },
            Value::from(default.as_str().unwrap_or("")),
        ),
        _ => return None,
    };
    let mut field = SettingField::new(key, label, kind, default).describe(desc);
    if matches!(
        key,
        "threads" | "hf_endpoint" | "download_proxy" | "max_download_gb" | "models_dir" | "temperature" | "llama_server_path"
    ) {
        field = field.as_advanced();
    }
    Some(field)
}

/// The *Inference build* options: `auto`, then `installed` (when given) or the schema's, each with its label.
fn with_build_options(mut field: SettingField, installed: Option<&[String]>) -> SettingField {
    if field.key != "inference_build" {
        return field;
    }
    if let FieldKind::Enum { options } = &mut field.kind {
        let ids: Vec<String> = match installed {
            Some(ids) => ids.iter().filter(|id| id.as_str() != AUTO).cloned().collect(),
            None => options.iter().map(|o| o.value.clone()).filter(|v| v != AUTO).collect(),
        };
        *options = std::iter::once(EnumOption::new(AUTO, label_of(AUTO)))
            .chain(ids.iter().map(|id| EnumOption::new(id, build_label(id))))
            .collect();
    }
    field
}

/// The section, from the library's schema, offering every build the schema names (tests, documentation).
#[cfg(test)]
pub fn section() -> SettingsSection {
    build_section(None)
}

/// The section with *Inference build* limited to `auto` and the builds installed here (`optionsSource:
/// aiInferenceBuilds`): a build that is not installed cannot be chosen, and a stored choice of a deleted
/// build falls back to automatic.
pub fn section_with_builds(installed: &[String]) -> SettingsSection {
    build_section(Some(installed))
}

fn build_section(installed: Option<&[String]>) -> SettingsSection {
    let mut s = SettingsSection::new(AI_SECTION, "AI assistant", SectionOwner::App).describe(
        "The assistant runs fully on this computer with a local model; nothing about your logs is sent anywhere. \
Only the model file is downloaded (from Hugging Face). Web pages reach it only when you hand them over from the internal browser.",
    );
    if let Some(fields) = imt_ai::settings::schema().as_array() {
        for f in fields.iter().filter_map(field_of) {
            s = s.field(with_build_options(f, installed));
        }
    }
    s
}

fn gpu_from(word: &str) -> GpuOffload {
    match word {
        "all" => GpuOffload::Layers(ALL_LAYERS),
        w => w.parse::<u32>().map_or(GpuOffload::Auto, GpuOffload::Layers),
    }
}

fn text(values: &ValidValues, key: &str) -> Option<String> {
    values.text(key).map(str::trim).filter(|s| !s.is_empty()).map(String::from)
}

/// Builds the settings the section's values describe, over `base` (fields the section does not have).
pub fn to_settings(values: &ValidValues, base: &AiSettings) -> AiSettings {
    let mut s = base.clone();
    let int = |k: &str, d: i64| values.int(k).unwrap_or(d);
    if let Some(b) = values.bool("autoload") {
        s.autoload = b;
    }
    s.idle_unload_minutes = int("idle_unload_minutes", i64::from(s.idle_unload_minutes)) as u32;
    s.gpu_offload = values.text("gpu_offload").map_or(s.gpu_offload, gpu_from);
    s.context_length = int("context_length", s.context_length as i64) as usize;
    s.max_tokens = int("max_tokens", s.max_tokens as i64) as usize;
    s.temperature = values.float("temperature").map_or(s.temperature, |t| t as f32);
    s.threads = int("threads", i64::from(s.threads)) as u32;
    if let Some(e) = text(values, "hf_endpoint") {
        s.hf_endpoint = e;
    }
    s.download_proxy = text(values, "download_proxy");
    s.max_download_gb = int("max_download_gb", i64::from(s.max_download_gb)) as u32;
    s.models_dir = text(values, "models_dir");
    if let Some(b) = text(values, "inference_build") {
        s.inference_build = b;
    }
    s.llama_server_path = text(values, "llama_server_path");
    if let Some(p) = values.text("runtime_updates").and_then(UpdatePolicy::parse) {
        s.runtime_updates = p;
    }
    if let Some(p) = values.text("model_updates").and_then(UpdatePolicy::parse) {
        s.model_updates = p;
    }
    s.update_check_hours = int("update_check_hours", i64::from(s.update_check_hours)).clamp(1, 720) as u32;
    s.clamp()
}

impl Configurable for AiSettingsSection {
    fn settings_section(&self) -> SettingsSection {
        section_with_builds(&self.core.installed_build_ids())
    }

    fn apply_settings(&self, values: &ValidValues) -> anyhow::Result<()> {
        let next = to_settings(values, &self.core.settings());
        self.core.apply_settings(next).map_err(|e| anyhow::anyhow!(e.message))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use imt_core::settings::sanitize;
    use serde_json::json;

    #[test]
    fn the_section_is_valid_and_covers_the_schema() {
        let s = section();
        s.check().unwrap();
        assert_eq!(s.id, AI_SECTION);
        let keys: Vec<&str> = s.fields.iter().map(|f| f.key.as_str()).collect();
        for f in imt_ai::settings::schema().as_array().unwrap() {
            let k = f["key"].as_str().unwrap();
            assert_eq!(keys.contains(&k), !NOT_FIELDS.contains(&k), "{k}");
        }
        assert!(s.description.contains("fully on this computer"));
        let gpu = s.find("gpu_offload").unwrap();
        assert!(matches!(&gpu.kind, FieldKind::Enum { options } if options.iter().any(|o| o.value == "all" && o.label.contains("All"))));
        assert!(s.find("hf_endpoint").unwrap().advanced);
        assert!(field_of(&json!({"key": "x", "type": "secret"})).is_none());
        assert!(
            field_of(&json!({"key": "x", "type": "int"})).is_none(),
            "an int without limits is skipped"
        );
        assert_eq!(label_of("8"), "8 layers on the GPU");
        assert_eq!(label_of("other"), "other");
    }

    #[test]
    fn defaults_map_back_to_default_settings() {
        let s = section();
        let (values, errors) = sanitize(&s, &Value::Null);
        assert!(errors.is_empty());
        assert_eq!(to_settings(&values, &AiSettings::default()), AiSettings::default());
    }

    #[test]
    fn values_become_settings() {
        // Absolute on every platform (`/models` is not absolute on Windows).
        let (models, server) = if cfg!(windows) {
            (r"C:\models", r"C:\llama\llama-server.exe")
        } else {
            ("/models", "/opt/llama/llama-server")
        };
        let s = section();
        let (values, errors) = sanitize(
            &s,
            &json!({
                "autoload": false, "idle_unload_minutes": 0, "gpu_offload": "all", "context_length": 4096,
                "max_tokens": 512, "temperature": 0.5, "threads": 4, "hf_endpoint": "https://mirror.example",
                "download_proxy": "http://p:3128", "max_download_gb": 9, "models_dir": models,
                "inference_build": "vulkan", "llama_server_path": server,
                "runtime_updates": "automatic", "model_updates": "manual", "update_check_hours": 6
            }),
        );
        assert!(errors.is_empty(), "{errors:?}");
        let base = AiSettings {
            active_model: Some("keep.gguf".into()),
            ..AiSettings::default()
        };
        let a = to_settings(&values, &base);
        assert_eq!(a.active_model.as_deref(), Some("keep.gguf"));
        assert!(!a.autoload);
        assert_eq!(
            (a.idle_unload_minutes, a.context_length, a.max_tokens, a.threads),
            (0, 4096, 512, 4)
        );
        assert_eq!(a.gpu_offload, GpuOffload::Layers(ALL_LAYERS));
        assert!((a.temperature - 0.5).abs() < 1e-6);
        assert_eq!(a.hf_endpoint, "https://mirror.example");
        assert_eq!(a.download_proxy.as_deref(), Some("http://p:3128"));
        assert_eq!(a.models_dir.as_deref(), Some(models));
        assert_eq!(
            (a.inference_build.as_str(), a.llama_server_path.as_deref()),
            ("vulkan", Some(server))
        );
        assert_eq!(
            (a.runtime_updates, a.model_updates, a.update_check_hours),
            (UpdatePolicy::Automatic, UpdatePolicy::Manual, 6)
        );
        assert_eq!(label_of("ask"), "Ask before updating");
        for (word, expected) in [
            ("8", GpuOffload::Layers(8)),
            ("0", GpuOffload::Layers(0)),
            ("auto", GpuOffload::Auto),
        ] {
            assert_eq!(gpu_from(word), expected);
        }
    }
}
