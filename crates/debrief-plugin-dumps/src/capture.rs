//! "Create dump of process…" (ADR 0023): `MiniDumpWriteDump` of a running process into a folder the user chose.
//! The target is only read through the documented OS API (`PROCESS_QUERY_INFORMATION | PROCESS_VM_READ`, plus
//! `PROCESS_DUP_HANDLE` for its handle table); nothing is injected or changed in it. Windows only; elsewhere every
//! operation answers "not available on this platform".

use serde::Serialize;
use std::path::{Path, PathBuf};

/// How much of the process to write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DumpType {
    /// Threads, stacks, modules, handles, unloaded modules, memory layout.
    Mini,
    /// The minidump plus the heap (private read/write memory) and data segments.
    Heap,
    /// All memory.
    Full,
}

impl DumpType {
    pub fn parse(s: &str) -> Option<DumpType> {
        match s {
            "mini" => Some(DumpType::Mini),
            "heap" => Some(DumpType::Heap),
            "full" => Some(DumpType::Full),
            _ => None,
        }
    }

    /// The `MINIDUMP_TYPE` flags.
    pub fn flags(self) -> i32 {
        const NORMAL: i32 = 0;
        const WITH_DATA_SEGS: i32 = 0x1;
        const WITH_FULL_MEMORY: i32 = 0x2;
        const WITH_HANDLE_DATA: i32 = 0x4;
        const WITH_UNLOADED_MODULES: i32 = 0x20;
        const WITH_INDIRECTLY_REFERENCED_MEMORY: i32 = 0x40;
        const WITH_PRIVATE_READ_WRITE_MEMORY: i32 = 0x200;
        const WITH_FULL_MEMORY_INFO: i32 = 0x800;
        const WITH_THREAD_INFO: i32 = 0x1000;
        let base = NORMAL | WITH_HANDLE_DATA | WITH_UNLOADED_MODULES | WITH_FULL_MEMORY_INFO | WITH_THREAD_INFO;
        match self {
            DumpType::Mini => base,
            DumpType::Heap => base | WITH_DATA_SEGS | WITH_PRIVATE_READ_WRITE_MEMORY | WITH_INDIRECTLY_REFERENCED_MEMORY,
            DumpType::Full => base | WITH_FULL_MEMORY,
        }
    }

    pub fn suffix(self) -> &'static str {
        match self {
            DumpType::Mini => "mini",
            DumpType::Heap => "heap",
            DumpType::Full => "full",
        }
    }
}

/// A running process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessRow {
    pub pid: u32,
    pub name: String,
    pub parent: u32,
    pub threads: u32,
}

/// A written dump. `file` is the tool output contract's key for a file the tool wrote (the window may open it).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Created {
    pub file: String,
    pub bytes: u64,
}

/// Why a dump could not be written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureError {
    /// Not on this platform.
    Unsupported,
    /// The process runs as another user or elevated.
    Denied(String),
    /// No such process (any more).
    NotFound(u32),
    Failed(String),
}

impl std::fmt::Display for CaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CaptureError::Unsupported => write!(f, "creating dumps is not available on this platform"),
            CaptureError::Denied(m) | CaptureError::Failed(m) => write!(f, "{m}"),
            CaptureError::NotFound(pid) => write!(f, "process {pid} is not running (any more)"),
        }
    }
}

impl std::error::Error for CaptureError {}

/// Most processes listed.
pub const MAX_PROCESSES: usize = 8192;

/// A file name for the dump: `<exe>.<pid>.<type>.<time>.dmp`, characters a path cannot hold replaced.
pub fn file_name(exe: &str, pid: u32, kind: DumpType, stamp: &str) -> String {
    let safe: String = debrief_dump::modules::file_name(exe)
        .chars()
        .map(|c| if c.is_alphanumeric() || "._-".contains(c) { c } else { '_' })
        .take(100)
        .collect();
    let safe = if safe.is_empty() { "process".to_string() } else { safe };
    format!("{safe}.{pid}.{}.{stamp}.dmp", kind.suffix())
}

/// A path in `folder` for the dump that does not exist yet.
pub fn unused_path(folder: &Path, name: &str) -> PathBuf {
    let first = folder.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = name.rsplit_once('.').unwrap_or((name, ""));
    (2..1000)
        .map(|i| folder.join(format!("{stem}-{i}.{ext}")))
        .find(|p| !p.exists())
        .unwrap_or(first)
}

#[cfg(not(windows))]
pub fn processes() -> Result<Vec<ProcessRow>, CaptureError> {
    Err(CaptureError::Unsupported)
}

#[cfg(not(windows))]
pub fn create(_pid: u32, _kind: DumpType, _folder: &Path, _stamp: &str) -> Result<Created, CaptureError> {
    Err(CaptureError::Unsupported)
}

#[cfg(windows)]
pub use win::{create, processes};

#[cfg(windows)]
mod win {
    use super::*;
    use windows::core::PWSTR;
    use windows::Win32::Foundation::{CloseHandle, ERROR_ACCESS_DENIED, ERROR_INVALID_PARAMETER, HANDLE};
    use windows::Win32::System::Diagnostics::Debug::{MiniDumpWriteDump, MINIDUMP_TYPE};
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_ACCESS_RIGHTS, PROCESS_DUP_HANDLE, PROCESS_NAME_WIN32, PROCESS_QUERY_INFORMATION,
        PROCESS_VM_READ,
    };

    /// Closes the handle when dropped.
    struct Owned(HANDLE);

    impl Drop for Owned {
        fn drop(&mut self) {
            // SAFETY: the handle was returned open by the API that created it and is closed once.
            let _ = unsafe { CloseHandle(self.0) };
        }
    }

    fn utf16_name(buf: &[u16]) -> String {
        let n = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..n])
    }

    pub fn processes() -> Result<Vec<ProcessRow>, CaptureError> {
        // SAFETY: a process snapshot; the handle is closed by `Owned`.
        let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }
            .map_err(|e| CaptureError::Failed(format!("cannot list processes: {e}")))?;
        let snap = Owned(snap);
        let mut out = Vec::new();
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        // SAFETY: `entry` is a properly sized PROCESSENTRY32W the API fills.
        let mut ok = unsafe { Process32FirstW(snap.0, &mut entry) }.is_ok();
        while ok && out.len() < MAX_PROCESSES {
            out.push(ProcessRow {
                pid: entry.th32ProcessID,
                name: utf16_name(&entry.szExeFile),
                parent: entry.th32ParentProcessID,
                threads: entry.cntThreads,
            });
            // SAFETY: as above.
            ok = unsafe { Process32NextW(snap.0, &mut entry) }.is_ok();
        }
        out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.pid.cmp(&b.pid)));
        Ok(out)
    }

    fn open(pid: u32) -> Result<(Owned, bool), CaptureError> {
        let read = PROCESS_QUERY_INFORMATION | PROCESS_VM_READ;
        let try_open = |rights: PROCESS_ACCESS_RIGHTS| {
            // SAFETY: opens a process handle with the rights asked; closed by `Owned`.
            unsafe { OpenProcess(rights, false, pid) }.map(Owned)
        };
        match try_open(read | PROCESS_DUP_HANDLE) {
            Ok(h) => Ok((h, true)),
            Err(e) if e.code() == ERROR_ACCESS_DENIED.to_hresult() => match try_open(read) {
                // Without PROCESS_DUP_HANDLE the dump has no handle table.
                Ok(h) => Ok((h, false)),
                Err(_) => Err(CaptureError::Denied(format!(
                    "Access to process {pid} was denied: it runs as another user or as administrator. Restart Debrief as administrator to dump it."
                ))),
            },
            Err(e) if e.code() == ERROR_INVALID_PARAMETER.to_hresult() => Err(CaptureError::NotFound(pid)),
            Err(e) => Err(CaptureError::Failed(format!("cannot open process {pid}: {e}"))),
        }
    }

    fn image_name(h: &Owned) -> Option<String> {
        let mut buf = vec![0u16; 1024];
        let mut len = buf.len() as u32;
        // SAFETY: `buf` holds `len` UTF-16 units; the API writes at most that many and updates `len`.
        unsafe { QueryFullProcessImageNameW(h.0, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len) }.ok()?;
        Some(String::from_utf16_lossy(&buf[..len as usize]))
    }

    pub fn create(pid: u32, kind: DumpType, folder: &Path, stamp: &str) -> Result<Created, CaptureError> {
        use std::os::windows::io::AsRawHandle;
        if pid == 0 || pid == 4 {
            return Err(CaptureError::Denied("the System process cannot be dumped".into()));
        }
        let (process, handles) = open(pid)?;
        let exe = image_name(&process).unwrap_or_else(|| "process".into());
        let path = unused_path(folder, &file_name(&exe, pid, kind, stamp));
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| CaptureError::Failed(format!("cannot create {}: {e}", path.display())))?;
        let mut flags = kind.flags();
        if !handles {
            flags &= !0x4;
        }
        // SAFETY: both handles are open for the call; the process is only read by dbghelp; no exception, user
        // stream or callback parameters are passed.
        let written = unsafe { MiniDumpWriteDump(process.0, pid, HANDLE(file.as_raw_handle()), MINIDUMP_TYPE(flags), None, None, None) };
        let bytes = file.metadata().map(|m| m.len()).unwrap_or(0);
        drop(file);
        match written {
            Ok(()) => Ok(Created {
                file: path.to_string_lossy().into_owned(),
                bytes,
            }),
            Err(e) => {
                let _ = std::fs::remove_file(&path);
                if e.code() == ERROR_ACCESS_DENIED.to_hresult() {
                    Err(CaptureError::Denied(format!(
                        "Reading process {pid} was denied. Restart Debrief as administrator to dump it."
                    )))
                } else {
                    Err(CaptureError::Failed(format!("writing the dump of process {pid} failed: {e}")))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types_names_and_paths() {
        assert_eq!(DumpType::parse("heap"), Some(DumpType::Heap));
        assert_eq!(DumpType::parse("huge"), None);
        assert_eq!(DumpType::Full.flags() & 0x2, 0x2);
        assert_eq!(DumpType::Mini.flags() & 0x202, 0);
        assert_eq!(DumpType::Heap.flags() & 0x200, 0x200);
        assert_eq!(
            file_name("C:\\a b\\Order Service.exe", 7, DumpType::Mini, "20261014-104124"),
            "Order_Service.exe.7.mini.20261014-104124.dmp"
        );
        assert_eq!(file_name("", 7, DumpType::Full, "t"), "process.7.full.t.dmp");
        let dir = tempfile::tempdir().unwrap();
        let p = unused_path(dir.path(), "a.dmp");
        assert_eq!(p, dir.path().join("a.dmp"));
        std::fs::write(&p, b"x").unwrap();
        assert_eq!(unused_path(dir.path(), "a.dmp"), dir.path().join("a-2.dmp"));
        assert_eq!(CaptureError::NotFound(3).to_string(), "process 3 is not running (any more)");
        assert!(CaptureError::Unsupported.to_string().contains("not available"));
    }

    #[cfg(not(windows))]
    #[test]
    fn other_platforms_say_not_available() {
        assert_eq!(processes().unwrap_err(), CaptureError::Unsupported);
        assert_eq!(
            create(1, DumpType::Mini, Path::new("/tmp"), "t").unwrap_err(),
            CaptureError::Unsupported
        );
    }

    /// Dumps a child process this test starts (a sleeping copy of this test binary), reads the dump back and
    /// finds its threads and modules.
    #[cfg(windows)]
    #[test]
    fn a_running_process_is_dumped_and_reads_back() {
        if std::env::var_os("DEBRIEF_DUMP_FIXTURE").is_some() {
            std::thread::sleep(std::time::Duration::from_secs(30));
            return;
        }
        #[allow(clippy::disallowed_methods)]
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "capture::tests::a_running_process_is_dumped_and_reads_back",
                "--nocapture",
            ])
            .env("DEBRIEF_DUMP_FIXTURE", "1")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(500));
        let dir = tempfile::tempdir().unwrap();
        let listed = processes().unwrap();
        assert!(listed.iter().any(|p| p.pid == child.id()));
        let made = create(child.id(), DumpType::Mini, dir.path(), "test");
        let full = create(child.id(), DumpType::Full, dir.path(), "test");
        let pid = child.id();
        let _ = child.kill();
        let _ = child.wait();
        drop(child);
        let made = made.unwrap();
        assert!(made.bytes > 0 && made.file.ends_with(".mini.test.dmp"));
        let bytes = std::fs::read(&made.file).unwrap();
        let d = crate::dump_reader::read_dump(&bytes, &debrief_dump::Limits::default()).unwrap();
        assert!(d.exception.is_none());
        assert!(!d.threads.is_empty() && d.threads.iter().all(|t| !t.frames.is_empty()));
        assert!(d.modules.iter().any(|m| m.name().eq_ignore_ascii_case("ntdll.dll")));
        assert!(d.handles.as_ref().is_some_and(|h| h.total > 0));
        assert_eq!(d.process.as_ref().and_then(|p| p.pid), Some(pid));
        // With the folder of this binary's PDB as a symbol folder, its frames get names.
        let exe_dir = std::env::current_exe().unwrap().parent().unwrap().to_path_buf();
        let opts = debrief_dump::ImportOptions::default();
        if let debrief_dump::Document::Dump(named) = crate::import::import_dump(&bytes, &opts, &[exe_dir]).unwrap() {
            let ours = named
                .modules
                .iter()
                .position(|m| m.name().starts_with("debrief_plugin_dumps"))
                .unwrap() as u32;
            assert!(
                named.modules[ours as usize].symbols.is_loaded(),
                "{:?}",
                named.modules[ours as usize].symbols
            );
            assert!(named
                .threads
                .iter()
                .flat_map(|t| &t.frames)
                .any(|f| f.module == Some(ours) && f.function.is_some()));
        }
        let full = full.unwrap();
        let src = std::fs::read(&full.file).unwrap();
        let f = crate::dump_reader::read_dump(&src, &debrief_dump::Limits::default()).unwrap();
        assert_eq!(f.kind, debrief_dump::model::DumpKind::Full);
        assert!(f.threads.iter().any(|t| t.frames.len() > 1));
        assert_eq!(
            create(pid, DumpType::Mini, dir.path(), "gone").unwrap_err(),
            CaptureError::NotFound(pid)
        );
    }
}
