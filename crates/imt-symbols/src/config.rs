//! Where symbols come from: the user's search path, the cache (symstore layout) and, when the user enables it, a
//! symbol server. Parsed and checked once, here.

use crate::pe::PdbId;
use crate::SymbolError;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Most folders in a search path.
pub const MAX_PATHS: usize = 32;
/// Longest folder in a search path.
pub const MAX_PATH_LEN: usize = 1024;

/// Where symbols are looked for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SymbolConfig {
    /// Folders with PDBs (searched first, as given).
    pub paths: Vec<PathBuf>,
    /// The cache in symstore layout (`<pdb>/<key>/<pdb>`): searched, and downloads go there.
    pub cache: Option<PathBuf>,
    /// A symbol server to ask for PDBs the paths and the cache do not have (`None`: never).
    pub server: Option<ServerConfig>,
}

/// A symbol server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    /// Base address (`https://msdl.microsoft.com/download/symbols`).
    pub url: String,
    /// Time one download may take.
    pub timeout: Duration,
    /// Largest PDB downloaded.
    pub max_bytes: u64,
}

impl ServerConfig {
    /// The Microsoft public symbol server.
    pub fn microsoft() -> ServerConfig {
        ServerConfig {
            url: "https://msdl.microsoft.com/download/symbols".into(),
            timeout: Duration::from_secs(120),
            max_bytes: 512 << 20,
        }
    }

    /// The address must be `https://` (plain `http://` only to the loopback host, for tests).
    pub fn check(&self) -> Result<(), SymbolError> {
        let u = self.url.trim_end_matches('/');
        let loopback = ["http://127.0.0.1", "http://localhost", "http://[::1]"].iter().any(|p| {
            u.strip_prefix(p)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with([':', '/']))
        });
        if !(u.starts_with("https://") || loopback) || u.contains(char::is_whitespace) || u.len() > 512 {
            return Err(SymbolError::InvalidPath(format!(
                "the symbol server address must start with https:// (got {})",
                self.url
            )));
        }
        Ok(())
    }
}

/// Parses a search path as the user types it: absolute folders separated by `;`. Symbol-server syntax
/// (`srv*…`, `cache*…`) is refused with a pointer to the setting that does it.
pub fn parse_search_path(text: &str) -> Result<Vec<PathBuf>, SymbolError> {
    let mut out = Vec::new();
    for part in text.split(';').map(str::trim).filter(|p| !p.is_empty()) {
        let lower = part.to_ascii_lowercase();
        if lower.starts_with("srv*") || lower.starts_with("cache*") || lower.starts_with("symsrv*") || part.contains('*') {
            return Err(SymbolError::InvalidPath(format!(
                "\"{part}\": symbol server entries are not used here; enable \"Download symbols from Microsoft\" instead"
            )));
        }
        if part.chars().count() > MAX_PATH_LEN || part.contains('\0') {
            return Err(SymbolError::InvalidPath(format!(
                "a folder may have at most {MAX_PATH_LEN} characters"
            )));
        }
        let p = PathBuf::from(part);
        if !is_absolute(&p) {
            return Err(SymbolError::InvalidPath(format!("\"{part}\" is not an absolute folder")));
        }
        if out.len() == MAX_PATHS {
            return Err(SymbolError::InvalidPath(format!("at most {MAX_PATHS} folders")));
        }
        out.push(p);
    }
    Ok(out)
}

/// Absolute on Windows (`C:\…`, `\\server\share`) or elsewhere (`/…`).
fn is_absolute(p: &Path) -> bool {
    if p.is_absolute() {
        return true;
    }
    // Windows paths are checked the same way on every platform (settings are portable).
    let s = p.to_string_lossy();
    let b = s.as_bytes();
    (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/'))
        || s.starts_with(r"\\")
        || s.starts_with('/')
}

/// A PDB file name taken from an image (untrusted): one path component of safe characters ending in `.pdb`.
pub fn safe_pdb_name(id: &PdbId) -> Option<&str> {
    let name = id.file_name();
    let ok = !name.is_empty()
        && name.len() <= 255
        && name.to_ascii_lowercase().ends_with(".pdb")
        && name != ".pdb"
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '+' | ' ' | '(' | ')'));
    ok.then_some(name)
}

/// Where the cache keeps `id`'s PDB: `<cache>/<pdb>/<key>/<pdb>` (`None` for an unsafe name).
pub fn cache_file(cache: &Path, id: &PdbId) -> Option<PathBuf> {
    let name = safe_pdb_name(id)?;
    Some(cache.join(name).join(id.key()).join(name))
}

impl SymbolConfig {
    /// The search path given to DbgHelp: the user's folders, then `extra` (the cache folder of a module).
    pub fn search_path(&self, extra: &[PathBuf]) -> String {
        self.paths
            .iter()
            .chain(extra)
            .map(|p| p.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(";")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(path: &str) -> PdbId {
        PdbId {
            guid: [1; 16],
            age: 3,
            path: path.into(),
        }
    }

    #[test]
    fn search_paths_are_absolute_folders_only() {
        let p = parse_search_path(r" C:\syms ; ;\\srv\share\pdbs;/opt/syms").unwrap();
        assert_eq!(p.len(), 3);
        assert!(parse_search_path("").unwrap().is_empty());
        for bad in [
            "relative\\dir",
            "srv*C:\\cache*https://msdl.microsoft.com/download/symbols",
            "cache*C:\\c",
            "C:\\a*b",
        ] {
            assert!(parse_search_path(bad).is_err(), "{bad}");
        }
        assert!(parse_search_path(&format!("C:\\{}", "a".repeat(MAX_PATH_LEN))).is_err());
        let many = vec!["C:\\a"; MAX_PATHS + 1].join(";");
        assert!(parse_search_path(&many).is_err());
        let c = SymbolConfig {
            paths: p,
            ..SymbolConfig::default()
        };
        assert!(c.search_path(&[PathBuf::from("X")]).ends_with(";X"));
    }

    #[test]
    fn server_addresses_are_https_except_loopback() {
        ServerConfig::microsoft().check().unwrap();
        for ok in ["http://127.0.0.1:8080/s", "http://localhost", "https://syms.example.com/"] {
            ServerConfig {
                url: ok.into(),
                ..ServerConfig::microsoft()
            }
            .check()
            .unwrap();
        }
        for bad in ["http://example.com", "ftp://x", "http://127.0.0.1.evil.com/", "https://a b"] {
            assert!(
                ServerConfig {
                    url: bad.into(),
                    ..ServerConfig::microsoft()
                }
                .check()
                .is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn pdb_names_from_images_cannot_escape_the_cache() {
        let cache = Path::new("cache");
        let f = cache_file(cache, &id(r"D:\build\busy.pdb")).unwrap();
        assert_eq!(f, cache.join("busy.pdb").join(id("x").key()).join("busy.pdb"));
        for bad in ["..\\..\\evil.pdb", "../x.pdb", "a.dll", ".pdb", "con\0.pdb", "a:b.pdb", ""] {
            let path = if bad.contains("..") {
                bad.to_string()
            } else {
                format!("C:\\b\\{bad}")
            };
            let name = id(&path);
            let r = cache_file(cache, &name);
            assert!(r.is_none() || !r.as_ref().unwrap().to_string_lossy().contains(".."), "{bad}: {r:?}");
        }
        assert!(safe_pdb_name(&id("..")).is_none());
        assert!(is_absolute(Path::new(r"\\srv\s")) && is_absolute(Path::new("C:/x")));
    }
}
