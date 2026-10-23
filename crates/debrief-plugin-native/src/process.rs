//! Processes that can be profiled: listing (name, path, architecture, whether the sampler may open it), and the
//! threads, modules and runtime of one process. Windows only; elsewhere the list is empty.

use serde::Serialize;

/// Most processes listed.
pub const MAX_PROCESSES: usize = 4096;

/// A process the dialog lists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeProcess {
    pub pid: u32,
    pub name: String,
    pub path: Option<String>,
    /// `x86`, `x64` or `arm64` when known.
    pub arch: Option<String>,
    /// The sampler can open it (same user, not protected); otherwise only ETW (administrator) records it.
    pub accessible: bool,
}

/// The processes of this machine (not Debrief's own plugin process), sorted by name then pid.
pub fn list() -> Vec<NativeProcess> {
    let mut v = sys::list();
    v.retain(|p| p.pid != 0 && p.pid != 4 && p.pid != std::process::id());
    v.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.pid.cmp(&b.pid)));
    v.truncate(MAX_PROCESSES);
    v
}

/// Whether a module name is the .NET runtime (`clr.dll`, `coreclr.dll`).
pub fn is_clr_module(name: &str) -> bool {
    let n = name.rsplit(['\\', '/']).next().unwrap_or(name).to_ascii_lowercase();
    n == "clr.dll" || n == "coreclr.dll"
}

#[cfg(windows)]
pub use sys::{arch as arch_of, has_clr, info, modules, thread_name, threads};

#[cfg(windows)]
mod sys {
    use super::{is_clr_module, NativeProcess, MAX_PROCESSES};
    use debrief_plugin_profiling::nettrace::ext::ImageLoad;
    use windows::core::PWSTR;
    use windows::Win32::Foundation::{CloseHandle, LocalFree, HANDLE, HLOCAL};
    use windows::Win32::System::Diagnostics::ToolHelp::*;
    use windows::Win32::System::SystemInformation::{
        IMAGE_FILE_MACHINE, IMAGE_FILE_MACHINE_AMD64, IMAGE_FILE_MACHINE_ARM64, IMAGE_FILE_MACHINE_I386,
    };
    use windows::Win32::System::Threading::{
        GetThreadDescription, IsWow64Process2, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_INFORMATION,
        PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ,
    };

    /// A handle closed on drop.
    pub(crate) struct Owned(pub HANDLE);

    impl Drop for Owned {
        fn drop(&mut self) {
            // SAFETY: a handle this struct owns.
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    fn wide(s: &[u16]) -> String {
        let end = s.iter().position(|&u| u == 0).unwrap_or(s.len());
        String::from_utf16_lossy(&s[..end])
    }

    /// `x86`, `x64` or `arm64` of an open process.
    pub fn arch(process: HANDLE) -> Option<&'static str> {
        let mut machine = IMAGE_FILE_MACHINE::default();
        let mut native = IMAGE_FILE_MACHINE::default();
        // SAFETY: valid out pointers; the handle has query rights.
        unsafe { IsWow64Process2(process, &mut machine, Some(&mut native)) }.ok()?;
        let m = if machine.0 == 0 { native } else { machine };
        match m {
            IMAGE_FILE_MACHINE_I386 => Some("x86"),
            IMAGE_FILE_MACHINE_AMD64 => Some("x64"),
            IMAGE_FILE_MACHINE_ARM64 => Some("arm64"),
            _ => None,
        }
    }

    fn image_path(process: HANDLE) -> Option<String> {
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        // SAFETY: a buffer of `len` characters.
        unsafe { QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len) }.ok()?;
        Some(String::from_utf16_lossy(&buf[..(len as usize).min(buf.len())]))
    }

    pub(super) fn describe(pid: u32, name: String) -> NativeProcess {
        // SAFETY: handles are owned and closed.
        let limited = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
            .ok()
            .map(Owned);
        let accessible = unsafe { OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, false, pid) }
            .ok()
            .map(Owned)
            .is_some();
        NativeProcess {
            pid,
            path: limited.as_ref().and_then(|h| image_path(h.0)),
            arch: limited.as_ref().and_then(|h| arch(h.0)).map(str::to_string),
            name,
            accessible,
        }
    }

    pub(super) fn list() -> Vec<NativeProcess> {
        // SAFETY: the snapshot is closed by Owned.
        let Ok(snap) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }).map(Owned) else {
            return Vec::new();
        };
        let mut e = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut out = Vec::new();
        // SAFETY: a sized entry; the snapshot is valid.
        let mut ok = unsafe { Process32FirstW(snap.0, &mut e) }.is_ok();
        while ok && out.len() < MAX_PROCESSES {
            out.push(describe(e.th32ProcessID, wide(&e.szExeFile)));
            // SAFETY: as above.
            ok = unsafe { Process32NextW(snap.0, &mut e) }.is_ok();
        }
        out
    }

    /// One process, when it is running.
    pub fn info(pid: u32) -> Option<NativeProcess> {
        list().into_iter().find(|p| p.pid == pid)
    }

    /// The thread ids of `pid`.
    pub fn threads(pid: u32) -> Vec<u32> {
        // SAFETY: the snapshot is closed by Owned.
        let Ok(snap) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) }).map(Owned) else {
            return Vec::new();
        };
        let mut e = THREADENTRY32 {
            dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
            ..Default::default()
        };
        let mut out = Vec::new();
        // SAFETY: a sized entry; the snapshot is valid.
        let mut ok = unsafe { Thread32First(snap.0, &mut e) }.is_ok();
        while ok && out.len() < 65_536 {
            if e.th32OwnerProcessID == pid {
                out.push(e.th32ThreadID);
            }
            // SAFETY: as above.
            ok = unsafe { Thread32Next(snap.0, &mut e) }.is_ok();
        }
        out
    }

    /// The modules of `pid` (32- and 64-bit) as images.
    pub fn modules(pid: u32) -> Vec<ImageLoad> {
        // ERROR_BAD_LENGTH while the process is starting: retry a few times.
        for _ in 0..5 {
            // SAFETY: the snapshot is closed by Owned.
            let snap = match unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid) } {
                Ok(h) => Owned(h),
                Err(_) => {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                    continue;
                }
            };
            let mut e = MODULEENTRY32W {
                dwSize: std::mem::size_of::<MODULEENTRY32W>() as u32,
                ..Default::default()
            };
            let mut out = Vec::new();
            // SAFETY: a sized entry; the snapshot is valid.
            let mut ok = unsafe { Module32FirstW(snap.0, &mut e) }.is_ok();
            while ok && out.len() < 16_384 {
                out.push(ImageLoad {
                    base: e.modBaseAddr as u64,
                    size: u64::from(e.modBaseSize),
                    path: wide(&e.szExePath),
                });
                // SAFETY: as above.
                ok = unsafe { Module32NextW(snap.0, &mut e) }.is_ok();
            }
            return out;
        }
        Vec::new()
    }

    /// Whether `pid` has loaded the .NET runtime.
    pub fn has_clr(pid: u32) -> bool {
        modules(pid).iter().any(|m| is_clr_module(&m.path))
    }

    /// A thread's description (`SetThreadDescription`), if it has one.
    pub fn thread_name(thread: HANDLE) -> Option<String> {
        // SAFETY: the handle has THREAD_QUERY_LIMITED_INFORMATION; the string is freed below.
        let p = unsafe { GetThreadDescription(thread) }.ok()?;
        // SAFETY: a NUL-terminated string from the system.
        let s = unsafe { p.to_string() }.ok();
        // SAFETY: allocated by GetThreadDescription.
        unsafe {
            let _ = LocalFree(Some(HLOCAL(p.0 as *mut core::ffi::c_void)));
        }
        s.filter(|s| !s.is_empty())
    }
}

#[cfg(not(windows))]
mod sys {
    pub(super) fn list() -> Vec<super::NativeProcess> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clr_modules_are_recognised() {
        assert!(is_clr_module(r"C:\Windows\Microsoft.NET\Framework64\v4.0.30319\clr.dll"));
        assert!(is_clr_module("CoreCLR.dll"));
        assert!(!is_clr_module("clrjit.dll"));
    }

    #[test]
    fn listing_never_fails_and_leaves_ourselves_out() {
        let l = list();
        assert!(l.iter().all(|p| p.pid != std::process::id() && p.pid != 0));
        #[cfg(windows)]
        {
            assert!(!l.is_empty());
            // Our parent (cargo) or a sibling: some process of this user is accessible with an architecture.
            assert!(l.iter().any(|p| p.accessible && p.arch.is_some()), "{l:?}");
            let me = std::process::id();
            assert!(!threads(me).is_empty());
            let mods = modules(me);
            assert!(mods.iter().any(|m| m.path.to_ascii_lowercase().ends_with("ntdll.dll")));
            assert!(!has_clr(me));
            assert!(info(u32::MAX - 1).is_none());
        }
    }
}
