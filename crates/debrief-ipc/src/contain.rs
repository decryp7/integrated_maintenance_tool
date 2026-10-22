//! Windows: starting a child inside an AppContainer ([`crate::sandbox::Containment::Launcher`], ADR 0027), so a
//! binary Debrief did not build is confined before its first instruction:
//!
//! - the AppContainer (one profile per name, created on first use) has no access to the user's files, registry or
//!   objects except what is granted to its SID: the folders the spec names ([`grant`]) and the host's pipe;
//! - network access only through the `internetClient` / `privateNetworkClientServer` capabilities, given when the
//!   sandbox allows the network;
//! - the process is created suspended, put into its job object (memory, CPU and child-process limits, killed with
//!   Debrief) and only then resumed, so it cannot start anything before the job holds it;
//! - it inherits exactly two handles (its stdin pipe, its stderr pipe) and an environment built from the spec.

use crate::job::{JobGuard, JobLimits};
use crate::process::SpawnSpec;
use std::fs::File;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::FromRawHandle;
use std::os::windows::process::ExitStatusExt;
use std::path::Path;
use std::process::ExitStatus;
use windows::core::{w, HRESULT, PCWSTR, PWSTR};
use windows::Win32::Foundation::{
    CloseHandle, LocalFree, SetHandleInformation, GENERIC_ALL, GENERIC_EXECUTE, GENERIC_READ, HANDLE, HANDLE_FLAGS, HANDLE_FLAG_INHERIT,
    HLOCAL, WAIT_OBJECT_0,
};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSidToSidW, GetNamedSecurityInfoW, SetEntriesInAclW, SetNamedSecurityInfoW, EXPLICIT_ACCESS_W,
    GRANT_ACCESS, NO_MULTIPLE_TRUSTEE, SE_FILE_OBJECT, TRUSTEE_IS_SID, TRUSTEE_IS_WELL_KNOWN_GROUP, TRUSTEE_W,
};
use windows::Win32::Security::Isolation::{
    CreateAppContainerProfile, DeleteAppContainerProfile, DeriveAppContainerSidFromAppContainerName,
};
use windows::Win32::Security::{
    FreeSid, ACL, DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES, SECURITY_CAPABILITIES, SID_AND_ATTRIBUTES,
    SUB_CONTAINERS_AND_OBJECTS_INHERIT,
};
use windows::Win32::System::Pipes::CreatePipe;
use windows::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, GetExitCodeProcess, InitializeProcThreadAttributeList, ResumeThread, TerminateProcess,
    UpdateProcThreadAttribute, WaitForSingleObject, BELOW_NORMAL_PRIORITY_CLASS, CREATE_NO_WINDOW, CREATE_SUSPENDED,
    CREATE_UNICODE_ENVIRONMENT, EXTENDED_STARTUPINFO_PRESENT, LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_CREATION_FLAGS, PROCESS_INFORMATION,
    PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, STARTF_USESTDHANDLES, STARTUPINFOEXW, STARTUPINFOW,
};

/// `HRESULT_FROM_WIN32(ERROR_ALREADY_EXISTS)`.
const ALREADY_EXISTS: HRESULT = HRESULT(0x800700B7_u32 as i32);
/// `SE_GROUP_ENABLED`.
const SE_GROUP_ENABLED: u32 = 0x4;
/// The capabilities that open the network to an AppContainer: `internetClient`, `privateNetworkClientServer`.
const NETWORK_CAPABILITIES: [&str; 2] = ["S-1-15-3-1", "S-1-15-3-3"];

fn wide(s: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(std::iter::once(0)).collect()
}

/// A SID allocated by the system, freed on drop.
struct Sid {
    psid: PSID,
    /// `FreeSid` (container SIDs) or `LocalFree` (converted string SIDs).
    local: bool,
}

impl Drop for Sid {
    fn drop(&mut self) {
        // SAFETY: the SID was allocated by the call named in `local` and is freed once.
        unsafe {
            if self.local {
                let _ = LocalFree(Some(HLOCAL(self.psid.0)));
            } else {
                let _ = FreeSid(self.psid);
            }
        }
    }
}

fn string_sid(text: &str) -> io::Result<Sid> {
    let mut psid = PSID::default();
    let w = wide(text);
    // SAFETY: `w` is NUL-terminated and outlives the call; the SID is freed by `Sid`.
    unsafe { ConvertStringSidToSidW(PCWSTR(w.as_ptr()), &mut psid) }.map_err(|e| io::Error::other(format!("SID {text}: {e}")))?;
    Ok(Sid { psid, local: true })
}

/// An AppContainer profile (created on first use, then reused by name).
pub struct Container {
    sid: Sid,
    /// The SID as `S-1-15-2-...`, for security descriptors.
    pub sid_string: String,
}

// SAFETY: the SID is plain memory owned by this value and only read.
unsafe impl Send for Container {}
// SAFETY: as above; nothing mutates the SID.
unsafe impl Sync for Container {}

impl Container {
    /// The container named `name` (see [`crate::sandbox::Containment::valid_name`]).
    pub fn open(name: &str) -> io::Result<Container> {
        if !crate::sandbox::Containment::valid_name(name) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("\"{name}\" is not a valid container name"),
            ));
        }
        let w = wide(name);
        // Profiles are created one at a time: two creations of one name at once fail with a misleading error.
        static CREATING: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _one = CREATING.lock().unwrap_or_else(|p| p.into_inner());
        let mut attempt = 0;
        let psid = loop {
            attempt += 1;
            // SAFETY: NUL-terminated strings that outlive the calls; the returned SID is freed by `Sid`.
            let created = unsafe { CreateAppContainerProfile(PCWSTR(w.as_ptr()), PCWSTR(w.as_ptr()), w!("A Debrief plugin"), None) };
            match created {
                Ok(sid) => break sid,
                // SAFETY: as above.
                Err(e) if e.code() == ALREADY_EXISTS => match unsafe { DeriveAppContainerSidFromAppContainerName(PCWSTR(w.as_ptr())) } {
                    Ok(sid) => break sid,
                    Err(e) => return Err(io::Error::other(format!("AppContainer {name}: {e}"))),
                },
                // Another process may be creating the same profile right now: try again shortly.
                Err(_) if attempt < 5 => std::thread::sleep(std::time::Duration::from_millis(100 * attempt)),
                Err(e) => return Err(io::Error::other(format!("cannot create the AppContainer {name}: {e}"))),
            }
        };
        let sid = Sid { psid, local: false };
        let mut text = PWSTR::null();
        // SAFETY: a valid SID; the string is freed below.
        let sid_string = unsafe {
            ConvertSidToStringSidW(sid.psid, &mut text).map_err(|e| io::Error::other(e.to_string()))?;
            let s = text.to_string().unwrap_or_default();
            let _ = LocalFree(Some(HLOCAL(text.0 as *mut _)));
            s
        };
        Ok(Container { sid, sid_string })
    }

    /// Deletes the container profile named `name` (when its plugin is removed); a missing profile is not an error.
    pub fn delete(name: &str) -> io::Result<()> {
        if !crate::sandbox::Containment::valid_name(name) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("\"{name}\" is not a valid container name"),
            ));
        }
        let w = wide(name);
        // SAFETY: a NUL-terminated string that outlives the call.
        unsafe { DeleteAppContainerProfile(PCWSTR(w.as_ptr())) }.map_err(|e| io::Error::other(format!("AppContainer {name}: {e}")))
    }

    /// Lets the container read and execute (or, with `write`, change) `path` and everything below it.
    pub fn grant(&self, path: &Path, write: bool) -> io::Result<()> {
        let name = wide(path);
        let mut dacl: *mut ACL = std::ptr::null_mut();
        let mut sd = PSECURITY_DESCRIPTOR::default();
        // SAFETY: Win32 calls with valid pointers; the descriptor and the new ACL are freed below.
        unsafe {
            let err = GetNamedSecurityInfoW(
                PCWSTR(name.as_ptr()),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(&mut dacl),
                None,
                &mut sd,
            );
            if err.is_err() {
                return Err(io::Error::other(format!(
                    "cannot read the permissions of {}: {err:?}",
                    path.display()
                )));
            }
            let rights = if write { GENERIC_ALL.0 } else { GENERIC_READ.0 | GENERIC_EXECUTE.0 };
            let entry = EXPLICIT_ACCESS_W {
                grfAccessPermissions: rights,
                grfAccessMode: GRANT_ACCESS,
                grfInheritance: SUB_CONTAINERS_AND_OBJECTS_INHERIT,
                Trustee: TRUSTEE_W {
                    pMultipleTrustee: std::ptr::null_mut(),
                    MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
                    TrusteeForm: TRUSTEE_IS_SID,
                    TrusteeType: TRUSTEE_IS_WELL_KNOWN_GROUP,
                    ptstrName: PWSTR(self.sid.psid.0 as *mut u16),
                },
            };
            let mut new_acl: *mut ACL = std::ptr::null_mut();
            let err = SetEntriesInAclW(Some(&[entry]), Some(dacl), &mut new_acl);
            let result = if err.is_err() {
                Err(io::Error::other(format!(
                    "cannot extend the permissions of {}: {err:?}",
                    path.display()
                )))
            } else {
                let err = SetNamedSecurityInfoW(
                    PCWSTR(name.as_ptr()),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION,
                    None,
                    None,
                    Some(new_acl),
                    None,
                );
                if err.is_err() {
                    Err(io::Error::other(format!(
                        "cannot grant the plugin access to {}: {err:?}",
                        path.display()
                    )))
                } else {
                    Ok(())
                }
            };
            if !new_acl.is_null() {
                let _ = LocalFree(Some(HLOCAL(new_acl as *mut _)));
            }
            let _ = LocalFree(Some(HLOCAL(sd.0)));
            result
        }
    }
}

/// An owned process handle.
struct Handle(HANDLE);

// SAFETY: process handles may be used from any thread.
unsafe impl Send for Handle {}
// SAFETY: the calls made through a shared reference (wait, exit code, terminate) are thread-safe.
unsafe impl Sync for Handle {}

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_invalid() && !self.0 .0.is_null() {
            // SAFETY: owned handle, closed once.
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

/// A process started in a container.
pub struct Contained {
    process: Handle,
    pid: u32,
}

impl Contained {
    pub fn id(&self) -> u32 {
        self.pid
    }

    pub fn try_wait(&self) -> io::Result<Option<ExitStatus>> {
        // SAFETY: a valid process handle.
        unsafe {
            if WaitForSingleObject(self.process.0, 0) != WAIT_OBJECT_0 {
                return Ok(None);
            }
            let mut code = 0u32;
            GetExitCodeProcess(self.process.0, &mut code).map_err(|e| io::Error::other(e.to_string()))?;
            Ok(Some(ExitStatus::from_raw(code)))
        }
    }

    pub fn kill(&self) -> io::Result<()> {
        // SAFETY: a valid process handle; terminating an exited process fails harmlessly.
        match unsafe { TerminateProcess(self.process.0, 1) } {
            Ok(()) => Ok(()),
            Err(_) if self.try_wait()?.is_some() => Ok(()),
            Err(e) => Err(io::Error::other(e.to_string())),
        }
    }

    pub fn wait(&self) -> io::Result<ExitStatus> {
        // SAFETY: a valid process handle.
        unsafe {
            let _ = WaitForSingleObject(self.process.0, u32::MAX);
        }
        self.try_wait()?.ok_or_else(|| io::Error::other("the process did not exit"))
    }

    /// The process handle (for the job object).
    pub fn raw(&self) -> HANDLE {
        self.process.0
    }
}

/// What [`spawn`] started.
pub struct Started {
    pub child: Contained,
    pub stdin: File,
    pub stderr: Option<File>,
    pub job: JobGuard,
}

/// One argument quoted for `CommandLineToArgvW` (the MSVC runtime's rules).
fn quote(arg: &str, out: &mut String) {
    if !arg.is_empty() && !arg.chars().any(|c| matches!(c, ' ' | '\t' | '\n' | '\u{b}' | '"')) {
        out.push_str(arg);
        return;
    }
    out.push('"');
    let mut backslashes = 0usize;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.extend(std::iter::repeat_n('\\', backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    out.extend(std::iter::repeat_n('\\', backslashes * 2));
    out.push('"');
}

/// The command line of `spec` (program first).
pub fn command_line(spec: &SpawnSpec) -> String {
    let mut line = String::new();
    quote(&spec.exe.to_string_lossy(), &mut line);
    for a in &spec.args {
        line.push(' ');
        quote(a, &mut line);
    }
    line
}

/// Variables Windows needs to create an AppContainer process (it moves the container's profile folders below
/// them); they name folders, not secrets.
const CONTAINER_ENV: &[&str] = &["USERPROFILE", "LOCALAPPDATA", "APPDATA"];

/// The environment block of `spec`: the kept variables of this process, [`CONTAINER_ENV`] and the spec's own
/// (later ones win, names compared without case), sorted, `K=V<NUL>...<NUL><NUL>`.
fn environment(spec: &SpawnSpec) -> Vec<u16> {
    let mut vars: Vec<(String, String)> = Vec::new();
    let mut set = |k: &str, v: String| {
        vars.retain(|(have, _)| !have.eq_ignore_ascii_case(k));
        vars.push((k.to_string(), v));
    };
    for k in spec.keep_env.iter().chain(CONTAINER_ENV) {
        if let Ok(v) = std::env::var(k) {
            set(k, v);
        }
    }
    for (k, v) in &spec.env {
        set(k, v.clone());
    }
    vars.sort_by_key(|(k, _)| k.to_ascii_uppercase());
    let mut block: Vec<u16> = Vec::new();
    for (k, v) in vars {
        block.extend(format!("{k}={v}").encode_utf16());
        block.push(0);
    }
    if block.is_empty() {
        block.push(0);
    }
    block.push(0);
    block
}

fn pipe(inherit_read: bool) -> io::Result<(HANDLE, HANDLE)> {
    let sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: true.into(),
    };
    let (mut r, mut w) = (HANDLE::default(), HANDLE::default());
    // SAFETY: valid out pointers; the handles are owned by the caller.
    unsafe {
        CreatePipe(&mut r, &mut w, Some(&sa), 0).map_err(|e| io::Error::other(e.to_string()))?;
        // Only the child's end is inherited.
        let ours = if inherit_read { w } else { r };
        SetHandleInformation(ours, HANDLE_FLAG_INHERIT.0, HANDLE_FLAGS(0)).map_err(|e| io::Error::other(e.to_string()))?;
    }
    Ok((r, w))
}

fn close(h: HANDLE) {
    if !h.0.is_null() {
        // SAFETY: an owned handle, closed once.
        unsafe {
            let _ = CloseHandle(h);
        }
    }
}

/// Starts `spec` in `container` with `limits`, suspended until its job holds it. `stderr`: pipe its standard
/// error (else it has none). The working directory is the spec's (it must be one the container can read).
pub fn spawn(spec: &SpawnSpec, container: &Container, network: bool, limits: &JobLimits, stderr: bool) -> io::Result<Started> {
    let (in_r, in_w) = pipe(true)?;
    let (err_r, err_w) = if stderr {
        match pipe(false) {
            Ok(p) => p,
            Err(e) => {
                close(in_r);
                close(in_w);
                return Err(e);
            }
        }
    } else {
        (HANDLE::default(), HANDLE::default())
    };
    let result = spawn_with(spec, container, network, limits, in_r, err_w);
    close(in_r);
    close(err_w);
    match result {
        // SAFETY: the parent's ends are owned here and move into the files.
        Ok((child, job)) => Ok(Started {
            child,
            stdin: unsafe { File::from_raw_handle(in_w.0) },
            stderr: stderr.then(|| unsafe { File::from_raw_handle(err_r.0) }),
            job,
        }),
        Err(e) => {
            close(in_w);
            close(err_r);
            Err(e)
        }
    }
}

fn spawn_with(
    spec: &SpawnSpec,
    container: &Container,
    network: bool,
    limits: &JobLimits,
    stdin: HANDLE,
    stderr: HANDLE,
) -> io::Result<(Contained, JobGuard)> {
    let caps: Vec<Sid> = if network {
        NETWORK_CAPABILITIES.iter().map(|s| string_sid(s)).collect::<io::Result<_>>()?
    } else {
        Vec::new()
    };
    let mut cap_attrs: Vec<SID_AND_ATTRIBUTES> = caps
        .iter()
        .map(|c| SID_AND_ATTRIBUTES {
            Sid: c.psid,
            Attributes: SE_GROUP_ENABLED,
        })
        .collect();
    let capabilities = SECURITY_CAPABILITIES {
        AppContainerSid: container.sid.psid,
        Capabilities: if cap_attrs.is_empty() {
            std::ptr::null_mut()
        } else {
            cap_attrs.as_mut_ptr()
        },
        CapabilityCount: cap_attrs.len() as u32,
        Reserved: 0,
    };
    let handles: Vec<HANDLE> = [stdin, stderr].into_iter().filter(|h| !h.0.is_null()).collect();
    let mut size = 0usize;
    // SAFETY: the first call only reports the size of the list.
    let _ = unsafe { InitializeProcThreadAttributeList(None, 2, None, &mut size) };
    let mut buf = vec![0u8; size.max(64)];
    let list = LPPROC_THREAD_ATTRIBUTE_LIST(buf.as_mut_ptr() as *mut _);
    let mut cmd: Vec<u16> = command_line(spec).encode_utf16().chain(std::iter::once(0)).collect();
    let exe = wide(&spec.exe);
    let env = environment(spec);
    let cwd = spec.current_dir.as_ref().map(wide);
    let mut flags: PROCESS_CREATION_FLAGS = EXTENDED_STARTUPINFO_PRESENT | CREATE_SUSPENDED | CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT;
    if spec.below_normal_priority {
        flags |= BELOW_NORMAL_PRIORITY_CLASS;
    }
    let mut pi = PROCESS_INFORMATION::default();
    // SAFETY: every pointer refers to a local that outlives the calls; the attribute list is deleted below and the
    // process and thread handles are owned afterwards.
    unsafe {
        InitializeProcThreadAttributeList(Some(list), 2, None, &mut size).map_err(|e| io::Error::other(e.to_string()))?;
        let created = UpdateProcThreadAttribute(
            list,
            0,
            PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES as usize,
            Some(&capabilities as *const SECURITY_CAPABILITIES as *const _),
            std::mem::size_of::<SECURITY_CAPABILITIES>(),
            None,
            None,
        )
        .map_err(|e| windows::core::Error::new(e.code(), format!("security capabilities: {}", e.message())))
        .and_then(|()| {
            UpdateProcThreadAttribute(
                list,
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                Some(handles.as_ptr() as *const _),
                handles.len() * std::mem::size_of::<HANDLE>(),
                None,
                None,
            )
            .map_err(|e| windows::core::Error::new(e.code(), format!("handle list: {}", e.message())))
        })
        .and_then(|()| {
            let si = STARTUPINFOEXW {
                StartupInfo: STARTUPINFOW {
                    cb: std::mem::size_of::<STARTUPINFOEXW>() as u32,
                    dwFlags: STARTF_USESTDHANDLES,
                    hStdInput: stdin,
                    hStdOutput: HANDLE::default(),
                    hStdError: stderr,
                    ..Default::default()
                },
                lpAttributeList: list,
            };
            CreateProcessW(
                PCWSTR(exe.as_ptr()),
                Some(PWSTR(cmd.as_mut_ptr())),
                None,
                None,
                true,
                flags,
                Some(env.as_ptr() as *const _),
                cwd.as_ref().map_or(PCWSTR::null(), |c| PCWSTR(c.as_ptr())),
                &si.StartupInfo,
                &mut pi,
            )
        });
        DeleteProcThreadAttributeList(list);
        created.map_err(|e| io::Error::other(format!("cannot start {} in its sandbox ({}): {e}", spec.label, spec.exe.display())))?;
    }
    let child = Contained {
        process: Handle(pi.hProcess),
        pid: pi.dwProcessId,
    };
    let thread = Handle(pi.hThread);
    // The job holds the process before it runs: no window in which it could start another program.
    let Some(job) = JobGuard::assign_handle(child.raw(), limits) else {
        let _ = child.kill();
        return Err(io::Error::other(format!("cannot put {} into its job object", spec.label)));
    };
    // SAFETY: the suspended main thread of the process just created.
    if unsafe { ResumeThread(thread.0) } == u32::MAX {
        let _ = child.kill();
        return Err(io::Error::other(format!("cannot resume {}", spec.label)));
    }
    drop(caps);
    Ok((child, job))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_lines_quote_like_the_msvc_runtime() {
        let mut s = SpawnSpec::new(r"C:\Program Files\x\p.exe", "p");
        s.args = vec!["--plugin".into(), "a b".into(), r#"q"uote"#.into(), r"tail\".into(), String::new()];
        assert_eq!(command_line(&s), r#""C:\Program Files\x\p.exe" --plugin "a b" "q\"uote" tail\ """#);
        let mut t = SpawnSpec::new("p", "p");
        t.args = vec![r"a\\b c\".into()];
        assert_eq!(command_line(&t), r#"p "a\\b c\\""#);
    }

    #[test]
    fn the_environment_block_holds_only_what_the_spec_names() {
        let mut s = SpawnSpec::new("p", "p");
        s.keep_env = &["SystemRoot", "SYSTEMROOT"];
        s.env = vec![("DEBRIEF_X".into(), "1".into()), ("systemroot".into(), "override".into())];
        let block = String::from_utf16(&environment(&s)).unwrap();
        assert!(block.ends_with("\0\0"));
        let vars: Vec<&str> = block.trim_end_matches('\0').split('\0').collect();
        assert!(vars.contains(&"DEBRIEF_X=1") && vars.contains(&"systemroot=override"), "{vars:?}");
        let upper = |v: &&str| v.to_ascii_uppercase();
        assert_eq!(vars.iter().filter(|v| upper(v).starts_with("SYSTEMROOT=")).count(), 1);
        let allowed = ["DEBRIEF_X=", "SYSTEMROOT=", "USERPROFILE=", "LOCALAPPDATA=", "APPDATA="];
        assert!(vars.iter().all(|v| allowed.iter().any(|k| upper(v).starts_with(k))), "{vars:?}");
        let mut sorted = vars.clone();
        sorted.sort_by_key(upper);
        assert_eq!(sorted, vars);
    }

    #[test]
    fn containers_are_created_reused_and_deleted() {
        assert!(Container::open("bad name").is_err());
        assert!(Container::open(&"x".repeat(65)).is_err());
        assert!(Container::delete("bad name").is_err());
        let name = "debrief.test.contain-unit";
        let a = Container::open(name).unwrap();
        let b = Container::open(name).unwrap();
        assert!(a.sid_string.starts_with("S-1-15-2-"));
        assert_eq!(a.sid_string, b.sid_string, "the same profile is reused");
        Container::delete(name).unwrap();
        // Deleting again: gone already, which is fine.
        let _ = Container::delete(name);
    }
}
