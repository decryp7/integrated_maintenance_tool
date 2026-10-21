//! Paths: kernel image paths (`\Device\HarddiskVolume3\…`) to the drive paths DbgHelp opens, and where the
//! captures go (the elevated process writes only below the user's own application-data folders, ADR 0022 §7).

use crate::NativeError;
use std::path::{Path, PathBuf};

/// A kernel path as a drive path: `\Device\HarddiskVolume3\x` with `(\Device\HarddiskVolume3, C:)` in `devices`
/// becomes `C:\x`; `\SystemRoot\x` becomes `<system_root>\x`; `\??\C:\x` becomes `C:\x`. Drive paths are kept;
/// anything else is `None`.
pub fn map_device_path(path: &str, devices: &[(String, String)], system_root: Option<&str>) -> Option<String> {
    let b = path.as_bytes();
    if b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\' {
        return Some(path.to_string());
    }
    if let Some(rest) = path.strip_prefix(r"\??\") {
        return Some(rest.to_string());
    }
    let lower = path.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix(r"\systemroot\") {
        let root = system_root?.trim_end_matches('\\');
        return Some(format!("{root}\\{}", &path[path.len() - rest.len()..]));
    }
    devices.iter().find_map(|(device, drive)| {
        let d = device.to_ascii_lowercase();
        let rest = lower.strip_prefix(&d)?;
        rest.starts_with('\\')
            .then(|| format!("{drive}{}", &path[path.len() - rest.len()..]))
    })
}

/// Whether `dir` lies inside one of `roots` (compared without case, as Windows does).
pub fn is_within(dir: &Path, roots: &[PathBuf]) -> bool {
    let norm = |p: &Path| {
        let s = p.to_string_lossy().replace('/', "\\").to_lowercase();
        let s = s.strip_prefix(r"\\?\").map(str::to_string).unwrap_or(s);
        s.trim_end_matches('\\').to_string()
    };
    let d = norm(dir);
    if d.split('\\').any(|c| c == "..") {
        return false;
    }
    roots
        .iter()
        .map(|r| norm(r))
        .any(|r| !r.is_empty() && (d == r || d.starts_with(&format!("{r}\\"))))
}

/// The folder captures are written to: `<data dir>/captures/native` (created).
pub fn capture_dir(data_dir: Option<&Path>) -> Result<PathBuf, NativeError> {
    let dir = data_dir
        .ok_or_else(|| NativeError::Unsupported("IMT has no data folder to write captures to".into()))?
        .join("captures")
        .join("native");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// The capture folder of the elevated process: the host's data folder only when it is inside the user's own
/// `%APPDATA%` or `%LOCALAPPDATA%` (as the elevated token sees them); anything else is refused.
pub fn elevated_capture_dir(data_dir: Option<&Path>) -> Result<PathBuf, NativeError> {
    let data = data_dir.ok_or_else(|| NativeError::Unsupported("IMT has no data folder to write captures to".into()))?;
    let roots = user_data_roots();
    let canonical = data.canonicalize().unwrap_or_else(|_| data.to_path_buf());
    if !is_within(&canonical, &roots) {
        return Err(NativeError::Unsupported(format!(
            "the elevated recorder writes only into your application-data folder, not {}",
            data.display()
        )));
    }
    capture_dir(Some(&canonical))
}

/// The user's application-data folders (`%APPDATA%`, `%LOCALAPPDATA%`), as this process's token resolves them.
#[cfg(windows)]
pub fn user_data_roots() -> Vec<PathBuf> {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{FOLDERID_LocalAppData, FOLDERID_RoamingAppData, SHGetKnownFolderPath, KF_FLAG_DEFAULT};
    let mut out = Vec::new();
    for id in [&FOLDERID_RoamingAppData, &FOLDERID_LocalAppData] {
        // SAFETY: a known folder id; the returned string is freed below.
        if let Ok(p) = unsafe { SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None) } {
            // SAFETY: a NUL-terminated string from the shell.
            if let Ok(s) = unsafe { p.to_string() } {
                out.push(PathBuf::from(s));
            }
            // SAFETY: allocated by SHGetKnownFolderPath.
            unsafe { CoTaskMemFree(Some(p.0 as *const core::ffi::c_void)) };
        }
    }
    out
}

#[cfg(not(windows))]
pub fn user_data_roots() -> Vec<PathBuf> {
    Vec::new()
}

/// The drive letters' device names (`\Device\HarddiskVolume3` → `C:`).
#[cfg(windows)]
pub fn device_map() -> Vec<(String, String)> {
    use windows::core::HSTRING;
    use windows::Win32::Storage::FileSystem::QueryDosDeviceW;
    let mut out = Vec::new();
    for letter in b'A'..=b'Z' {
        let drive = format!("{}:", char::from(letter));
        let mut buf = [0u16; 1024];
        // SAFETY: a valid drive name and buffer.
        let n = unsafe { QueryDosDeviceW(&HSTRING::from(drive.as_str()), Some(&mut buf)) } as usize;
        if n == 0 {
            continue;
        }
        let first = buf[..n].split(|&u| u == 0).next().unwrap_or_default();
        if !first.is_empty() {
            out.push((String::from_utf16_lossy(first), drive));
        }
    }
    out
}

#[cfg(not(windows))]
pub fn device_map() -> Vec<(String, String)> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_paths_become_drive_paths() {
        let devices = vec![(r"\Device\HarddiskVolume3".to_string(), "C:".to_string())];
        let root = Some(r"C:\Windows");
        assert_eq!(
            map_device_path(r"\Device\HarddiskVolume3\app\busy.exe", &devices, root).as_deref(),
            Some(r"C:\app\busy.exe")
        );
        assert_eq!(
            map_device_path(r"\SystemRoot\System32\ntdll.dll", &devices, root).as_deref(),
            Some(r"C:\Windows\System32\ntdll.dll")
        );
        assert_eq!(map_device_path(r"\??\D:\x.dll", &devices, root).as_deref(), Some(r"D:\x.dll"));
        assert_eq!(map_device_path(r"C:\y.dll", &devices, root).as_deref(), Some(r"C:\y.dll"));
        assert_eq!(map_device_path(r"\Device\HarddiskVolume30\x", &devices, root), None);
        assert_eq!(map_device_path(r"\SystemRoot\x", &devices, None), None);
        assert_eq!(map_device_path("", &devices, root), None);
    }

    #[test]
    fn elevated_output_stays_in_the_users_app_data() {
        let roots = vec![
            PathBuf::from(r"C:\Users\me\AppData\Roaming"),
            PathBuf::from(r"C:\Users\me\AppData\Local"),
        ];
        assert!(is_within(Path::new(r"C:\Users\me\AppData\Roaming\imt"), &roots));
        assert!(is_within(Path::new(r"c:\users\ME\appdata\local\imt\x"), &roots));
        assert!(is_within(Path::new(r"\\?\C:\Users\me\AppData\Local\imt"), &roots));
        assert!(!is_within(Path::new(r"C:\Windows\System32"), &roots));
        assert!(!is_within(Path::new(r"C:\Users\me\AppData\Roaming2"), &roots));
        assert!(!is_within(Path::new(r"C:\Users\me\AppData\Roaming\..\..\x"), &roots));
        assert!(!is_within(Path::new(r"C:\x"), &[]));
        assert!(elevated_capture_dir(None).is_err());
        assert!(elevated_capture_dir(Some(Path::new(r"C:\Windows\Temp\imt"))).is_err());
        let tmp = tempfile::tempdir().unwrap();
        let d = capture_dir(Some(tmp.path())).unwrap();
        assert!(d.ends_with(Path::new("captures").join("native")) && d.is_dir());
    }

    #[cfg(windows)]
    #[test]
    fn this_machine_has_drives_and_app_data() {
        assert!(device_map().iter().any(|(_, d)| d == "C:"));
        assert_eq!(user_data_roots().len(), 2);
    }
}
