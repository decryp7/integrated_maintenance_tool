//! Finding .NET processes: which runtime each runs (.NET Framework through
//! `clr.dll`, .NET Core through `coreclr.dll` or its diagnostics port), its
//! bitness and whether recording it needs elevation. Only process and module
//! lists are read; nothing is injected or loaded into the targets.

use serde::Serialize;

/// Which .NET runtime a process runs (decides the backend).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Runtime {
    /// .NET Framework 4.x (`clr.dll`): ETW.
    Framework,
    /// .NET Core 3+ / .NET 5+ (`coreclr.dll`, diagnostics port): EventPipe.
    Core,
}

/// A process that can be recorded (`DotnetProcess` on the wire).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub command_line: Option<String>,
    pub runtime_version: Option<String>,
    /// `x86`, `x64`, `arm64` (when known).
    pub arch: Option<String>,
    pub runtime: Runtime,
    /// Recording it needs administrator rights that IMT does not have (ETW).
    pub needs_elevation: bool,
}

impl ProcessInfo {
    /// Pointer size of the target (4 for x86, else 8).
    pub fn pointer_size(&self) -> u32 {
        if self.arch.as_deref() == Some("x86") {
            4
        } else {
            8
        }
    }
}

/// The runtime a loaded module means, with a version hint from its path.
pub fn runtime_of_module(module: &str, path: &str) -> Option<(Runtime, Option<String>)> {
    match module.to_ascii_lowercase().as_str() {
        "clr.dll" => Some((Runtime::Framework, Some(framework_version(path)))),
        "coreclr.dll" | "libcoreclr.so" | "libcoreclr.dylib" => Some((Runtime::Core, core_version(path))),
        _ => None,
    }
}

fn framework_version(path: &str) -> String {
    let p = path.to_ascii_lowercase();
    if p.contains("framework64") {
        ".NET Framework 4 (64-bit)".to_string()
    } else if p.contains("framework") {
        ".NET Framework 4 (32-bit)".to_string()
    } else {
        ".NET Framework 4".to_string()
    }
}

/// `...\Microsoft.NETCore.App\8.0.11\coreclr.dll` → `.NET 8.0.11`.
fn core_version(path: &str) -> Option<String> {
    let parts: Vec<&str> = path.split(['\\', '/']).collect();
    let i = parts.iter().position(|p| p.eq_ignore_ascii_case("Microsoft.NETCore.App"))?;
    let v = parts.get(i + 1)?;
    v.chars().next().filter(char::is_ascii_digit).map(|_| format!(".NET {v}"))
}

/// The process id of a diagnostics port name: `dotnet-diagnostic-1234` (Windows pipe) or
/// `dotnet-diagnostic-1234-5678-socket` (Unix socket).
pub fn pid_of_port(name: &str) -> Option<u32> {
    let rest = name.strip_prefix("dotnet-diagnostic-")?;
    let pid = rest.split('-').next()?;
    if rest.contains('-') && !rest.ends_with("-socket") {
        return None;
    }
    pid.parse().ok().filter(|&p| p > 0)
}

/// Every .NET process (never fails: what cannot be inspected is skipped).
pub fn list() -> Vec<ProcessInfo> {
    let mut out = sys::list();
    out.sort_by(|a, b| {
        a.name
            .to_ascii_lowercase()
            .cmp(&b.name.to_ascii_lowercase())
            .then(a.pid.cmp(&b.pid))
    });
    out
}

/// One process, when it runs .NET.
pub fn info(pid: u32) -> Option<ProcessInfo> {
    sys::info(pid)
}

#[cfg(windows)]
mod sys {
    use super::{pid_of_port, runtime_of_module, ProcessInfo, Runtime};
    use crate::eventpipe;
    use std::collections::HashSet;
    use std::time::Duration;
    use windows::core::w;
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
    use windows::Win32::Storage::FileSystem::{FindClose, FindFirstFileW, FindNextFileW, WIN32_FIND_DATAW};
    use windows::Win32::System::Diagnostics::ToolHelp::*;
    use windows::Win32::System::SystemInformation::{
        IMAGE_FILE_MACHINE, IMAGE_FILE_MACHINE_AMD64, IMAGE_FILE_MACHINE_ARM64, IMAGE_FILE_MACHINE_I386,
    };
    use windows::Win32::System::Threading::{
        GetCurrentProcess, IsWow64Process2, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    struct Handle(HANDLE);
    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: a handle we opened.
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    fn wide_str(w: &[u16]) -> String {
        let n = w.iter().position(|&c| c == 0).unwrap_or(w.len());
        String::from_utf16_lossy(&w[..n])
    }

    /// Process ids with a diagnostics pipe (`\\.\pipe\dotnet-diagnostic-<pid>`).
    fn diagnostic_pipes() -> HashSet<u32> {
        let mut out = HashSet::new();
        let mut data = WIN32_FIND_DATAW::default();
        // SAFETY: data is a valid out buffer; the find handle is closed below.
        let Ok(h) = (unsafe { FindFirstFileW(w!(r"\\.\pipe\*"), &mut data) }) else {
            return out;
        };
        loop {
            if let Some(pid) = pid_of_port(&wide_str(&data.cFileName)) {
                out.insert(pid);
            }
            // SAFETY: valid handle and buffer.
            if unsafe { FindNextFileW(h, &mut data) }.is_err() || out.len() > 100_000 {
                break;
            }
        }
        // SAFETY: the handle from FindFirstFileW.
        unsafe {
            let _ = FindClose(h);
        }
        out
    }

    /// (pid, exe name) of every process.
    fn processes() -> Vec<(u32, String)> {
        let mut out = Vec::new();
        // SAFETY: snapshot handle closed by `Handle`.
        let Ok(snap) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else {
            return out;
        };
        let snap = Handle(snap);
        let mut e = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        // SAFETY: valid snapshot and entry with dwSize set.
        let mut ok = unsafe { Process32FirstW(snap.0, &mut e) }.is_ok();
        while ok {
            out.push((e.th32ProcessID, wide_str(&e.szExeFile)));
            ok = unsafe { Process32NextW(snap.0, &mut e) }.is_ok();
        }
        out
    }

    /// The runtime loaded in `pid` (Err: its modules cannot be read, e.g. missing rights).
    fn runtime(pid: u32) -> Result<Option<(Runtime, Option<String>)>, ()> {
        // SAFETY: snapshot handle closed by `Handle`.
        let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid) }.map_err(|_| ())?;
        let snap = Handle(snap);
        let mut e = MODULEENTRY32W {
            dwSize: std::mem::size_of::<MODULEENTRY32W>() as u32,
            ..Default::default()
        };
        let mut ok = unsafe { Module32FirstW(snap.0, &mut e) }.is_ok();
        while ok {
            if let Some(r) = runtime_of_module(&wide_str(&e.szModule), &wide_str(&e.szExePath)) {
                return Ok(Some(r));
            }
            ok = unsafe { Module32NextW(snap.0, &mut e) }.is_ok();
        }
        Ok(None)
    }

    fn arch(pid: u32) -> Option<String> {
        // SAFETY: handle closed by `Handle`.
        let h = Handle(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?);
        let mut process = IMAGE_FILE_MACHINE::default();
        let mut native = IMAGE_FILE_MACHINE::default();
        unsafe { IsWow64Process2(h.0, &mut process, Some(&mut native)) }.ok()?;
        let machine = if process.0 == 0 { native } else { process };
        Some(
            match machine {
                IMAGE_FILE_MACHINE_I386 => "x86",
                IMAGE_FILE_MACHINE_AMD64 => "x64",
                IMAGE_FILE_MACHINE_ARM64 => "arm64",
                _ => return None,
            }
            .to_string(),
        )
    }

    /// Whether IMT runs elevated (ETW sessions need it).
    pub(crate) fn elevated() -> bool {
        let mut token = HANDLE::default();
        // SAFETY: the current process pseudo-handle; token closed by `Handle`.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }.is_err() {
            return false;
        }
        let token = Handle(token);
        let mut e = TOKEN_ELEVATION::default();
        let mut len = 0u32;
        let ok = unsafe {
            GetTokenInformation(
                token.0,
                TokenElevation,
                Some(&mut e as *mut _ as *mut core::ffi::c_void),
                std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                &mut len,
            )
        };
        ok.is_ok() && e.TokenIsElevated != 0
    }

    fn describe(pid: u32, name: String, pipes: &HashSet<u32>, elevated: bool) -> Option<ProcessInfo> {
        let (runtime, version) = match runtime(pid) {
            Ok(Some(r)) => r,
            Ok(None) if !pipes.contains(&pid) => return None,
            Ok(None) | Err(()) if pipes.contains(&pid) => (Runtime::Core, None),
            _ => return None,
        };
        let mut info = ProcessInfo {
            pid,
            name,
            command_line: None,
            runtime_version: version,
            arch: arch(pid),
            runtime,
            needs_elevation: runtime == Runtime::Framework && !elevated,
        };
        if runtime == Runtime::Core {
            if let Ok(p) = eventpipe::process_info(pid, Duration::from_millis(500)) {
                info.command_line = Some(p.command_line).filter(|c| !c.is_empty());
                if let Some(v) = p.runtime_version.filter(|v| !v.is_empty()) {
                    info.runtime_version = Some(format!(".NET {v}"));
                }
                if info.arch.is_none() && !p.arch.is_empty() {
                    info.arch = Some(p.arch.to_ascii_lowercase());
                }
            }
        }
        Some(info)
    }

    pub(super) fn list() -> Vec<ProcessInfo> {
        let pipes = diagnostic_pipes();
        let elevated = elevated();
        let me = std::process::id();
        processes()
            .into_iter()
            .filter(|(pid, _)| *pid > 4 && *pid != me)
            .filter_map(|(pid, name)| describe(pid, name, &pipes, elevated))
            .collect()
    }

    pub(super) fn info(pid: u32) -> Option<ProcessInfo> {
        let name = processes().into_iter().find(|(p, _)| *p == pid)?.1;
        describe(pid, name, &diagnostic_pipes(), elevated())
    }

    /// Thread ids of `pid` (seeds the ETW session before its thread rundown arrives).
    pub(crate) fn threads(pid: u32) -> Vec<u32> {
        let mut out = Vec::new();
        // SAFETY: snapshot closed by `Handle`.
        let Ok(snap) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) }) else {
            return out;
        };
        let snap = Handle(snap);
        let mut e = THREADENTRY32 {
            dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
            ..Default::default()
        };
        let mut ok = unsafe { Thread32First(snap.0, &mut e) }.is_ok();
        while ok {
            if e.th32OwnerProcessID == pid {
                out.push(e.th32ThreadID);
            }
            ok = unsafe { Thread32Next(snap.0, &mut e) }.is_ok();
        }
        out
    }
}

#[cfg(not(windows))]
mod sys {
    use super::{pid_of_port, ProcessInfo, Runtime};
    use crate::eventpipe;
    use std::time::Duration;

    pub(super) fn list() -> Vec<ProcessInfo> {
        let dir = std::env::var_os("TMPDIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut pids: Vec<u32> = entries
            .flatten()
            .take(100_000)
            .filter_map(|e| pid_of_port(&e.file_name().to_string_lossy()))
            .collect();
        pids.sort_unstable();
        pids.dedup();
        pids.into_iter().filter(|&p| p != std::process::id()).filter_map(info).collect()
    }

    pub(super) fn info(pid: u32) -> Option<ProcessInfo> {
        let p = eventpipe::process_info(pid, Duration::from_millis(500)).ok()?;
        let name = std::fs::read_to_string(format!("/proc/{pid}/comm"))
            .ok()
            .map(|s| s.trim().to_string())
            .or_else(|| {
                p.command_line
                    .split_whitespace()
                    .next()
                    .map(|c| c.rsplit('/').next().unwrap_or(c).to_string())
            })
            .unwrap_or_else(|| format!("Process {pid}"));
        Some(ProcessInfo {
            pid,
            name,
            command_line: Some(p.command_line).filter(|c| !c.is_empty()),
            runtime_version: p.runtime_version.map(|v| format!(".NET {v}")),
            arch: Some(p.arch.to_ascii_lowercase()).filter(|a| !a.is_empty()),
            runtime: Runtime::Core,
            needs_elevation: false,
        })
    }
}

#[cfg(windows)]
pub(crate) use sys::{elevated, threads};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_runtime_modules() {
        let (r, v) = runtime_of_module("CLR.DLL", r"C:\Windows\Microsoft.NET\Framework64\v4.0.30319\clr.dll").unwrap();
        assert_eq!(r, Runtime::Framework);
        assert_eq!(v.as_deref(), Some(".NET Framework 4 (64-bit)"));
        let (_, v) = runtime_of_module("clr.dll", r"C:\Windows\Microsoft.NET\Framework\v4.0.30319\clr.dll").unwrap();
        assert_eq!(v.as_deref(), Some(".NET Framework 4 (32-bit)"));
        assert_eq!(runtime_of_module("clr.dll", "x").unwrap().1.as_deref(), Some(".NET Framework 4"));
        let (r, v) = runtime_of_module(
            "coreclr.dll",
            r"C:\Program Files\dotnet\shared\Microsoft.NETCore.App\10.0.1\coreclr.dll",
        )
        .unwrap();
        assert_eq!(r, Runtime::Core);
        assert_eq!(v.as_deref(), Some(".NET 10.0.1"));
        assert_eq!(runtime_of_module("coreclr.dll", r"C:\app\coreclr.dll").unwrap().1, None);
        assert_eq!(
            runtime_of_module("coreclr.dll", r"C:\x\Microsoft.NETCore.App\abc\coreclr.dll")
                .unwrap()
                .1,
            None
        );
        assert!(runtime_of_module("kernel32.dll", "").is_none());
    }

    #[test]
    fn parses_diagnostic_port_names() {
        assert_eq!(pid_of_port("dotnet-diagnostic-1234"), Some(1234));
        assert_eq!(pid_of_port("dotnet-diagnostic-1234-99887766-socket"), Some(1234));
        assert_eq!(pid_of_port("dotnet-diagnostic-0"), None);
        assert_eq!(pid_of_port("dotnet-diagnostic-12-x"), None);
        assert_eq!(pid_of_port("dotnet-diagnostic-abc"), None);
        assert_eq!(pid_of_port("other-1234"), None);
    }

    #[test]
    fn pointer_size_follows_arch_and_listing_never_fails() {
        let mut p = ProcessInfo {
            pid: 1,
            name: "a".into(),
            command_line: None,
            runtime_version: None,
            arch: Some("x86".into()),
            runtime: Runtime::Framework,
            needs_elevation: false,
        };
        assert_eq!(p.pointer_size(), 4);
        p.arch = None;
        assert_eq!(p.pointer_size(), 8);
        let _ = list();
        assert!(info(u32::MAX - 3).is_none());
        let json = serde_json::to_value(&p).unwrap();
        assert_eq!(json["needsElevation"], false);
        assert_eq!(json["runtime"], "framework");
    }
}
