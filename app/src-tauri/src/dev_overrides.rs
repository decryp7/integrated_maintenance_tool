//! Debug-build-only environment overrides for automated runs of the real app (the WebdriverIO smoke tests in
//! `app/e2e-tauri`). Release builds never read these variables.
//!
//! - `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`: msedgedriver turns on WebView2's remote debugging through this
//!   variable, but WebView2 ignores it for elevated host processes. It still honours arguments given through
//!   the API, so debug builds pass the value to every webview they create, merged with wry's defaults.
//! - `DEBRIEF_DATA_DIR`: an absolute folder that replaces Debrief's data, config, cache and log folders and the
//!   webview's user data folder, so a test run never touches the user's profile (see [`crate::app_dirs`]). It
//!   also gives the run its own app identifier ([`instance_identifier`]), so the single-instance lock never hands
//!   a test launch to another run or to an installed Debrief.
//! - `DEBRIEF_OPEN_PATHS`: absolute paths (separated like `PATH`) to open at start-up, as if given on the command
//!   line. msedgedriver turns every launch argument into a `--switch` (a path arrives as `--c:\…`), so a driven
//!   app on Windows cannot receive paths on its command line.
//!
//! The values are parsed once, at start-up ([`init`]); an invalid one stops the app with a message saying what
//! to fix, instead of silently falling back to the user's profile.

use std::ffi::OsString;
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

use tauri::{Manager, Runtime, WebviewWindowBuilder};

/// The variable msedgedriver sets for WebView2.
pub const BROWSER_ARGS_VAR: &str = "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS";
/// The variable that relocates all of Debrief's folders.
pub const DATA_DIR_VAR: &str = "DEBRIEF_DATA_DIR";
/// The variable listing paths to open at start-up.
pub const OPEN_PATHS_VAR: &str = "DEBRIEF_OPEN_PATHS";

/// The arguments wry passes when none are given; setting `additional_browser_args` replaces them, so they are
/// always part of the merged value.
pub const WRY_DEFAULT_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection";

/// Longest accepted `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` value, in bytes.
pub const MAX_BROWSER_ARGS_LEN: usize = 4096;
/// Most `--flag[=value]` tokens accepted.
pub const MAX_BROWSER_ARGS: usize = 64;
/// Longest accepted `DEBRIEF_DATA_DIR` (and `DEBRIEF_OPEN_PATHS` entry), in bytes.
pub const MAX_DATA_DIR_LEN: usize = 1024;
/// Most `DEBRIEF_OPEN_PATHS` entries accepted.
pub const MAX_OPEN_PATHS: usize = 16;

const DISABLE_FEATURES: &str = "--disable-features";

/// Why an override was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverrideError {
    BrowserArgsTooLong(usize),
    TooManyBrowserArgs(usize),
    BadBrowserArg(String),
    DataDirNotUnicode,
    DataDirTooLong(usize),
    DataDirNotAbsolute(PathBuf),
    DataDirHasParent(PathBuf),
    OpenPathsNotUnicode,
    TooManyOpenPaths(usize),
    OpenPathTooLong(usize),
    OpenPathNotAbsolute(PathBuf),
}

impl fmt::Display for OverrideError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BrowserArgsTooLong(n) => write!(
                f,
                "{BROWSER_ARGS_VAR} is {n} bytes long; at most {MAX_BROWSER_ARGS_LEN} are allowed"
            ),
            Self::TooManyBrowserArgs(n) => write!(f, "{BROWSER_ARGS_VAR} has {n} arguments; at most {MAX_BROWSER_ARGS} are allowed"),
            Self::BadBrowserArg(a) => {
                write!(
                    f,
                    "{BROWSER_ARGS_VAR} contains `{a}`; only `--flag` or `--flag=value` arguments (no quotes or spaces) are allowed"
                )
            }
            Self::DataDirNotUnicode => write!(f, "{DATA_DIR_VAR} is not valid Unicode"),
            Self::DataDirTooLong(n) => write!(f, "{DATA_DIR_VAR} is {n} bytes long; at most {MAX_DATA_DIR_LEN} are allowed"),
            Self::DataDirNotAbsolute(p) => write!(f, "{DATA_DIR_VAR} must be an absolute path, not `{}`", p.display()),
            Self::DataDirHasParent(p) => write!(f, "{DATA_DIR_VAR} must not contain `..`: `{}`", p.display()),
            Self::OpenPathsNotUnicode => write!(f, "{OPEN_PATHS_VAR} is not valid Unicode"),
            Self::TooManyOpenPaths(n) => write!(f, "{OPEN_PATHS_VAR} lists {n} paths; at most {MAX_OPEN_PATHS} are allowed"),
            Self::OpenPathTooLong(n) => write!(
                f,
                "{OPEN_PATHS_VAR} has a path of {n} bytes; at most {MAX_DATA_DIR_LEN} are allowed"
            ),
            Self::OpenPathNotAbsolute(p) => write!(f, "{OPEN_PATHS_VAR} must list absolute paths, not `{}`", p.display()),
        }
    }
}

impl std::error::Error for OverrideError {}

/// The overrides in effect for this process (empty in release builds).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Overrides {
    /// The full `additional_browser_args` value (wry's defaults merged with the variable's flags).
    pub browser_args: Option<String>,
    /// The root that replaces Debrief's folders.
    pub data_dir: Option<PathBuf>,
    /// Paths to open at start-up, after the command line's.
    pub open_paths: Vec<PathBuf>,
}

impl Overrides {
    /// Parses the overrides from `lookup` when `enabled`; when not, the variables are not even read.
    pub fn from_lookup(enabled: bool, lookup: impl Fn(&str) -> Option<OsString>) -> Result<Self, OverrideError> {
        if !enabled {
            return Ok(Self::default());
        }
        let browser_args = match lookup(BROWSER_ARGS_VAR) {
            // A non-Unicode value cannot be a valid flag list.
            Some(v) => merge_browser_args(
                &v.into_string()
                    .map_err(|v| OverrideError::BadBrowserArg(v.to_string_lossy().into_owned()))?,
            )?,
            None => None,
        };
        let data_dir = match lookup(DATA_DIR_VAR) {
            Some(v) => parse_data_dir(&v)?,
            None => None,
        };
        let open_paths = match lookup(OPEN_PATHS_VAR) {
            Some(v) => parse_open_paths(&v)?,
            None => Vec::new(),
        };
        Ok(Self {
            browser_args,
            data_dir,
            open_paths,
        })
    }

    /// The process environment's overrides: only debug builds read it.
    pub fn from_env() -> Result<Self, OverrideError> {
        Self::from_lookup(cfg!(debug_assertions), |k| std::env::var_os(k))
    }
}

static OVERRIDES: OnceLock<Overrides> = OnceLock::new();

/// Parses the environment's overrides once; call first thing in `run`. An error names the variable to fix.
pub fn init() -> Result<(), OverrideError> {
    let parsed = Overrides::from_env()?;
    let _ = OVERRIDES.set(parsed);
    Ok(())
}

/// The overrides in effect (none before [`init`], e.g. in unit tests).
pub fn current() -> &'static Overrides {
    static NONE: Overrides = Overrides {
        browser_args: None,
        data_dir: None,
        open_paths: Vec::new(),
    };
    OVERRIDES.get().unwrap_or(&NONE)
}

/// The app identifier for this process: with a relocated data folder, `base` plus a suffix derived from that
/// folder, so parallel test runs on one machine (and an installed Debrief) each get their own single-instance lock;
/// otherwise `base` unchanged. The suffix is a valid D-Bus name element (the lock's name on Linux).
pub fn instance_identifier(base: &str, data_dir: Option<&Path>) -> String {
    match data_dir {
        Some(dir) => format!("{base}.run_{:016x}", fnv1a(dir.as_os_str().as_encoded_bytes())),
        None => base.to_string(),
    }
}

/// FNV-1a: a stable hash, so every process of one run derives the same identifier.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes
        .iter()
        .fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3))
}

/// Applies the overrides to a window being built: every webview of the app must get the same browser arguments
/// and user data folder (WebView2 rejects environments that disagree).
pub fn apply<'a, R: Runtime, M: Manager<R>>(builder: WebviewWindowBuilder<'a, R, M>) -> WebviewWindowBuilder<'a, R, M> {
    apply_overrides(builder, current())
}

fn apply_overrides<'a, R: Runtime, M: Manager<R>>(
    mut builder: WebviewWindowBuilder<'a, R, M>,
    o: &Overrides,
) -> WebviewWindowBuilder<'a, R, M> {
    if let Some(args) = &o.browser_args {
        builder = builder.additional_browser_args(args);
    }
    if let Some(dir) = &o.data_dir {
        builder = builder.data_directory(crate::app_dirs::webview_data_dir(dir));
    }
    builder
}

/// Merges the variable's flags with [`WRY_DEFAULT_ARGS`] into one `additional_browser_args` value. Blank input
/// means no override (`None`: wry keeps its defaults). Every token must be `--flag` or `--flag=value`; a
/// `--disable-features` list is combined with wry's rather than replacing it.
pub fn merge_browser_args(extra: &str) -> Result<Option<String>, OverrideError> {
    if extra.len() > MAX_BROWSER_ARGS_LEN {
        return Err(OverrideError::BrowserArgsTooLong(extra.len()));
    }
    let tokens: Vec<&str> = extra.split_ascii_whitespace().collect();
    if tokens.is_empty() {
        return Ok(None);
    }
    if tokens.len() > MAX_BROWSER_ARGS {
        return Err(OverrideError::TooManyBrowserArgs(tokens.len()));
    }
    let mut features: Vec<String> = feature_list(WRY_DEFAULT_ARGS);
    let mut rest = Vec::new();
    for token in tokens {
        if !valid_flag(token) {
            return Err(OverrideError::BadBrowserArg(token.to_string()));
        }
        if token.starts_with(&format!("{DISABLE_FEATURES}=")) {
            for f in feature_list(token) {
                if !features.contains(&f) {
                    features.push(f);
                }
            }
        } else {
            rest.push(token);
        }
    }
    let mut out = format!("{DISABLE_FEATURES}={}", features.join(","));
    for token in rest {
        out.push(' ');
        out.push_str(token);
    }
    Ok(Some(out))
}

/// The comma-separated values of a `--disable-features=a,b` token.
fn feature_list(token: &str) -> Vec<String> {
    token
        .split_once('=')
        .map(|(_, v)| v.split(',').filter(|f| !f.is_empty()).map(str::to_string).collect())
        .unwrap_or_default()
}

/// `--name` or `--name=value`: the name is ASCII alphanumerics, `-`, `_` or `.`; the value is printable ASCII
/// other than quotes (whitespace already split the tokens).
fn valid_flag(token: &str) -> bool {
    let Some(body) = token.strip_prefix("--") else { return false };
    let (name, value) = match body.split_once('=') {
        Some((n, v)) => (n, Some(v)),
        None => (body, None),
    };
    let name_ok = name.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    let value_ok = value.is_none_or(|v| v.chars().all(|c| c.is_ascii_graphic() && !matches!(c, '"' | '\'' | '`')));
    name_ok && value_ok
}

/// `DEBRIEF_DATA_DIR` as an absolute path without `..`; blank means no override.
pub fn parse_data_dir(value: &std::ffi::OsStr) -> Result<Option<PathBuf>, OverrideError> {
    let text = value.to_str().ok_or(OverrideError::DataDirNotUnicode)?.trim();
    if text.is_empty() {
        return Ok(None);
    }
    if text.len() > MAX_DATA_DIR_LEN {
        return Err(OverrideError::DataDirTooLong(text.len()));
    }
    let path = Path::new(text);
    if !path.is_absolute() {
        return Err(OverrideError::DataDirNotAbsolute(path.to_path_buf()));
    }
    if path.components().any(|c| c == Component::ParentDir) {
        return Err(OverrideError::DataDirHasParent(path.to_path_buf()));
    }
    Ok(Some(path.to_path_buf()))
}

/// `DEBRIEF_OPEN_PATHS` split like `PATH`: absolute paths only; blank entries are skipped. They are checked for
/// existence and canonicalized later, like command-line paths ([`crate::instance::existing`]).
pub fn parse_open_paths(value: &std::ffi::OsStr) -> Result<Vec<PathBuf>, OverrideError> {
    let text = value.to_str().ok_or(OverrideError::OpenPathsNotUnicode)?;
    let paths: Vec<PathBuf> = std::env::split_paths(text).filter(|p| !p.as_os_str().is_empty()).collect();
    if paths.len() > MAX_OPEN_PATHS {
        return Err(OverrideError::TooManyOpenPaths(paths.len()));
    }
    for p in &paths {
        let len = p.as_os_str().len();
        if len > MAX_DATA_DIR_LEN {
            return Err(OverrideError::OpenPathTooLong(len));
        }
        if !p.is_absolute() {
            return Err(OverrideError::OpenPathNotAbsolute(p.clone()));
        }
    }
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn abs(p: &str) -> String {
        if cfg!(windows) {
            format!("C:\\{p}")
        } else {
            format!("/{p}")
        }
    }

    fn lookup(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let map: HashMap<String, OsString> = vars.iter().map(|(k, v)| (k.to_string(), OsString::from(v))).collect();
        move |k| map.get(k).cloned()
    }

    #[test]
    fn blank_args_keep_wrys_defaults() {
        assert_eq!(merge_browser_args(""), Ok(None));
        assert_eq!(merge_browser_args("  \t "), Ok(None));
    }

    #[test]
    fn msedgedriver_args_are_appended_to_the_defaults() {
        let merged = merge_browser_args("--remote-debugging-port=0 --enable-automation")
            .unwrap()
            .unwrap();
        assert_eq!(merged, format!("{WRY_DEFAULT_ARGS} --remote-debugging-port=0 --enable-automation"));
    }

    #[test]
    fn disable_features_lists_are_merged_not_replaced() {
        let merged = merge_browser_args("--disable-features=Foo,msPdfOOUI --x").unwrap().unwrap();
        assert_eq!(merged, "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection,Foo --x");
    }

    #[test]
    fn values_may_hold_paths_and_urls() {
        let merged = merge_browser_args(r"--user-data-dir=C:\Temp\scoped_dir1_2 --proxy-server=http://127.0.0.1:8080").unwrap();
        assert!(merged
            .unwrap()
            .ends_with(r"--user-data-dir=C:\Temp\scoped_dir1_2 --proxy-server=http://127.0.0.1:8080"));
    }

    #[test]
    fn anything_but_flags_is_rejected() {
        for bad in [
            "remote-debugging-port=0",
            "-x",
            "--",
            "--=1",
            "--a b",
            "--a=\"q\"",
            "--a='q'",
            "--a=`q`",
            "--a!b",
            "--é",
        ] {
            let first = bad.split_ascii_whitespace().find(|t| !valid_flag(t)).unwrap_or(bad);
            assert_eq!(
                merge_browser_args(bad),
                Err(OverrideError::BadBrowserArg(first.to_string())),
                "{bad}"
            );
        }
    }

    #[test]
    fn size_limits_are_enforced() {
        let long = format!("--a={}", "x".repeat(MAX_BROWSER_ARGS_LEN));
        assert_eq!(merge_browser_args(&long), Err(OverrideError::BrowserArgsTooLong(long.len())));
        let many = vec!["--a"; MAX_BROWSER_ARGS + 1].join(" ");
        assert_eq!(
            merge_browser_args(&many),
            Err(OverrideError::TooManyBrowserArgs(MAX_BROWSER_ARGS + 1))
        );
        assert!(merge_browser_args(&vec!["--a"; MAX_BROWSER_ARGS].join(" ")).is_ok());
    }

    #[test]
    fn data_dir_must_be_absolute_without_parents() {
        assert_eq!(parse_data_dir("".as_ref()), Ok(None));
        assert_eq!(
            parse_data_dir(abs("debrief-e2e").as_ref()),
            Ok(Some(PathBuf::from(abs("debrief-e2e"))))
        );
        assert_eq!(
            parse_data_dir("relative/dir".as_ref()),
            Err(OverrideError::DataDirNotAbsolute("relative/dir".into()))
        );
        let up = abs("a/../b");
        assert_eq!(parse_data_dir(up.as_ref()), Err(OverrideError::DataDirHasParent(up.clone().into())));
        let long = abs(&"x".repeat(MAX_DATA_DIR_LEN));
        assert_eq!(parse_data_dir(long.as_ref()), Err(OverrideError::DataDirTooLong(long.len())));
    }

    #[cfg(unix)]
    #[test]
    fn non_unicode_values_are_rejected() {
        use std::os::unix::ffi::OsStringExt;
        let bad = OsString::from_vec(vec![b'/', 0xff]);
        assert_eq!(parse_data_dir(&bad), Err(OverrideError::DataDirNotUnicode));
        assert_eq!(parse_open_paths(&bad), Err(OverrideError::OpenPathsNotUnicode));
        let o = Overrides::from_lookup(true, |k| (k == BROWSER_ARGS_VAR).then(|| bad.clone()));
        assert!(matches!(o, Err(OverrideError::BadBrowserArg(_))));
    }

    #[test]
    fn open_paths_are_absolute_and_bounded() {
        let sep = if cfg!(windows) { ";" } else { ":" };
        let (a, b) = (abs("logs"), abs("more logs"));
        let both = format!("{a}{sep}{sep}{b}{sep}");
        assert_eq!(parse_open_paths(both.as_ref()), Ok(vec![PathBuf::from(&a), PathBuf::from(&b)]));
        assert_eq!(parse_open_paths("".as_ref()), Ok(vec![]));
        assert_eq!(
            parse_open_paths("rel".as_ref()),
            Err(OverrideError::OpenPathNotAbsolute("rel".into()))
        );
        let many = vec![a.clone(); MAX_OPEN_PATHS + 1].join(sep);
        assert_eq!(
            parse_open_paths(many.as_ref()),
            Err(OverrideError::TooManyOpenPaths(MAX_OPEN_PATHS + 1))
        );
        let long = abs(&"x".repeat(MAX_DATA_DIR_LEN));
        assert_eq!(parse_open_paths(long.as_ref()), Err(OverrideError::OpenPathTooLong(long.len())));
    }

    #[test]
    fn enabled_lookup_parses_every_variable() {
        let dir = abs("debrief-e2e");
        let o = Overrides::from_lookup(
            true,
            lookup(&[(BROWSER_ARGS_VAR, "--remote-debugging-port=0"), (DATA_DIR_VAR, &dir)]),
        )
        .unwrap();
        assert_eq!(
            o.browser_args.as_deref(),
            Some(format!("{WRY_DEFAULT_ARGS} --remote-debugging-port=0").as_str())
        );
        assert_eq!(o.data_dir, Some(PathBuf::from(dir)));
        assert!(o.open_paths.is_empty());
        let logs = abs("logs");
        let o = Overrides::from_lookup(true, lookup(&[(OPEN_PATHS_VAR, &logs)])).unwrap();
        assert_eq!(o.open_paths, vec![PathBuf::from(logs)]);
        assert!(Overrides::from_lookup(true, lookup(&[(OPEN_PATHS_VAR, "rel")])).is_err());
        assert_eq!(Overrides::from_lookup(true, lookup(&[])), Ok(Overrides::default()));
        assert!(Overrides::from_lookup(true, lookup(&[(DATA_DIR_VAR, "rel")])).is_err());
        assert!(Overrides::from_lookup(true, lookup(&[(BROWSER_ARGS_VAR, "bad")])).is_err());
    }

    #[test]
    fn disabled_lookup_ignores_the_environment_entirely() {
        let o = Overrides::from_lookup(false, |k| panic!("{k} must not be read when disabled"));
        assert_eq!(o, Ok(Overrides::default()));
    }

    /// The release path: whatever the environment holds, a release build has no overrides.
    #[test]
    fn from_env_follows_the_build_profile() {
        let enabled = cfg!(debug_assertions);
        let o = Overrides::from_lookup(
            enabled,
            lookup(&[
                (BROWSER_ARGS_VAR, "--remote-debugging-port=0"),
                (DATA_DIR_VAR, "rel"),
                (OPEN_PATHS_VAR, "rel"),
            ]),
        );
        if enabled {
            assert!(o.is_err())
        } else {
            assert_eq!(o, Ok(Overrides::default()))
        }
    }

    #[cfg(not(debug_assertions))]
    #[test]
    fn release_builds_ignore_the_variables() {
        // No other test of this binary reads these variables in release builds.
        std::env::set_var(BROWSER_ARGS_VAR, "not a flag");
        std::env::set_var(DATA_DIR_VAR, "relative");
        std::env::set_var(OPEN_PATHS_VAR, "relative");
        assert_eq!(Overrides::from_env(), Ok(Overrides::default()));
    }

    #[test]
    fn errors_name_the_variable() {
        for e in [
            OverrideError::BrowserArgsTooLong(1),
            OverrideError::TooManyBrowserArgs(1),
            OverrideError::BadBrowserArg("x".into()),
        ] {
            assert!(e.to_string().contains(BROWSER_ARGS_VAR));
        }
        for e in [
            OverrideError::DataDirNotUnicode,
            OverrideError::DataDirTooLong(1),
            OverrideError::DataDirNotAbsolute("r".into()),
            OverrideError::DataDirHasParent("..".into()),
        ] {
            assert!(e.to_string().contains(DATA_DIR_VAR));
        }
        for e in [
            OverrideError::OpenPathsNotUnicode,
            OverrideError::TooManyOpenPaths(1),
            OverrideError::OpenPathTooLong(1),
            OverrideError::OpenPathNotAbsolute("r".into()),
        ] {
            assert!(e.to_string().contains(OPEN_PATHS_VAR));
        }
    }

    #[test]
    fn current_is_empty_before_init() {
        // Unit tests never call `init`, so the shell behaves as without overrides.
        if OVERRIDES.get().is_none() {
            assert_eq!(current(), &Overrides::default());
        }
    }

    #[test]
    fn a_relocated_data_folder_gets_its_own_identifier() {
        let base = "net.decryptology.debrief";
        assert_eq!(instance_identifier(base, None), base);
        let (a, b) = (PathBuf::from(abs("debrief-e2e-a")), PathBuf::from(abs("debrief-e2e-b")));
        let id_a = instance_identifier(base, Some(&a));
        assert_eq!(id_a, instance_identifier(base, Some(&a)), "stable across processes of one run");
        assert_ne!(id_a, instance_identifier(base, Some(&b)), "parallel runs do not share the lock");
        let suffix = id_a.strip_prefix(&format!("{base}.")).unwrap();
        assert!(suffix.starts_with("run_") && suffix.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
    }

    #[test]
    fn apply_sets_args_and_data_folder_on_the_builder() {
        let app = crate::test_support::test_app_with_commands(None);
        let dir = std::env::temp_dir().join("debrief-dev-overrides-test");
        let o = Overrides {
            browser_args: Some(WRY_DEFAULT_ARGS.into()),
            data_dir: Some(dir),
            open_paths: vec![],
        };
        let w = apply_overrides(
            tauri::WebviewWindowBuilder::new(&app, "w-overrides", tauri::WebviewUrl::default()),
            &o,
        )
        .build()
        .unwrap();
        assert_eq!(w.label(), "w-overrides");
        let none = apply(tauri::WebviewWindowBuilder::new(
            &app,
            "w-no-overrides",
            tauri::WebviewUrl::default(),
        ))
        .build();
        assert!(none.is_ok());
    }
}
