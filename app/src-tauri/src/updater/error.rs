//! Typed update errors: what went wrong, said for the user, with the next step. The plugin's errors are
//! classified once, here ([`classify`]); everything after works with [`UpdateError`].

use serde::Serialize;

/// Downloads larger than this are refused (a Debrief installer is well under 200 MB).
pub const MAX_DOWNLOAD_BYTES: u64 = 512 * 1024 * 1024;

/// Why checking, downloading or installing an update failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateError {
    /// The build has no update public key, so nothing could be verified: updates are off.
    NotAvailable,
    /// No network (the server's name could not be resolved).
    Offline,
    /// The update server did not answer, timed out or failed.
    Unreachable {
        detail: String,
    },
    /// The server answered without release information (no published release, or a 404).
    NoRelease,
    /// The release has no update for this platform.
    NoPlatform {
        detail: String,
    },
    /// The release information is malformed or does not pass validation; it is not used.
    InvalidManifest {
        detail: String,
    },
    /// The downloaded update does not carry a valid signature from the Debrief signing key: refused.
    SignatureInvalid {
        detail: String,
    },
    /// The update is larger than [`MAX_DOWNLOAD_BYTES`].
    TooLarge {
        limit: u64,
    },
    DiskFull,
    InstallFailed {
        detail: String,
    },
    /// The request does not fit what the updater is doing now (e.g. downloading twice).
    Busy {
        detail: String,
    },
}

/// Which step failed (the same plugin error means different things in each).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Check,
    Download,
    Install,
}

/// The wire form: a stable `kind` for the UI's actions, plus text it shows as is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateErrorDto {
    pub kind: &'static str,
    pub message: String,
    pub next_step: String,
    pub detail: Option<String>,
}

impl UpdateError {
    pub fn kind(&self) -> &'static str {
        match self {
            UpdateError::NotAvailable => "notAvailable",
            UpdateError::Offline => "offline",
            UpdateError::Unreachable { .. } => "unreachable",
            UpdateError::NoRelease => "noRelease",
            UpdateError::NoPlatform { .. } => "noPlatform",
            UpdateError::InvalidManifest { .. } => "invalidManifest",
            UpdateError::SignatureInvalid { .. } => "signatureInvalid",
            UpdateError::TooLarge { .. } => "tooLarge",
            UpdateError::DiskFull => "diskFull",
            UpdateError::InstallFailed { .. } => "installFailed",
            UpdateError::Busy { .. } => "busy",
        }
    }

    pub fn message(&self) -> String {
        match self {
            UpdateError::NotAvailable => "Updates are not available in this build.".into(),
            UpdateError::Offline => "Debrief could not reach the network.".into(),
            UpdateError::Unreachable { .. } => "The update server could not be reached.".into(),
            UpdateError::NoRelease => "The update server has no release information.".into(),
            UpdateError::NoPlatform { .. } => "The latest release has no update for this system.".into(),
            UpdateError::InvalidManifest { .. } => "The update information from the server is invalid, so Debrief ignored it.".into(),
            UpdateError::SignatureInvalid { .. } => "The update's signature is not valid, so Debrief refused to install it.".into(),
            UpdateError::TooLarge { limit } => format!(
                "The update is larger than {}, so Debrief did not download it.",
                debrief_plugin_api::units::bytes(*limit)
            ),
            UpdateError::DiskFull => "There is not enough disk space to install the update.".into(),
            UpdateError::InstallFailed { .. } => "The update could not be installed.".into(),
            UpdateError::Busy { detail } => detail.clone(),
        }
    }

    pub fn next_step(&self) -> &'static str {
        match self {
            UpdateError::NotAvailable => {
                "This build was made without an update signing key, so it cannot verify updates. Download new versions from the releases page."
            }
            UpdateError::Offline => "Check your internet connection or proxy, then try again.",
            UpdateError::Unreachable { .. } => "Try again later. If it keeps failing, download the new version from the releases page.",
            UpdateError::NoRelease => "There may be no published release yet. Try again later or look at the releases page.",
            UpdateError::NoPlatform { .. } => "Download the installer for your system from the releases page.",
            UpdateError::InvalidManifest { .. } => "Try again later; if it persists, report it to the Debrief maintainers.",
            UpdateError::SignatureInvalid { .. } => {
                "The download was discarded. Do not install it by hand; report this to the Debrief maintainers."
            }
            UpdateError::TooLarge { .. } => "Download the installer from the releases page.",
            UpdateError::DiskFull => "Free some disk space, then try again.",
            UpdateError::InstallFailed { .. } => "Try again, or install the new version from the releases page.",
            UpdateError::Busy { .. } => "Wait for the current step to finish.",
        }
    }

    pub fn detail(&self) -> Option<String> {
        match self {
            UpdateError::Unreachable { detail }
            | UpdateError::NoPlatform { detail }
            | UpdateError::InvalidManifest { detail }
            | UpdateError::SignatureInvalid { detail }
            | UpdateError::InstallFailed { detail } => Some(detail.clone()),
            _ => None,
        }
    }

    /// A download can simply be tried again (not after a refused signature: check again first).
    pub fn retry_download(&self) -> bool {
        !matches!(
            self,
            UpdateError::SignatureInvalid { .. } | UpdateError::TooLarge { .. } | UpdateError::NotAvailable
        )
    }

    pub fn to_dto(&self) -> UpdateErrorDto {
        UpdateErrorDto {
            kind: self.kind(),
            message: self.message(),
            next_step: self.next_step().into(),
            detail: self.detail(),
        }
    }
}

impl std::fmt::Display for UpdateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.detail() {
            Some(d) => write!(f, "{} ({d})", self.message()),
            None => f.write_str(&self.message()),
        }
    }
}

impl std::error::Error for UpdateError {}

impl Serialize for UpdateError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.to_dto().serialize(s)
    }
}

/// Update commands reject with the one [`crate::CmdError`]: the kind is its code, the next step its hint.
impl From<UpdateError> for crate::CmdError {
    fn from(e: UpdateError) -> crate::CmdError {
        use crate::ErrorCode as C;
        let code = match &e {
            UpdateError::NotAvailable => C::NotAvailable,
            UpdateError::Offline => C::Offline,
            UpdateError::Unreachable { .. } => C::Unreachable,
            UpdateError::NoRelease => C::NoRelease,
            UpdateError::NoPlatform { .. } => C::NoPlatform,
            UpdateError::InvalidManifest { .. } => C::InvalidManifest,
            UpdateError::SignatureInvalid { .. } => C::SignatureInvalid,
            UpdateError::TooLarge { .. } => C::TooLarge,
            UpdateError::DiskFull => C::DiskFull,
            UpdateError::InstallFailed { .. } => C::InstallFailed,
            UpdateError::Busy { .. } => C::Busy,
        };
        crate::CmdError::new(code, e.message()).hint(e.next_step()).detail(e.detail())
    }
}

/// True for "no space left" on any platform.
pub fn is_disk_full(e: &std::io::Error) -> bool {
    if e.kind() == std::io::ErrorKind::StorageFull {
        return true;
    }
    // ERROR_HANDLE_DISK_FULL, ERROR_DISK_FULL
    #[cfg(windows)]
    let codes: &[i32] = &[39, 112];
    // ENOSPC, EDQUOT
    #[cfg(target_os = "linux")]
    let codes: &[i32] = &[28, 122];
    #[cfg(not(any(windows, target_os = "linux")))]
    let codes: &[i32] = &[28, 69];
    e.raw_os_error().is_some_and(|c| codes.contains(&c))
}

/// A connection failure that means "no network" rather than "server down": name resolution failed.
pub fn looks_offline(error_chain: &str) -> bool {
    let t = error_chain.to_ascii_lowercase();
    [
        "dns error",
        "failed to lookup address",
        "no such host is known",
        "name or service not known",
        "nodename nor servname",
        "temporary failure in name resolution",
        "network is unreachable",
        "no route to host",
    ]
    .iter()
    .any(|p| t.contains(p))
}

/// The whole `source()` chain as one line.
pub fn chain(e: &(dyn std::error::Error + 'static)) -> String {
    let mut out = e.to_string();
    let mut cur = e.source();
    let mut depth = 0;
    while let Some(s) = cur {
        if depth >= 8 {
            break;
        }
        out.push_str(": ");
        out.push_str(&s.to_string());
        cur = s.source();
        depth += 1;
    }
    out
}

/// Network failure of a request (connect, timeout, status, body).
pub fn network(connect: bool, chain_text: String) -> UpdateError {
    if connect && looks_offline(&chain_text) {
        UpdateError::Offline
    } else {
        UpdateError::Unreachable { detail: chain_text }
    }
}

/// Classifies an updater plugin error.
pub fn classify(e: &tauri_plugin_updater::Error, step: Step) -> UpdateError {
    use tauri_plugin_updater::Error as E;
    let detail = e.to_string();
    match e {
        E::Reqwest(r) => {
            let text = chain(r);
            if step == Step::Check && r.is_decode() {
                UpdateError::InvalidManifest { detail: text }
            } else {
                network(r.is_connect(), text)
            }
        }
        E::Network(_) => UpdateError::Unreachable { detail },
        E::ReleaseNotFound => UpdateError::NoRelease,
        E::TargetNotFound(_) | E::TargetsNotFound(_) | E::UnsupportedArch | E::UnsupportedOs => UpdateError::NoPlatform { detail },
        E::Serialization(_) | E::Semver(_) | E::UrlParse(_) | E::InsecureTransportProtocol => UpdateError::InvalidManifest { detail },
        E::Minisign(_) | E::Base64(_) | E::SignatureUtf8(_) | E::SignedVersionMismatch { .. } | E::MissingSignedVersion => {
            UpdateError::SignatureInvalid { detail }
        }
        E::Io(io) if is_disk_full(io) => UpdateError::DiskFull,
        E::EmptyEndpoints => UpdateError::NotAvailable,
        _ if step == Step::Install => UpdateError::InstallFailed { detail },
        _ => UpdateError::Unreachable { detail },
    }
}
