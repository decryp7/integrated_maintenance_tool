//! What a running llama-server can do, from what it reports (`GET /props`), never from what Debrief expects of a build
//! (ADR 0009): newer builds chose their own media marker, and builds differ in what `/props` carries. Read once after
//! every start into a typed [`ServerInfo`]; the backend decides the context, image support and media marker from it.

use crate::completion::{media_marker, DEFAULT_MEDIA_MARKER};
use debrief_ai::worker::protocol::ServerInfo;
use serde_json::Value;
use std::path::Path;

/// Longest text kept from a `/props` field (model names, build ids).
const MAX_FIELD: usize = 200;

fn short(v: Option<&Value>) -> Option<String> {
    let s = v?.as_str()?.trim();
    if s.is_empty() {
        return None;
    }
    Some(s.chars().take(MAX_FIELD).collect())
}

/// The typed record of a server's `/props`. Missing, mistyped or empty fields are `None`; never panics.
pub fn server_info(props: &Value) -> ServerInfo {
    let n_ctx = props
        .pointer("/default_generation_settings/n_ctx")
        .or_else(|| props.get("n_ctx"))
        .and_then(Value::as_u64)
        .filter(|n| *n > 0)
        .and_then(|n| usize::try_from(n).ok());
    let modality = |name: &str| props.get("modalities").and_then(|m| m.get(name)).and_then(Value::as_bool);
    // The alias, or the model file's name (never its folder: the path may name the user).
    let model = short(props.get("model_alias")).or_else(|| {
        short(props.get("model_path")).and_then(|p| {
            let name = p.rsplit(['/', '\\']).next().unwrap_or(&p).to_string();
            Some(name).filter(|n| !n.is_empty())
        })
    });
    ServerInfo {
        n_ctx,
        vision: modality("vision"),
        audio: modality("audio"),
        media_marker: media_marker(props),
        model,
        build: short(props.get("build_info")),
        chat_template: props
            .get("chat_template")
            .and_then(Value::as_str)
            .is_some_and(|t| !t.trim().is_empty()),
    }
}

/// What the backend uses, decided from what the server reported and how it was started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capabilities {
    pub n_ctx: usize,
    /// Images are accepted: the projector was loaded and the server does not deny reading images.
    pub vision: bool,
    /// The marker each image takes in the prompt.
    pub media_marker: String,
    /// The record as the server reported it (for the AI status).
    pub reported: ServerInfo,
}

impl Capabilities {
    /// `requested_ctx` when the server does not say; `projector` when it was started with `--mmproj`. A server that
    /// reports `modalities.vision = false` despite the projector gets no images (it would refuse them).
    pub fn decide(reported: ServerInfo, requested_ctx: usize, projector: bool) -> Capabilities {
        Capabilities {
            n_ctx: reported.n_ctx.unwrap_or(requested_ctx),
            vision: projector && reported.vision != Some(false),
            media_marker: reported.media_marker.clone().unwrap_or_else(|| DEFAULT_MEDIA_MARKER.to_string()),
            reported,
        }
    }
}

/// The model's file stem, as the name to show when neither the GGUF nor the server names it.
pub fn file_stem(model: &Path) -> String {
    model.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_full_props_answer_is_read() {
        let props = json!({
            "default_generation_settings": {"n_ctx": 8192},
            "modalities": {"vision": true, "audio": false},
            "media_marker": "<__media_ab12__>",
            "model_path": "/home/someone/models/Qwen3-VL-4B.gguf",
            "build_info": "b7000-abc1234",
            "chat_template": "{% for m in messages %}...{% endfor %}"
        });
        let i = server_info(&props);
        assert_eq!(i.n_ctx, Some(8192));
        assert_eq!((i.vision, i.audio), (Some(true), Some(false)));
        assert_eq!(i.media_marker.as_deref(), Some("<__media_ab12__>"));
        assert_eq!(i.model.as_deref(), Some("Qwen3-VL-4B.gguf"), "only the file name, never the folder");
        assert_eq!(i.build.as_deref(), Some("b7000-abc1234"));
        assert!(i.chat_template);
        let c = Capabilities::decide(i, 4096, true);
        assert_eq!((c.n_ctx, c.vision, c.media_marker.as_str()), (8192, true, "<__media_ab12__>"));
    }

    #[test]
    fn missing_and_odd_fields_fall_back_without_assuming() {
        for props in [
            json!({}),
            json!(null),
            json!({"n_ctx": 0, "modalities": "yes", "media_marker": 3, "model_alias": "  ", "chat_template": ""}),
            json!({"default_generation_settings": {"n_ctx": "big"}, "modalities": {"vision": "true"}}),
        ] {
            let i = server_info(&props);
            assert_eq!(i, ServerInfo::default(), "{props}");
            let c = Capabilities::decide(i, 4096, true);
            assert_eq!((c.n_ctx, c.vision, c.media_marker.as_str()), (4096, true, DEFAULT_MEDIA_MARKER));
        }
        // The top-level n_ctx of older builds, an alias before the path, Windows paths, long fields cut.
        let i = server_info(&json!({"n_ctx": 2048, "model_alias": "tiny", "model_path": "C:\\m\\x.gguf"}));
        assert_eq!((i.n_ctx, i.model.as_deref()), (Some(2048), Some("tiny")));
        assert_eq!(
            server_info(&json!({"model_path": "C:\\Users\\me\\x.gguf"})).model.as_deref(),
            Some("x.gguf")
        );
        assert_eq!(server_info(&json!({"build_info": "b".repeat(500)})).build.unwrap().len(), MAX_FIELD);
    }

    #[test]
    fn the_server_decides_about_images() {
        let denies = ServerInfo {
            vision: Some(false),
            ..ServerInfo::default()
        };
        assert!(
            !Capabilities::decide(denies, 1, true).vision,
            "a server that says no gets no images"
        );
        let claims = ServerInfo {
            vision: Some(true),
            ..ServerInfo::default()
        };
        assert!(
            !Capabilities::decide(claims, 1, false).vision,
            "never without the projector Debrief gave it"
        );
        assert_eq!(file_stem(Path::new("/m/model.Q4.gguf")), "model.Q4");
    }
}
