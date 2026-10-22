//! What the machine can run: RAM, cores and GPUs. The probe sits behind a trait so the
//! recommender can be tested with any machine. External tools (`nvidia-smi`) run with a timeout.

use debrief_collect::CommandRunner;
use serde::Serialize;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Gpu {
    pub name: String,
    /// Dedicated video memory in GB, when known.
    pub vram_gb: Option<f64>,
    /// Memory shared with the CPU (Apple Silicon).
    pub unified: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hardware {
    pub os: String,
    pub arch: String,
    pub cpu_cores: usize,
    pub total_ram_gb: f64,
    pub available_ram_gb: f64,
    pub gpus: Vec<Gpu>,
}

impl Hardware {
    /// A short description for the UI.
    pub fn summary(&self) -> String {
        let gpu = match self
            .gpus
            .iter()
            .max_by(|a, b| a.vram_gb.unwrap_or(0.0).total_cmp(&b.vram_gb.unwrap_or(0.0)))
        {
            Some(g) if g.unified => format!("{} (shared memory)", g.name),
            Some(g) => match g.vram_gb {
                Some(v) => format!("{} with {:.0} GB video memory", g.name, v),
                None => g.name.clone(),
            },
            None => "no GPU detected".to_string(),
        };
        format!("{:.0} GB RAM, {} cores, {}", self.total_ram_gb, self.cpu_cores, gpu)
    }
}

pub trait HardwareProbe {
    fn probe(&self) -> Hardware;
}

const GB: f64 = 1024.0 * 1024.0 * 1024.0;
const SMI_TIMEOUT: Duration = Duration::from_secs(5);

/// Parses `nvidia-smi --query-gpu=name,memory.total --format=csv,noheader,nounits` (MiB).
pub fn parse_nvidia_smi(out: &str) -> Vec<Gpu> {
    out.lines()
        .filter_map(|l| {
            let (name, mem) = l.rsplit_once(',')?;
            let mib: f64 = mem.trim().parse().ok()?;
            let name = name.trim();
            (!name.is_empty()).then(|| Gpu {
                name: name.to_string(),
                vram_gb: Some(mib / 1024.0),
                unified: false,
            })
        })
        .collect()
}

/// Apple Silicon: the GPU shares all of the RAM.
pub fn apple_silicon_gpu(os: &str, arch: &str, total_ram_gb: f64) -> Option<Gpu> {
    (os == "macos" && arch == "aarch64").then(|| Gpu {
        name: "Apple Silicon GPU".into(),
        vram_gb: Some(total_ram_gb),
        unified: true,
    })
}

/// Combines GPU lists: `nvidia-smi` is more precise than DXGI, so DXGI entries that name NVIDIA
/// (or repeat a known name) are dropped.
pub fn merge_gpus(preferred: Vec<Gpu>, others: Vec<Gpu>) -> Vec<Gpu> {
    let mut out = preferred;
    for g in others {
        let lower = g.name.to_lowercase();
        let dup = out.iter().any(|p| p.name.eq_ignore_ascii_case(&g.name))
            || (lower.contains("nvidia") && out.iter().any(|p| p.name.to_lowercase().contains("nvidia")));
        if !dup {
            out.push(g);
        }
    }
    out
}

/// Everything the probe needs to know, gathered by the caller (so this is testable anywhere).
pub struct Facts<'a> {
    pub os: &'a str,
    pub arch: &'a str,
    pub cpu_cores: usize,
    pub total_ram_gb: f64,
    pub available_ram_gb: f64,
    /// GPUs from a platform API (DXGI on Windows).
    pub platform_gpus: Vec<Gpu>,
}

pub fn assemble<R: CommandRunner>(runner: &R, f: Facts<'_>) -> Hardware {
    let mut gpus = Vec::new();
    if let Some(g) = apple_silicon_gpu(f.os, f.arch, f.total_ram_gb) {
        gpus.push(g);
    }
    let args: Vec<String> = ["--query-gpu=name,memory.total", "--format=csv,noheader,nounits"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    if let Ok(o) = runner.run("nvidia-smi", &args, SMI_TIMEOUT) {
        if o.success() {
            gpus = merge_gpus(gpus, parse_nvidia_smi(&String::from_utf8_lossy(&o.stdout)));
        }
    }
    let gpus = merge_gpus(gpus, f.platform_gpus);
    Hardware {
        os: f.os.into(),
        arch: f.arch.into(),
        cpu_cores: f.cpu_cores,
        total_ram_gb: f.total_ram_gb,
        available_ram_gb: f.available_ram_gb,
        gpus,
    }
}

/// The real machine.
pub struct SystemProbe<R: CommandRunner> {
    runner: R,
}

impl<R: CommandRunner> SystemProbe<R> {
    pub fn new(runner: R) -> Self {
        SystemProbe { runner }
    }
}

impl<R: CommandRunner> HardwareProbe for SystemProbe<R> {
    fn probe(&self) -> Hardware {
        let mut sys = sysinfo::System::new();
        sys.refresh_memory();
        let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
        assemble(
            &self.runner,
            Facts {
                os: std::env::consts::OS,
                arch: std::env::consts::ARCH,
                cpu_cores: cores,
                total_ram_gb: sys.total_memory() as f64 / GB,
                available_ram_gb: sys.available_memory() as f64 / GB,
                platform_gpus: platform_gpus(),
            },
        )
    }
}

#[cfg(windows)]
fn platform_gpus() -> Vec<Gpu> {
    crate::dxgi::adapters()
}

#[cfg(not(windows))]
fn platform_gpus() -> Vec<Gpu> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use debrief_collect::{CommandError, CommandOutput};
    use std::cell::RefCell;

    struct Fake(Result<CommandOutput, CommandError>, RefCell<Vec<String>>);
    impl CommandRunner for Fake {
        fn run(&self, program: &str, _args: &[String], _t: Duration) -> Result<CommandOutput, CommandError> {
            self.1.borrow_mut().push(program.to_string());
            match &self.0 {
                Ok(o) => Ok(o.clone()),
                Err(_) => Err(CommandError::NotFound(program.to_string())),
            }
        }
    }

    fn ok(s: &str) -> Fake {
        Fake(
            Ok(CommandOutput {
                exit_code: Some(0),
                stdout: s.as_bytes().to_vec(),
                stderr: String::new(),
            }),
            RefCell::new(vec![]),
        )
    }

    fn facts(os: &'static str, arch: &'static str, platform: Vec<Gpu>) -> Facts<'static> {
        Facts {
            os,
            arch,
            cpu_cores: 8,
            total_ram_gb: 16.0,
            available_ram_gb: 10.0,
            platform_gpus: platform,
        }
    }

    #[test]
    fn parses_nvidia_smi_lines() {
        let g = parse_nvidia_smi("NVIDIA GeForce RTX 4060 Laptop GPU, 8188\nbad line\nNVIDIA A, x\n, 100\n");
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].name, "NVIDIA GeForce RTX 4060 Laptop GPU");
        assert!((g[0].vram_gb.unwrap() - 7.996).abs() < 0.01);
    }

    #[test]
    fn apple_silicon_is_unified() {
        let h = assemble(&ok(""), facts("macos", "aarch64", vec![]));
        assert_eq!(h.gpus.len(), 1);
        assert!(h.gpus[0].unified);
        assert_eq!(h.gpus[0].vram_gb, Some(16.0));
        assert!(h.summary().contains("shared memory"));
        assert!(apple_silicon_gpu("macos", "x86_64", 16.0).is_none());
        assert!(apple_silicon_gpu("linux", "aarch64", 16.0).is_none());
    }

    #[test]
    fn nvidia_smi_wins_over_dxgi_duplicates() {
        let dxgi = vec![
            Gpu {
                name: "NVIDIA GeForce RTX 4060 Laptop GPU".into(),
                vram_gb: Some(8.0),
                unified: false,
            },
            Gpu {
                name: "AMD Radeon(TM) Graphics".into(),
                vram_gb: Some(0.5),
                unified: false,
            },
        ];
        let r = ok("NVIDIA GeForce RTX 4060 Laptop GPU, 8188\n");
        let h = assemble(&r, facts("windows", "x86_64", dxgi));
        assert_eq!(h.gpus.len(), 2);
        assert_eq!(h.gpus[0].name, "NVIDIA GeForce RTX 4060 Laptop GPU");
        assert_eq!(r.1.borrow().as_slice(), ["nvidia-smi"]);
        assert!(h.summary().contains("GB video memory"));
    }

    #[test]
    fn missing_or_failing_nvidia_smi_degrades() {
        let r = Fake(Err(CommandError::NotFound("nvidia-smi".into())), RefCell::new(vec![]));
        let h = assemble(&r, facts("linux", "x86_64", vec![]));
        assert!(h.gpus.is_empty());
        assert!(h.summary().contains("no GPU detected"));
        let bad = Fake(
            Ok(CommandOutput {
                exit_code: Some(9),
                stdout: b"NVIDIA X, 100".to_vec(),
                stderr: String::new(),
            }),
            RefCell::new(vec![]),
        );
        assert!(assemble(&bad, facts("linux", "x86_64", vec![])).gpus.is_empty());
    }

    #[test]
    fn summary_without_known_vram() {
        let h = Hardware {
            os: "x".into(),
            arch: "y".into(),
            cpu_cores: 2,
            total_ram_gb: 8.0,
            available_ram_gb: 4.0,
            gpus: vec![Gpu {
                name: "Mystery".into(),
                vram_gb: None,
                unified: false,
            }],
        };
        assert!(h.summary().ends_with("Mystery"));
    }

    #[test]
    fn real_probe_reports_sane_values() {
        struct Missing;
        impl CommandRunner for Missing {
            fn run(&self, p: &str, _: &[String], _: Duration) -> Result<CommandOutput, CommandError> {
                Err(CommandError::NotFound(p.into()))
            }
        }
        let h = SystemProbe::new(Missing).probe();
        assert!(h.cpu_cores >= 1 && h.total_ram_gb > 0.1 && !h.os.is_empty());
    }
}
