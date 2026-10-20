//! Inference build ids (`cpu`, `vulkan`, `cuda-13`, `metal`, ...): what they are called, which need a
//! GPU, and the order they are tried in. Shared by IMT (Settings, the runtime manager) and the worker,
//! so both agree. Pure.

/// The user's choice of build (setting `inference_build`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildChoice {
    /// The best installed build that works on this machine.
    Auto,
    /// A specific build id.
    Fixed(String),
}

impl BuildChoice {
    /// Parses the setting; blank, `auto` and malformed ids mean automatic.
    pub fn parse(s: &str) -> BuildChoice {
        let s = s.trim().to_ascii_lowercase();
        if s.is_empty() || s == "auto" || !is_build_id(&s) {
            BuildChoice::Auto
        } else {
            BuildChoice::Fixed(s)
        }
    }

    pub fn as_setting(&self) -> &str {
        match self {
            BuildChoice::Auto => "auto",
            BuildChoice::Fixed(id) => id,
        }
    }
}

/// Build ids are folder names: short, lowercase ASCII letters, digits, `.` and `-`.
pub fn is_build_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 32
        && !id.starts_with('.')
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-')
}

/// The number of a llama.cpp release tag (`b11669` → 11669), to order releases.
pub fn tag_number(tag: &str) -> Option<u64> {
    tag.strip_prefix('b').filter(|n| !n.is_empty()).and_then(|n| n.parse().ok())
}

/// What a build accelerates with, for the UI.
pub fn label(id: &str) -> String {
    match id {
        "cpu" => "CPU".into(),
        "vulkan" => "Vulkan (any GPU vendor)".into(),
        "metal" => "Metal (Apple GPU)".into(),
        "rocm" => "ROCm / HIP (AMD)".into(),
        "sycl" => "SYCL (Intel GPU, FP16)".into(),
        "sycl-fp32" => "SYCL (Intel GPU, FP32)".into(),
        "openvino" => "OpenVINO (Intel)".into(),
        "custom" => "Custom llama-server".into(),
        _ => match id.strip_prefix("cuda-") {
            Some(v) => format!("CUDA {v} (NVIDIA)"),
            None => id.to_string(),
        },
    }
}

/// Whether the build only makes sense with a GPU device (it is not used when none is listed).
pub fn needs_gpu(id: &str) -> bool {
    !matches!(id, "cpu" | "custom")
}

/// Rank in the automatic order (lower first); `None` = only used when chosen explicitly.
/// Metal, then CUDA (newest toolkit first), then Vulkan, then CPU. ROCm, SYCL and OpenVINO can be
/// chosen but are never picked automatically: Vulkan covers those GPUs with fewer driver requirements.
fn auto_rank(id: &str) -> Option<u32> {
    match id {
        "metal" | "custom" => Some(0),
        "vulkan" => Some(300),
        "cpu" => Some(900),
        _ => {
            let major: u32 = id.strip_prefix("cuda-")?.split('.').next()?.parse().ok()?;
            Some(200 - major.min(199))
        }
    }
}

/// The order to try builds in: the chosen one first (when installed), then the automatic order as the
/// fallback. Every installed id appears at most once; explicit-only builds appear only when chosen.
pub fn try_order(installed: &[String], choice: &BuildChoice) -> Vec<String> {
    let mut auto: Vec<&String> = installed.iter().filter(|id| auto_rank(id).is_some()).collect();
    auto.sort_by_key(|id| (auto_rank(id), id.as_str()));
    let mut out = Vec::new();
    if let BuildChoice::Fixed(id) = choice {
        if installed.contains(id) {
            out.push(id.clone());
        }
    }
    for id in auto {
        if !out.contains(id) {
            out.push(id.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn choices_parse_leniently() {
        assert_eq!(BuildChoice::parse(""), BuildChoice::Auto);
        assert_eq!(BuildChoice::parse(" AUTO "), BuildChoice::Auto);
        assert_eq!(BuildChoice::parse("../x"), BuildChoice::Auto);
        assert_eq!(BuildChoice::parse("Vulkan"), BuildChoice::Fixed("vulkan".into()));
        assert_eq!(BuildChoice::parse("vulkan").as_setting(), "vulkan");
        assert_eq!(BuildChoice::Auto.as_setting(), "auto");
        assert!(is_build_id("cuda-12.4") && !is_build_id(".hidden") && !is_build_id(&"a".repeat(33)) && !is_build_id(""));
    }

    #[test]
    fn automatic_order_prefers_cuda_then_vulkan_then_cpu() {
        let all = ids(&["cpu", "cuda-12", "cuda-13", "openvino", "rocm", "sycl", "vulkan"]);
        assert_eq!(try_order(&all, &BuildChoice::Auto), ids(&["cuda-13", "cuda-12", "vulkan", "cpu"]));
        assert_eq!(try_order(&ids(&["cpu", "metal"]), &BuildChoice::Auto), ids(&["metal", "cpu"]));
        assert_eq!(
            try_order(&all, &BuildChoice::Fixed("rocm".into())),
            ids(&["rocm", "cuda-13", "cuda-12", "vulkan", "cpu"])
        );
        assert_eq!(
            try_order(&all, &BuildChoice::Fixed("cpu".into())),
            ids(&["cpu", "cuda-13", "cuda-12", "vulkan"])
        );
        assert_eq!(try_order(&ids(&["cpu"]), &BuildChoice::Fixed("vulkan".into())), ids(&["cpu"]));
        assert!(try_order(&[], &BuildChoice::Auto).is_empty());
        assert!(try_order(&ids(&["future-npu"]), &BuildChoice::Auto).is_empty());
    }

    #[test]
    fn tags_are_numbered() {
        assert_eq!(tag_number("b11669"), Some(11669));
        assert_eq!(tag_number("b"), None);
        assert_eq!(tag_number("v1"), None);
        assert_eq!(tag_number("bx"), None);
    }

    #[test]
    fn labels_and_gpu_needs() {
        assert_eq!(label("cuda-13"), "CUDA 13 (NVIDIA)");
        assert_eq!(label("cpu"), "CPU");
        assert_eq!(label("future-npu"), "future-npu");
        for id in ["vulkan", "metal", "rocm", "sycl", "sycl-fp32", "openvino", "custom"] {
            assert!(!label(id).is_empty() && label(id) != id);
        }
        assert!(needs_gpu("vulkan") && !needs_gpu("cpu") && !needs_gpu("custom"));
    }
}
