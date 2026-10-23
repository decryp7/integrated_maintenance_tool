//! What the runtime manager shows: every build the pinned catalog offers for this platform, whether it
//! is installed, whether it works here (from the worker's report) and which one is suggested and in use.
//! A pure merge of the catalog, the store and the report.

use super::builds::{label, try_order, BuildChoice};
use super::catalog::RuntimeCatalog;
use super::report::{BuildsReport, Device};
use super::store::InstalledRuntime;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeEntry {
    pub id: String,
    pub label: String,
    pub download_bytes: u64,
    pub installed: bool,
    pub installed_bytes: Option<u64>,
    /// `None` when not installed or not checked yet.
    pub usable: Option<bool>,
    pub problem: Option<String>,
    pub devices: Vec<Device>,
    /// The build suggested for this machine.
    pub suggested: bool,
    /// The build that runs the model with the current settings.
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeOverview {
    pub release: String,
    pub tag: String,
    pub platform: String,
    /// The `inference_build` setting (`auto` or an id).
    pub choice: String,
    pub suggested: Option<String>,
    /// The build in use: the chosen one when it works, else the best working one.
    pub active: Option<String>,
    /// Set when a custom llama-server path is configured (it replaces the downloaded builds).
    pub custom_path: Option<String>,
    pub builds: Vec<RuntimeEntry>,
    /// Builds of older releases still on disk (they can be deleted).
    pub outdated: Vec<InstalledRuntime>,
    /// No build is installed (and no custom path is set): the setup should offer a download.
    pub needs_download: bool,
    /// Why usability could not be checked, if it could not.
    pub problem: Option<String>,
}

pub struct OverviewInput<'a> {
    pub catalog: &'a RuntimeCatalog,
    pub platform: &'a str,
    pub installed: &'a [InstalledRuntime],
    pub report: Option<&'a BuildsReport>,
    pub report_problem: Option<String>,
    pub suggested: Option<String>,
    pub choice: &'a BuildChoice,
    pub custom_path: Option<String>,
}

pub fn overview(i: &OverviewInput<'_>) -> RuntimeOverview {
    let current: Vec<&InstalledRuntime> = i.installed.iter().filter(|r| r.tag == i.catalog.tag).collect();
    let probe = |id: &str| i.report.and_then(|r| r.builds.iter().find(|b| b.id == id));
    let usable: Vec<String> = current
        .iter()
        .filter(|r| probe(&r.id).map(|p| p.usable).unwrap_or(true))
        .map(|r| r.id.clone())
        .collect();
    let active = if i.custom_path.is_some() {
        None
    } else {
        try_order(&usable, i.choice).into_iter().next()
    };
    let builds = i
        .catalog
        .builds(i.platform)
        .iter()
        .map(|b| {
            let inst = current.iter().find(|r| r.id == b.id);
            let p = inst.and_then(|_| probe(&b.id));
            RuntimeEntry {
                id: b.id.clone(),
                label: label(&b.id),
                download_bytes: b.download_bytes(),
                installed: inst.is_some(),
                installed_bytes: inst.map(|r| r.bytes),
                usable: p.map(|p| p.usable),
                problem: p.and_then(|p| p.problem.clone()),
                devices: p.map(|p| p.devices.clone()).unwrap_or_default(),
                suggested: i.suggested.as_deref() == Some(b.id.as_str()),
                active: active.as_deref() == Some(b.id.as_str()),
            }
        })
        .collect();
    RuntimeOverview {
        release: i.catalog.release.clone(),
        tag: i.catalog.tag.clone(),
        platform: i.platform.to_string(),
        choice: i.choice.as_setting().to_string(),
        suggested: i.suggested.clone(),
        active,
        needs_download: current.is_empty() && i.custom_path.is_none(),
        custom_path: i.custom_path.clone(),
        builds,
        outdated: i.installed.iter().filter(|r| r.tag != i.catalog.tag).cloned().collect(),
        problem: i.report_problem.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtimes::report::BuildInfo;

    fn installed(tag: &str, id: &str) -> InstalledRuntime {
        InstalledRuntime {
            id: id.into(),
            tag: tag.into(),
            dir: Default::default(),
            bytes: 100,
            current: false,
        }
    }

    fn info(id: &str, usable: bool) -> BuildInfo {
        BuildInfo {
            id: id.into(),
            label: label(id),
            usable,
            problem: (!usable).then(|| "no supported GPU was found".into()),
            devices: vec![],
        }
    }

    #[test]
    fn merges_catalog_store_and_report() {
        let c = RuntimeCatalog::embedded();
        let tag = c.tag.clone();
        let inst = vec![installed(&tag, "cpu"), installed(&tag, "cuda-13"), installed("b1", "vulkan")];
        let report = BuildsReport {
            release: Some(tag.clone()),
            source: "installed".into(),
            choice: "auto".into(),
            auto: Some("cpu".into()),
            builds: vec![info("cpu", true), info("cuda-13", false)],
            problem: None,
        };
        let o = overview(&OverviewInput {
            catalog: &c,
            platform: "windows-x64",
            installed: &inst,
            report: Some(&report),
            report_problem: None,
            suggested: Some("cuda-13".into()),
            choice: &BuildChoice::Auto,
            custom_path: None,
        });
        assert_eq!(o.active.as_deref(), Some("cpu"), "CUDA does not work here, so the CPU build runs");
        let cuda = o.builds.iter().find(|b| b.id == "cuda-13").unwrap();
        assert!(cuda.installed && cuda.suggested && !cuda.active);
        assert_eq!(cuda.usable, Some(false));
        assert!(cuda.problem.is_some());
        let vulkan = o.builds.iter().find(|b| b.id == "vulkan").unwrap();
        assert!(!vulkan.installed && vulkan.usable.is_none() && vulkan.download_bytes > 0);
        assert_eq!(o.outdated.len(), 1);
        assert!(!o.needs_download);
        let v = serde_json::to_value(&o).unwrap();
        assert!(v["builds"][0].get("downloadBytes").is_some() && v.get("needsDownload").is_some());
    }

    #[test]
    fn nothing_installed_needs_a_download_unless_a_custom_path_is_set() {
        let c = RuntimeCatalog::embedded();
        let mut i = OverviewInput {
            catalog: &c,
            platform: "linux-x64",
            installed: &[],
            report: None,
            report_problem: Some("x".into()),
            suggested: None,
            choice: &BuildChoice::Fixed("vulkan".into()),
            custom_path: None,
        };
        let o = overview(&i);
        assert!(o.needs_download && o.active.is_none());
        assert_eq!(o.choice, "vulkan");
        i.custom_path = Some("/opt/llama-server".into());
        assert!(!overview(&i).needs_download);
        // Unchecked installs count as usable.
        let inst = vec![installed(&c.tag, "vulkan")];
        i.installed = &inst;
        i.custom_path = None;
        assert_eq!(overview(&i).active.as_deref(), Some("vulkan"));
    }
}
