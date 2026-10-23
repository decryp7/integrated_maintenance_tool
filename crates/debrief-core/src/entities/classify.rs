//! Classification of one token.

use super::EntityKind;

type Found = Vec<(EntityKind, String)>;

const EXECUTABLE_EXT: &[&str] = &["exe", "bat", "cmd", "msi"];
const LIBRARY_EXT: &[&str] = &["dll", "so", "dylib", "sys", "ocx", "drv", "jar"];
const FILE_EXT: &[&str] = &[
    "log", "txt", "json", "xml", "cfg", "ini", "conf", "yaml", "yml", "toml", "db", "sqlite", "dmp", "etl", "pdb", "bin", "dat", "zip",
    "tmp", "csv", "evtx", "pcap", "pem", "crt", "cer", "key", "sql", "cs", "rs", "py", "js", "ts", "java", "cpp", "h", "c", "go", "gz",
    "tar", "7z", "bak", "lock", "pid", "sock", "plist", "reg", "msix", "dump",
];
const TLDS: &[&str] = &[
    "com",
    "net",
    "org",
    "io",
    "edu",
    "gov",
    "int",
    "dev",
    "cloud",
    "app",
    "info",
    "biz",
    "local",
    "lan",
    "internal",
    "corp",
    "home",
    "intranet",
    "localdomain",
    "uk",
    "de",
    "fr",
    "nl",
    "sg",
    "cn",
    "jp",
    "au",
    "ca",
    "eu",
    "ru",
    "br",
    "us",
    "ai",
    "co",
];
const ERRNO: &[&str] = &[
    "EPERM",
    "ENOENT",
    "EINTR",
    "EIO",
    "ENXIO",
    "E2BIG",
    "ENOEXEC",
    "EBADF",
    "ECHILD",
    "EAGAIN",
    "ENOMEM",
    "EACCES",
    "EFAULT",
    "EBUSY",
    "EEXIST",
    "EXDEV",
    "ENODEV",
    "ENOTDIR",
    "EISDIR",
    "EINVAL",
    "ENFILE",
    "EMFILE",
    "ENOTTY",
    "EFBIG",
    "ENOSPC",
    "ESPIPE",
    "EROFS",
    "EMLINK",
    "EPIPE",
    "EDOM",
    "ERANGE",
    "EDEADLK",
    "ENAMETOOLONG",
    "ENOSYS",
    "ENOTEMPTY",
    "ELOOP",
    "EWOULDBLOCK",
    "ETIMEDOUT",
    "ECONNREFUSED",
    "ECONNRESET",
    "ECONNABORTED",
    "EHOSTUNREACH",
    "ENETUNREACH",
    "EADDRINUSE",
    "EADDRNOTAVAIL",
    "ENOTCONN",
    "EISCONN",
    "EINPROGRESS",
    "EALREADY",
    "ENOTSOCK",
    "EPROTO",
    "ENOBUFS",
    "ESTALE",
    "EDQUOT",
    "ECANCELED",
];

fn upper_name(t: &str) -> bool {
    t.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// Entities of one token (`prev` is the previous token, lower-cased, trimmed of punctuation).
pub(super) fn classify_token(raw: &str, prev: &str) -> Found {
    let t = raw
        .trim_start_matches(['!', '?', '"', '*', '#'])
        .trim_end_matches(['.', ':', '!', '?', '"', '*', '#']);
    if t.is_empty() || t.chars().all(|c| c.is_ascii_digit() || c == '.') && !is_ipv4(t) && !(t.len() == 3 && http_context(prev)) {
        return Vec::new();
    }
    if let Some(ip) = ip(t) {
        return vec![(EntityKind::Ip, ip)];
    }
    if is_guid(t) {
        return vec![(EntityKind::Guid, t.to_lowercase())];
    }
    if is_hex_code(t) {
        return vec![(EntityKind::ErrorCode, t.to_uppercase().replacen("0X", "0x", 1))];
    }
    if is_error_name(t) {
        return vec![(EntityKind::ErrorCode, t.to_string())];
    }
    if t.len() == 3 && t.chars().all(|c| c.is_ascii_digit()) {
        return match t.parse::<u16>() {
            Ok(n) if (100..600).contains(&n) && http_context(prev) => vec![(EntityKind::ErrorCode, format!("HTTP {n}"))],
            _ => Vec::new(),
        };
    }
    if is_path(t) {
        return path(t);
    }
    if is_exception(t) {
        return vec![(EntityKind::Exception, t.to_string())];
    }
    if let Some(e) = file_name(t) {
        return vec![e];
    }
    if let Some(h) = host(t) {
        return vec![(EntityKind::Host, h)];
    }
    Vec::new()
}

fn http_context(prev: &str) -> bool {
    matches!(
        prev,
        "status" | "statuscode" | "http" | "code" | "response" | "returned" | "http/1.0" | "http/1.1" | "http/2" | "http/3"
    )
}

fn is_ipv4(t: &str) -> bool {
    t.parse::<std::net::Ipv4Addr>().is_ok()
}

/// An IP address, optionally with `:port` (IPv4) or a zone-less IPv6.
fn ip(t: &str) -> Option<String> {
    let host = match t.rsplit_once(':') {
        Some((h, p)) if h.contains('.') && !h.contains(':') && p.chars().all(|c| c.is_ascii_digit()) => h,
        _ => t,
    };
    if host.parse::<std::net::Ipv4Addr>().is_ok() {
        return Some(host.to_string());
    }
    if host.matches(':').count() >= 2 && host.chars().all(|c| c.is_ascii_hexdigit() || c == ':' || c == '.') {
        if let Ok(a) = host.parse::<std::net::Ipv6Addr>() {
            return Some(a.to_string());
        }
    }
    None
}

fn is_guid(t: &str) -> bool {
    let parts: Vec<&str> = t.split('-').collect();
    parts.len() == 5
        && [8usize, 4, 4, 4, 12]
            .iter()
            .zip(&parts)
            .all(|(n, p)| p.len() == *n && p.chars().all(|c| c.is_ascii_hexdigit()))
}

/// `0x8xxxxxxx`, `0xCxxxxxxx` (HRESULT, NTSTATUS): eight hex digits starting at 8 or above.
fn is_hex_code(t: &str) -> bool {
    let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) else {
        return false;
    };
    h.len() == 8
        && h.chars().all(|c| c.is_ascii_hexdigit())
        && h.starts_with(|c: char| c.is_ascii_hexdigit() && c.to_digit(16).is_some_and(|d| d >= 8))
}

fn is_error_name(t: &str) -> bool {
    if ERRNO.contains(&t) {
        return true;
    }
    ["WSAE", "ERROR_", "STATUS_", "E_", "HRESULT_"]
        .iter()
        .any(|p| t.starts_with(p) && t.len() >= p.len() + 3 && upper_name(t))
}

fn is_exception(t: &str) -> bool {
    let last = t.rsplit(['.', '+', '$', ':']).next().unwrap_or(t);
    let ident = t.chars().all(|c| c.is_alphanumeric() || matches!(c, '.' | '_' | '$' | '+' | ':'));
    let suffix = |s: &str| last.len() > s.len() && last.ends_with(s);
    ident
        && last.starts_with(|c: char| c.is_alphabetic())
        && (suffix("Exception") || suffix("Error") && last.starts_with(char::is_uppercase))
}

fn ext_of(name: &str) -> Option<(&str, &str)> {
    let (stem, ext) = name.rsplit_once('.')?;
    (!stem.is_empty() && !ext.is_empty() && ext.len() <= 8 && ext.chars().all(|c| c.is_ascii_alphanumeric())).then_some((stem, ext))
}

fn file_kind(name: &str) -> Option<EntityKind> {
    let lower = name.to_lowercase();
    // libfoo.so.1.2
    if let Some(i) = lower.find(".so.") {
        if lower[i + 4..]
            .split('.')
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
        {
            return Some(EntityKind::Library);
        }
    }
    let (_, ext) = ext_of(&lower)?;
    if EXECUTABLE_EXT.contains(&ext) {
        Some(EntityKind::Executable)
    } else if LIBRARY_EXT.contains(&ext) {
        Some(EntityKind::Library)
    } else if FILE_EXT.contains(&ext) {
        Some(EntityKind::File)
    } else {
        None
    }
}

fn file_name(t: &str) -> Option<(EntityKind, String)> {
    if t.contains('/') || t.contains('\\') || t.contains('@') || !t.chars().any(|c| c.is_alphabetic()) {
        return None;
    }
    if !t.chars().all(|c| c.is_alphanumeric() || matches!(c, '.' | '_' | '-' | '+' | '$')) {
        return None;
    }
    file_kind(t).map(|k| (k, t.to_string()))
}

fn is_path(t: &str) -> bool {
    let b = t.as_bytes();
    let drive = b.len() > 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/');
    let unc = t.starts_with("\\\\") && t.len() > 3;
    let posix = (t.starts_with('/') || t.starts_with("~/") || t.starts_with("./") || t.starts_with("../"))
        && t.matches('/').count() >= 2
        && !t.starts_with("//");
    let relative = !t.contains(' ')
        && (t.contains('\\') || t.contains('/'))
        && t.matches(['\\', '/']).count() >= 1
        && ext_of(t.rsplit(['\\', '/']).next().unwrap_or("")).is_some()
        && t.chars().next().is_some_and(char::is_alphanumeric)
        && !t.contains("//");
    (drive || unc || posix || relative) && !t.contains(['*', '?', '<', '>']) && t.chars().filter(|c| c.is_alphanumeric()).count() >= 2
}

/// The path, its file name (with kind) and, for a UNC path, its host.
fn path(t: &str) -> Found {
    let t = t.trim_end_matches(['\\', '/']);
    let mut out = vec![(EntityKind::Path, t.to_string())];
    if let Some(unc) = t.strip_prefix("\\\\") {
        if let Some(h) = unc.split('\\').next().filter(|h| !h.is_empty()) {
            out.push((EntityKind::Host, h.to_lowercase()));
        }
    }
    let name = t.rsplit(['\\', '/']).next().unwrap_or(t);
    if let Some(k) = file_kind(name) {
        out.push((k, name.to_string()));
    }
    out
}

fn host(t: &str) -> Option<String> {
    let h = match t.rsplit_once(':') {
        Some((h, p)) if !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()) => h,
        _ => t,
    };
    let labels: Vec<&str> = h.split('.').collect();
    if labels.len() < 2
        || labels
            .iter()
            .any(|l| l.is_empty() || l.len() > 63 || l.starts_with('-') || l.ends_with('-'))
    {
        return None;
    }
    if !h.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-') || h.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return None;
    }
    let tld = labels.last()?.to_lowercase();
    // A name that is really a file (`app.json`) is not a host.
    (TLDS.contains(&tld.as_str()) && file_kind(h).is_none()).then(|| h.to_lowercase())
}
