//! Suggest settings (ADR 0031 §4): recommended values from the environment, by one pure function, and the
//! before/after table the Settings screen shows. The probe of the real machine is the thin edge at the bottom.

use super::section::schema;
use super::PerformanceSettings;
use serde::Serialize;
use serde_json::Value;
use std::path::Path;

/// The kind of disk the data folder is on, when the system tells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DiskKind {
    // Not reported by this build's probe yet (see `probe`); suggestions already handle it.
    #[allow(dead_code)]
    Ssd,
    Hdd,
}

/// What the suggestion is computed from. Sizes are MB.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Environment {
    pub total_memory_mb: u64,
    /// Memory available now (other programs' use excluded).
    pub free_memory_mb: u64,
    pub cores: u32,
    /// Free space where projects are kept, when known.
    pub disk_free_mb: Option<u64>,
    pub disk_kind: Option<DiskKind>,
    /// The active AI model (with its image projector), 0 when there is none: it is subtracted first.
    pub ai_model_mb: u64,
}

/// The system keeps at least this much, MB, and at least a quarter of the memory.
const MIN_RESERVE_MB: u64 = 2048;
/// Below this headroom, freed memory goes back at once and threads are few.
const TIGHT_MB: u64 = 3072;
/// Below this free disk space, spilling is capped at a quarter of it.
const SHORT_DISK_MB: u64 = 20 * 1024;

fn round_down(n: u64, step: u64) -> u64 {
    n / step * step
}

fn to_u32(n: u64) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// The memory Debrief's processes may share, MB: the total less the system's reserve and the AI model, no more than is
/// free now, at least 1 GB.
pub fn pool_mb(env: &Environment) -> u64 {
    let reserve = MIN_RESERVE_MB.max(env.total_memory_mb / 4);
    let ai = env.ai_model_mb * 6 / 5;
    env.total_memory_mb
        .saturating_sub(reserve)
        .saturating_sub(ai)
        .min(env.free_memory_mb)
        .max(1024)
}

/// The suggested settings for `env`, over `current` (what the suggestion does not cover stays: the spill folder, the
/// AI worker's cap, per-plugin caps, DuckDB's explicit limit is cleared so it follows the query memory).
pub fn suggest(env: &Environment, current: &PerformanceSettings) -> PerformanceSettings {
    let pool = pool_mb(env);
    let tight = pool < TIGHT_MB;
    let cap = round_down((pool / 4).clamp(1024, 8192), 256);
    let leave = match env.cores {
        0 | 1 => 0,
        2..=8 => 1,
        _ => 2,
    };
    let usable = env.cores.saturating_sub(leave).max(1);
    let hdd = env.disk_kind == Some(DiskKind::Hdd);
    PerformanceSettings {
        store_cap_mb: to_u32(cap),
        // The tested ratios of the 2 GiB cap: 768 MB importing, 300 MB querying.
        import_budget_mb: to_u32(round_down(cap * 3 / 8, 64).clamp(384, 4096)),
        query_budget_mb: to_u32((cap * 300 / 2048).clamp(236, 2048)),
        plugin_memory_mb: to_u32(round_down((pool / 4).clamp(512, 4096), 256)),
        return_memory_at_once: tight,
        // A spinning disk reads one file at a time best; elsewhere the import memory decides.
        parse_threads: if hdd { usable.min(4) } else { 0 },
        query_threads: if tight { 2 } else { (usable / 2).clamp(2, 8) },
        index_threads: if env.cores >= 8 { 2 } else { 1 },
        leave_cores_free: leave,
        duck_memory_limit_mb: 0,
        max_spill_mb: match env.disk_free_mb {
            Some(free) if free < SHORT_DISK_MB => to_u32((free / 4).max(1)),
            _ => 0,
        },
        allocator_flush_mb: if tight { 8 } else { 32 },
        ..current.clone()
    }
}

/// One line of the before/after table.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    pub key: String,
    pub label: String,
    pub current: Value,
    pub suggested: Value,
    pub unit: Option<String>,
}

/// What Suggest settings (or a preset) shows: the table of changed values and the values to apply.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Proposal {
    /// What the values were computed from (none for a preset).
    pub environment: Option<Environment>,
    /// The fields that change, in the section's order.
    pub rows: Vec<Row>,
    /// The section's values to apply (`set_settings` validates them again).
    pub values: Value,
}

/// The fields of `after` that differ from `before`, labelled by the schema.
pub fn proposal(environment: Option<Environment>, before: &PerformanceSettings, after: &PerformanceSettings) -> Proposal {
    let (b, a) = (before.to_values(), after.to_values());
    let rows = schema()
        .fields
        .iter()
        .filter(|f| b.get(&f.key) != a.get(&f.key))
        .map(|f| Row {
            key: f.key.clone(),
            label: f.label.clone(),
            current: b.get(&f.key).cloned().unwrap_or(Value::Null),
            suggested: a.get(&f.key).cloned().unwrap_or(Value::Null),
            unit: match &f.kind {
                debrief_core::settings::FieldKind::Int { unit, .. } => unit.clone(),
                _ => None,
            },
        })
        .collect();
    Proposal {
        environment,
        rows,
        values: a,
    }
}

/// This computer: its memory and cores (sysinfo), the free space where projects are kept, and the active AI model.
pub fn probe(data_dir: &Path, ai_model_bytes: u64) -> Environment {
    use debrief_ai::models::DiskSpace;
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    let mb = |b: u64| b >> 20;
    Environment {
        total_memory_mb: mb(sys.total_memory()),
        free_memory_mb: mb(sys.available_memory()),
        cores: super::section::cores(),
        disk_free_mb: debrief_ai::models::SystemDisk.available_bytes(data_dir).map(mb),
        // sysinfo tells the kind only with its disk feature, which Debrief does not build; unknown is treated as fast.
        disk_kind: None,
        ai_model_mb: mb(ai_model_bytes),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(total_gb: u64, cores: u32, ai_mb: u64) -> Environment {
        Environment {
            total_memory_mb: total_gb * 1024,
            free_memory_mb: total_gb * 1024,
            cores,
            disk_free_mb: Some(500 * 1024),
            disk_kind: Some(DiskKind::Ssd),
            ai_model_mb: ai_mb,
        }
    }

    /// (store cap, import, query, plugin, at once, parse, query threads, index threads, cores free, max spill, flush)
    type Expected = (u32, u32, u32, u32, bool, u32, u32, u32, u32, u32, u32);

    fn summary(s: &PerformanceSettings) -> Expected {
        (
            s.store_cap_mb,
            s.import_budget_mb,
            s.query_budget_mb,
            s.plugin_memory_mb,
            s.return_memory_at_once,
            s.parse_threads,
            s.query_threads,
            s.index_threads,
            s.leave_cores_free,
            s.max_spill_mb,
            s.allocator_flush_mb,
        )
    }

    #[test]
    fn suggestions_for_small_middle_and_large_machines() {
        let hdd_short = Environment {
            disk_free_mb: Some(10 * 1024),
            disk_kind: Some(DiskKind::Hdd),
            ..env(32, 16, 0)
        };
        let busy = Environment {
            free_memory_mb: 4096,
            ..env(16, 8, 0)
        };
        let table: [(&str, Environment, Expected); 5] = [
            // 8 GB, 4 cores and a 4.5 GB model: the floor everywhere, memory given back at once.
            (
                "8 GB / 4 cores with AI",
                env(8, 4, 4600),
                (1024, 384, 236, 512, true, 0, 2, 1, 1, 0, 8),
            ),
            // 16 GB, 8 cores: 12 GB to share.
            ("16 GB / 8 cores", env(16, 8, 0), (3072, 1152, 450, 3072, false, 0, 3, 2, 1, 0, 32)),
            // 32 GB, 16 cores, a spinning disk with 10 GB free: 4 parse threads, spilling capped.
            (
                "32 GB / 16 cores, HDD",
                hdd_short,
                (6144, 2304, 900, 4096, false, 4, 7, 2, 2, 2560, 32),
            ),
            // 64 GB: the store worker's suggestion stops at 8 GB.
            (
                "64 GB / 32 cores",
                env(64, 32, 0),
                (8192, 3072, 1200, 4096, false, 0, 8, 2, 2, 0, 32),
            ),
            // 16 GB but only 4 GB free now: sized by what is free.
            ("16 GB, 4 GB free", busy, (1024, 384, 236, 1024, false, 0, 3, 2, 1, 0, 32)),
        ];
        for (name, e, expected) in table {
            let s = suggest(&e, &PerformanceSettings::default());
            assert_eq!(summary(&s), expected, "{name}");
            assert!(
                super::super::section::check(&s, e.cores).is_empty(),
                "{name}: the suggestion is valid"
            );
            let (_, errors) = debrief_core::settings::sanitize(&schema(), &s.to_values());
            assert!(errors.is_empty(), "{name}: {errors:?}");
        }
    }

    #[test]
    fn the_ai_model_is_subtracted_first_and_unrelated_settings_stay() {
        assert_eq!(pool_mb(&env(16, 8, 0)), 12 * 1024);
        assert_eq!(pool_mb(&env(16, 8, 4096)), 12 * 1024 - 4915);
        assert_eq!(pool_mb(&env(4, 2, 8000)), 1024, "never below 1 GB");
        let mut current = PerformanceSettings {
            spill_folder: "/fast".into(),
            ai_worker_memory_mb: 2048,
            duck_memory_limit_mb: 500,
            ..PerformanceSettings::default()
        };
        current.plugin_memory.insert("p".into(), 300);
        let s = suggest(&env(16, 8, 0), &current);
        assert_eq!((s.spill_folder.as_str(), s.ai_worker_memory_mb), ("/fast", 2048));
        assert_eq!(s.plugin_memory["p"], 300);
        assert_eq!(s.duck_memory_limit_mb, 0, "follows the query memory");
        assert_eq!(suggest(&env(16, 1, 0), &current).leave_cores_free, 0);
    }

    #[test]
    fn the_proposal_lists_only_what_changes() {
        let before = PerformanceSettings::default();
        let after = suggest(&env(16, 8, 0), &before);
        let p = proposal(Some(env(16, 8, 0)), &before, &after);
        let keys: Vec<&str> = p.rows.iter().map(|r| r.key.as_str()).collect();
        assert_eq!(
            keys,
            ["storeCapMb", "importBudgetMb", "queryBudgetMb", "pluginMemoryMb", "queryThreads"]
        );
        let cap = &p.rows[0];
        assert_eq!(
            (cap.current.as_u64(), cap.suggested.as_u64(), cap.unit.as_deref()),
            (Some(2048), Some(3072), Some("MB"))
        );
        assert_eq!(cap.label, "Database worker memory cap");
        assert_eq!(p.values["storeCapMb"], 3072);
        assert!(proposal(None, &before, &before).rows.is_empty());
    }

    #[test]
    fn the_probe_reads_this_computer() {
        let e = probe(&std::env::temp_dir(), 3 << 20);
        assert!(e.total_memory_mb > 0 && e.cores >= 1);
        assert!(e.free_memory_mb <= e.total_memory_mb);
        assert_eq!(e.ai_model_mb, 3);
    }
}
