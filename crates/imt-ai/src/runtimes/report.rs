//! Which installed builds work on this machine, as reported by `imt-ai-worker --list-builds` (the worker
//! runs each build's `llama-server --list-devices`, out of IMT's process). The types are the JSON contract
//! between the worker and IMT; [`list_builds`] runs the worker and parses its answer.

use crate::error::{AiError, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

/// One accelerator a build reported.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    /// ggml's device name, e.g. `Vulkan0`, `CUDA0`, `MTL0`.
    pub name: String,
    pub description: String,
    pub total_mib: u64,
    pub free_mib: u64,
}

impl Device {
    /// The ggml backend this device belongs to, spelled for the UI: "Vulkan", "CUDA", "Metal", ...
    pub fn backend(&self) -> String {
        friendly_backend(self.name.trim_end_matches(|c: char| c.is_ascii_digit()))
    }

    /// Whether this is a real GPU (software rasterisers such as llvmpipe show up as Vulkan devices).
    pub fn is_gpu(&self) -> bool {
        let d = self.description.to_ascii_lowercase();
        !["llvmpipe", "lavapipe", "swiftshader", "basic render", "softpipe"]
            .iter()
            .any(|s| d.contains(s))
    }

    pub fn free_gb(&self) -> f64 {
        self.free_mib as f64 / 1024.0
    }
}

/// ggml's short backend names, spelled out for the UI.
pub fn friendly_backend(name: &str) -> String {
    match name.to_ascii_lowercase().as_str() {
        "mtl" | "metal" => "Metal".into(),
        "vulkan" | "vk" => "Vulkan".into(),
        "cuda" => "CUDA".into(),
        "rocm" | "hip" => "ROCm".into(),
        "sycl" => "SYCL".into(),
        "openvino" => "OpenVINO".into(),
        "" => "CPU".into(),
        _ => name.to_string(),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildInfo {
    pub id: String,
    pub label: String,
    /// It runs here and, for a GPU build, sees a GPU.
    pub usable: bool,
    /// Why it is not usable.
    pub problem: Option<String>,
    pub devices: Vec<Device>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildsReport {
    /// The llama.cpp build tag of the installed builds, when known.
    pub release: Option<String>,
    /// `installed`, `custom` or `missing`.
    pub source: String,
    /// The setting as the worker understood it: `auto` or an id.
    pub choice: String,
    /// The build `auto` would use now.
    pub auto: Option<String>,
    pub builds: Vec<BuildInfo>,
    /// Set when nothing was found (what to do about it).
    pub problem: Option<String>,
}

/// Runs `worker --list-builds` with `env` (see `AiSettings::worker_env`) and returns its report.
pub fn list_builds(worker: &Path, env: &[(String, String)], timeout: Duration) -> Result<BuildsReport> {
    let mut cmd = imt_core::process::quiet_command(worker);
    cmd.arg("--list-builds")
        .envs(env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = cmd
        .spawn()
        .map_err(|e| AiError::Engine(format!("cannot run the AI worker ({}): {e}", worker.display())))?;
    let out = child.stdout.take().map(|mut s| {
        std::thread::spawn(move || {
            let mut b = Vec::new();
            let _ = std::io::Read::read_to_end(&mut std::io::Read::take(&mut s, 4 * 1024 * 1024), &mut b);
            b
        })
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let out = out.and_then(|h| h.join().ok()).unwrap_or_default();
    match status {
        None => Err(AiError::Engine(format!(
            "checking the inference builds took longer than {} s",
            timeout.as_secs()
        ))),
        Some(s) if !s.success() => Err(AiError::Engine(format!("checking the inference builds failed ({s})"))),
        Some(_) => serde_json::from_slice(&out).map_err(|_| AiError::Malformed("the AI worker sent an unreadable build list".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devices_describe_themselves() {
        let d = Device {
            name: "CUDA0".into(),
            description: "NVIDIA GeForce RTX 4070".into(),
            total_mib: 12288,
            free_mib: 11264,
        };
        assert_eq!(d.backend(), "CUDA");
        assert!(d.is_gpu());
        assert!((d.free_gb() - 11.0).abs() < 1e-9);
        let soft = Device {
            name: "Vulkan1".into(),
            description: "llvmpipe (LLVM 15)".into(),
            ..d
        };
        assert!(!soft.is_gpu());
        for (raw, want) in [
            ("MTL", "Metal"),
            ("ROCm", "ROCm"),
            ("SYCL", "SYCL"),
            ("OpenVINO", "OpenVINO"),
            ("", "CPU"),
            ("BLAS", "BLAS"),
            ("vk", "Vulkan"),
        ] {
            assert_eq!(friendly_backend(raw), want);
        }
    }

    #[test]
    fn the_report_is_camel_case_json() {
        let r = BuildsReport {
            release: Some("b1".into()),
            source: "installed".into(),
            choice: "auto".into(),
            auto: Some("cpu".into()),
            builds: vec![BuildInfo {
                id: "cpu".into(),
                label: "CPU".into(),
                usable: true,
                problem: None,
                devices: vec![],
            }],
            problem: None,
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["builds"][0]["usable"], true);
        assert_eq!(serde_json::from_value::<BuildsReport>(v).unwrap(), r);
    }

    #[test]
    fn running_a_missing_or_wrong_worker_fails_cleanly() {
        let e = list_builds(Path::new("/no/such/worker"), &[], Duration::from_secs(1)).unwrap_err();
        assert!(matches!(e, AiError::Engine(_)));
        // Any program that does not print a report: the test binary itself, asked to list nothing.
        let me = std::env::current_exe().unwrap();
        let r = list_builds(&me, &[], Duration::from_secs(30));
        assert!(r.is_err());
    }
}
