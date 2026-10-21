//! The plugin adapters (ADR 0023). Two plugins, so each process gets only the rights it needs:
//!
//! - [`DumpsPlugin`] (`net.decryptology.dumps`): the readers. Untrusted, possibly huge files are parsed in its
//!   locked-down process (no network, no writes, low integrity, a 3 GiB memory cap); the only right it asks for
//!   is reading the symbol folders the user configures in its settings.
//! - [`DumpCapturePlugin`] (`net.decryptology.dumps.capture`): the `dumps.capture` tool (`processes`, `create`),
//!   which needs normal integrity to open another process for reading and to write into the folder the user
//!   chose (checked against the window's grants by the host before the tool sees it).

use crate::capture::{self, CaptureError, DumpType};
use crate::import::{import_dump, import_umdh};
use crate::source::Source;
use anyhow::{anyhow, Result};
use imt_dump::{DumpError, ImportOptions, Limits};
use imt_plugin_api::manifest::{Permission, PermissionRequest};
use imt_plugin_api::settings::{Configurable, FieldKind, SectionOwner, SettingField, SettingsSection, ValidValues};
use imt_plugin_api::tools::{OpSpec, PathAccess, PathKind, Platform, Schema, Tool, ToolCtx, ToolFailure, ToolInfo};
use imt_plugin_api::{Contributions, DocumentImporter, Plugin, RangeRead};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;

/// Symbol folders kept.
pub const MAX_SYMBOL_FOLDERS: usize = 16;
/// Memory of the readers' process.
pub const READER_MEMORY: u64 = 3 << 30;

/// The host's file as a [`Source`].
struct Ranges<'a>(&'a dyn RangeRead);

impl Source for Ranges<'_> {
    fn len(&self) -> u64 {
        self.0.len()
    }

    fn read(&self, offset: u64, len: usize) -> std::result::Result<Vec<u8>, String> {
        self.0
            .read_at(offset, u32::try_from(len).unwrap_or(u32::MAX))
            .map_err(|e| format!("{e:#}"))
    }
}

/// The symbol folders, shared by the settings and the importers.
#[derive(Default)]
pub struct SymbolFolders(RwLock<Vec<PathBuf>>);

impl SymbolFolders {
    pub fn get(&self) -> Vec<PathBuf> {
        self.0.read().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub fn set(&self, folders: Vec<PathBuf>) {
        *self.0.write().unwrap_or_else(|p| p.into_inner()) = folders;
    }
}

fn options(v: &Value) -> ImportOptions {
    serde_json::from_value(v.clone()).unwrap_or(ImportOptions {
        limits: Limits::default(),
        file_time_ms: None,
    })
}

/// Reads minidumps (`MDMP`, any extension).
pub struct MinidumpImporter {
    pub folders: Arc<SymbolFolders>,
}

impl DocumentImporter for MinidumpImporter {
    fn id(&self) -> &str {
        "minidump"
    }

    fn produces(&self) -> &str {
        "dump"
    }

    fn sniff(&self, _file_name: &str, head: &[u8]) -> u8 {
        if head.starts_with(b"MDMP") {
            100
        } else {
            0
        }
    }

    fn import_encoded(&self, _file_name: &str, file: &dyn RangeRead, options: &Value) -> Vec<u8> {
        let o = self::options(options);
        imt_dump::wire::encode_result(&import_dump(&Ranges(file), &o, &self.folders.get()))
    }
}

/// Reads UMDH snapshots and diff logs.
pub struct UmdhImporter {
    pub folders: Arc<SymbolFolders>,
}

impl DocumentImporter for UmdhImporter {
    fn id(&self) -> &str {
        "umdh"
    }

    fn produces(&self) -> &str {
        "umdh"
    }

    fn sniff(&self, _file_name: &str, head: &[u8]) -> u8 {
        crate::umdh_reader::sniff(head)
    }

    fn import_encoded(&self, _file_name: &str, file: &dyn RangeRead, options: &Value) -> Vec<u8> {
        let o = self::options(options);
        imt_dump::wire::encode_result(&import_umdh(&Ranges(file), &o, &self.folders.get()))
    }
}

/// The readers' settings: symbol folders.
pub struct DumpsSettings(pub Arc<SymbolFolders>);

impl Configurable for DumpsSettings {
    fn settings_section(&self) -> SettingsSection {
        SettingsSection::new("dumps", "Dumps", SectionOwner::Core)
            .describe("Crash and memory dumps, UMDH heap logs.")
            .field(
                SettingField::new(
                    "symbolFolders",
                    "Symbol folders",
                    FieldKind::StringList {
                        max_items: MAX_SYMBOL_FOLDERS,
                    },
                    json!([]),
                )
                .describe("Folders with PDB files (flat or in the symbol store layout) used to name the frames of dumps and UMDH logs. Absolute paths; nothing is downloaded."),
            )
    }

    fn apply_settings(&self, values: &ValidValues) -> Result<()> {
        let list = values
            .get("symbolFolders")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("missing setting symbolFolders"))?;
        let folders: Vec<PathBuf> = list
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .take(MAX_SYMBOL_FOLDERS)
            .collect();
        self.0.set(folders);
        Ok(())
    }
}

/// The dump and UMDH readers.
#[derive(Default)]
pub struct DumpsPlugin {
    folders: Arc<SymbolFolders>,
}

impl DumpsPlugin {
    pub fn new() -> DumpsPlugin {
        DumpsPlugin::default()
    }
}

impl Plugin for DumpsPlugin {
    fn id(&self) -> &str {
        "net.decryptology.dumps"
    }

    fn name(&self) -> &str {
        "Dumps"
    }

    fn version(&self) -> &str {
        env!("CARGO_PKG_VERSION")
    }

    fn register(&self, c: &mut Contributions) -> Result<()> {
        c.add_document_importer(MinidumpImporter {
            folders: self.folders.clone(),
        })
        .add_document_importer(UmdhImporter {
            folders: self.folders.clone(),
        })
        .set_settings(Arc::new(DumpsSettings(self.folders.clone())));
        Ok(())
    }

    fn permissions(&self) -> Vec<PermissionRequest> {
        vec![PermissionRequest {
            permission: Permission::ReadFolders,
            reason: "read PDB files in the symbol folders you configure".into(),
        }]
    }

    fn memory_limit(&self) -> Option<u64> {
        Some(READER_MEMORY)
    }
}

/// `YYYYMMDD-HHMMSS` (UTC) of `secs` since the Unix epoch.
pub fn stamp(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}{m:02}{d:02}-{:02}{:02}{:02}", rem / 3600, rem / 60 % 60, rem % 60)
}

/// The Create dump tool.
pub struct CaptureTool;

const TOOL: &str = "dumps.capture";

fn capture_failure(e: CaptureError) -> anyhow::Error {
    match e {
        CaptureError::Denied(m) => anyhow::Error::new(ToolFailure::denied(m)),
        other => anyhow!(other.to_string()),
    }
}

impl Tool for CaptureTool {
    fn id(&self) -> &str {
        TOOL
    }

    fn describe(&self) -> ToolInfo {
        ToolInfo::new(TOOL, "Create dump of process")
            .describe("Writes a minidump, a minidump with heap or a full dump of a running process with MiniDumpWriteDump. The process is only read; nothing is injected into it.")
            .only_on(&[Platform::Windows])
            .op(OpSpec::new("processes", "List processes"))
            .op(OpSpec::new("create", "Create dump")
                .args(
                    Schema::object()
                        .field("pid", Schema::int(1, i64::from(u32::MAX)))
                        .field("kind", Schema::one_of(["mini", "heap", "full"]))
                        .field("folder", Schema::path(PathKind::Dir, PathAccess::Write)),
                )
                .budget(Some(Duration::from_secs(1800))))
    }

    fn invoke(&self, op: &str, args: Value, _ctx: &ToolCtx) -> Result<Value> {
        match op {
            "processes" => Ok(json!(capture::processes().map_err(capture_failure)?)),
            "create" => {
                let pid = args
                    .get("pid")
                    .and_then(Value::as_u64)
                    .and_then(|p| u32::try_from(p).ok())
                    .ok_or_else(|| anyhow!("pid must be a process id"))?;
                let kind = args
                    .get("kind")
                    .and_then(Value::as_str)
                    .and_then(DumpType::parse)
                    .ok_or_else(|| anyhow!("kind must be mini, heap or full"))?;
                let folder = args
                    .get("folder")
                    .and_then(Value::as_str)
                    .map(Path::new)
                    .ok_or_else(|| anyhow!("folder is missing"))?;
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                Ok(json!(capture::create(pid, kind, folder, &stamp(now)).map_err(capture_failure)?))
            }
            other => Err(anyhow!("tool {TOOL} has no operation \"{other}\"")),
        }
    }
}

/// The capture tool's plugin.
pub struct DumpCapturePlugin;

impl Plugin for DumpCapturePlugin {
    fn id(&self) -> &str {
        "net.decryptology.dumps.capture"
    }

    fn name(&self) -> &str {
        "Create dumps"
    }

    fn version(&self) -> &str {
        env!("CARGO_PKG_VERSION")
    }

    fn register(&self, c: &mut Contributions) -> Result<()> {
        c.add_tool(Arc::new(CaptureTool));
        Ok(())
    }

    fn permissions(&self) -> Vec<PermissionRequest> {
        vec![PermissionRequest {
            permission: Permission::WriteFolders,
            reason: "open a process for reading and write its dump into the folder you choose".into(),
        }]
    }

    fn memory_limit(&self) -> Option<u64> {
        Some(256 << 20)
    }
}

/// An engine error as the plugin reports it (for tests and logs).
pub fn describe(e: &DumpError) -> String {
    format!("{:?}: {}", e.code, e.message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use imt_dump::model::Document;
    use imt_plugin_api::settings::validate;

    #[test]
    fn manifests_describe_both_plugins() {
        let p = DumpsPlugin::new();
        let mut c = Contributions::default();
        p.register(&mut c).unwrap();
        let m = c.manifest(&p);
        m.check().unwrap();
        assert_eq!(m.document_importers.len(), 2);
        assert!(m.asks_for(Permission::ReadFolders) && !m.asks_for(Permission::WriteFolders));
        assert_eq!(m.memory_limit, Some(READER_MEMORY));
        assert!(m.settings.is_some());
        let cap = DumpCapturePlugin;
        let mut c = Contributions::default();
        cap.register(&mut c).unwrap();
        let m = c.manifest(&cap);
        m.check().unwrap();
        assert_eq!(m.tools[0].id, TOOL);
        assert!(m.tools[0].unsupported_on(Platform::Linux).is_some());
        assert!(m.asks_for(Permission::WriteFolders));
    }

    #[test]
    fn importers_encode_documents_and_errors() {
        let f = Arc::new(SymbolFolders::default());
        let dump = MinidumpImporter { folders: f.clone() };
        let bytes = crate::synth::sample_crash();
        assert_eq!(dump.sniff("x", &bytes), 100);
        assert_eq!(dump.sniff("x", b"nope"), 0);
        let opts = serde_json::to_value(ImportOptions {
            limits: Limits::default(),
            file_time_ms: None,
        })
        .unwrap();
        let enc = dump.import_encoded("x.dmp", &bytes.as_slice(), &opts);
        let doc = imt_dump::wire::decode_result(&enc, &Limits::default()).unwrap();
        assert!(matches!(doc, Document::Dump(d) if d.threads.len() == 3));
        let bad = dump.import_encoded("x.dmp", &&b"MDMPxxxx"[..], &json!("not options"));
        let e = imt_dump::wire::decode_result(&bad, &Limits::default()).unwrap_err();
        assert!(describe(&e).contains("NotADump"), "{e}");
        let umdh = UmdhImporter { folders: f };
        let text = crate::umdh_reader::tests::SNAPSHOT.as_bytes();
        assert!(umdh.sniff("x.log", text) >= 90);
        let enc = umdh.import_encoded("x.log", &text, &opts);
        assert!(matches!(
            imt_dump::wire::decode_result(&enc, &Limits::default()).unwrap(),
            Document::Umdh(_)
        ));
    }

    #[test]
    fn symbol_folders_apply_absolute_paths_only() {
        let f = Arc::new(SymbolFolders::default());
        let s = DumpsSettings(f.clone());
        let section = s.settings_section();
        let abs = if cfg!(windows) { "C:\\symbols" } else { "/symbols" };
        let v = validate(&section, &json!({ "symbolFolders": [abs, "relative", "  "] })).unwrap();
        s.apply_settings(&v).unwrap();
        assert_eq!(f.get(), [PathBuf::from(abs)]);
    }

    #[test]
    fn the_tool_validates_and_stamps() {
        assert_eq!(stamp(1_791_974_484), "20261014-104124");
        assert_eq!(stamp(0), "19700101-000000");
        let ctx = ToolCtx::new(None, Arc::new(imt_plugin_api::tools::NoTasks));
        let t = CaptureTool;
        assert!(t.invoke("nope", json!({}), &ctx).is_err());
        assert!(t
            .invoke("create", json!({ "pid": 1, "kind": "huge", "folder": "/x" }), &ctx)
            .is_err());
        assert!(t.invoke("create", json!({ "kind": "mini" }), &ctx).is_err());
        #[cfg(not(windows))]
        assert!(t
            .invoke("processes", json!({}), &ctx)
            .unwrap_err()
            .to_string()
            .contains("not available"));
        #[cfg(windows)]
        assert!(t
            .invoke("processes", json!({}), &ctx)
            .unwrap()
            .as_array()
            .is_some_and(|a| !a.is_empty()));
        let denied = capture_failure(CaptureError::Denied("no".into()));
        assert_eq!(
            imt_plugin_api::tools::failure_reason(&denied),
            Some(imt_plugin_api::tools::FailureReason::Denied)
        );
    }
}
