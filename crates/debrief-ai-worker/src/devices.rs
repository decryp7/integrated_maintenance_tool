//! The devices a llama-server build can use, parsed from `llama-server --list-devices`:
//!
//! ```text
//! Available devices:
//!   Vulkan0: AMD Radeon RX 7800 XT (RADV NAVI32) (16368 MiB, 15912 MiB free)
//!   CUDA0: NVIDIA GeForce RTX 4070 (12281 MiB, 11034 MiB free)
//! ```
//!
//! A CPU-only build prints `(none)`. Parsing is pure and tolerant: lines that do not match are skipped.

pub use debrief_ai::runtimes::report::{friendly_backend, Device};

/// Parses the output of `--list-devices`.
pub fn parse_list_devices(text: &str) -> Vec<Device> {
    text.lines().filter_map(parse_line).collect()
}

fn parse_line(line: &str) -> Option<Device> {
    let line = line.trim();
    let (name, rest) = line.split_once(": ")?;
    if name.is_empty() || name.contains(char::is_whitespace) {
        return None;
    }
    // The memory is the last parenthesised group; the description may contain parentheses itself.
    let rest = rest.strip_suffix(')')?;
    let open = rest.rfind('(')?;
    let (description, mem) = (rest[..open].trim(), &rest[open + 1..]);
    let (total, free) = mem.split_once(',')?;
    let mib = |s: &str| {
        s.trim()
            .strip_suffix("free")
            .unwrap_or(s)
            .trim()
            .strip_suffix("MiB")?
            .trim()
            .parse::<u64>()
            .ok()
    };
    Some(Device {
        name: name.to_string(),
        description: description.to_string(),
        total_mib: mib(total)?,
        free_mib: mib(free)?,
    })
}

/// The GPU with the most free memory.
pub fn best_gpu(devices: &[Device]) -> Option<&Device> {
    devices.iter().filter(|d| d.is_gpu()).max_by_key(|d| d.free_mib)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_devices_and_skips_noise() {
        let out = "ggml_vulkan: Found 2 Vulkan devices:\nAvailable devices:\n  Vulkan0: AMD Radeon (RADV NAVI32) (16368 MiB, 15912 MiB free)\n  Vulkan1: llvmpipe (LLVM 15.0.7, 256 bits) (32000 MiB, 32000 MiB free)\n  CUDA0: NVIDIA GeForce RTX 4070 (12281 MiB, 11034 MiB free)\n  MTL0: Apple M2 (21845 MiB, 21845 MiB free)\n  junk: line (x MiB, y MiB free)\n";
        let d = parse_list_devices(out);
        assert_eq!(d.len(), 4);
        assert_eq!(d[0].description, "AMD Radeon (RADV NAVI32)");
        assert_eq!((d[0].total_mib, d[0].free_mib), (16368, 15912));
        assert_eq!(d[0].backend(), "Vulkan");
        assert!(!d[1].is_gpu());
        assert_eq!(d[2].backend(), "CUDA");
        assert_eq!(d[3].backend(), "Metal");
        assert!((d[2].free_gb() - 10.775).abs() < 0.01);
        assert_eq!(best_gpu(&d).map(|g| g.name.as_str()), Some("MTL0"));
    }

    #[test]
    fn a_cpu_build_has_no_devices() {
        assert!(parse_list_devices("Available devices:\n  (none)\n").is_empty());
        assert!(best_gpu(&[]).is_none());
        assert!(parse_list_devices("").is_empty());
    }
}
