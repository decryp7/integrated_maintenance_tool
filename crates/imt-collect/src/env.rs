//! Environment snapshot (`system/environment.json`).

use serde_json::{json, Value};
use sysinfo::{Disks, System};

/// The host name, if the OS reports one.
pub fn host_name() -> Option<String> {
    System::host_name()
}

/// Allow-listed variables (case-insensitive names), sorted by name.
pub fn filter_vars(allow: &[String], vars: impl Iterator<Item = (String, String)>) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = vars.filter(|(k, _)| allow.iter().any(|a| a.eq_ignore_ascii_case(k))).collect();
    v.sort();
    v
}

/// OS, host, CPU, memory, disks and uptime, plus the filtered variables.
pub fn snapshot(allow: &[String], vars: impl Iterator<Item = (String, String)>, imt_version: &str) -> Value {
    let mut sys = System::new();
    sys.refresh_memory();
    sys.refresh_cpu_all();
    let cpus = sys.cpus();
    let disks: Vec<Value> = Disks::new_with_refreshed_list()
        .iter()
        .map(|d| {
            json!({
                "name": d.name().to_string_lossy(),
                "mount": d.mount_point().to_string_lossy(),
                "fileSystem": d.file_system().to_string_lossy(),
                "totalBytes": d.total_space(),
                "availableBytes": d.available_space(),
            })
        })
        .collect();
    let env: serde_json::Map<String, Value> = filter_vars(allow, vars).into_iter().map(|(k, v)| (k, Value::String(v))).collect();
    json!({
        "os": { "name": System::name(), "version": System::os_version(), "longVersion": System::long_os_version(), "kernel": System::kernel_version(), "arch": System::cpu_arch() },
        "host": System::host_name(),
        "cpu": { "brand": cpus.first().map(|c| c.brand().to_string()), "logicalCores": cpus.len() },
        "memory": { "totalBytes": sys.total_memory(), "availableBytes": sys.available_memory() },
        "disks": disks,
        "uptimeSecs": System::uptime(),
        "bootTime": System::boot_time(),
        "imtVersion": imt_version,
        "collectorVersion": env!("CARGO_PKG_VERSION"),
        "environment": env,
    })
}
