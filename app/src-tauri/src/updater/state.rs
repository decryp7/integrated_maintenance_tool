//! The update flow as a pure state machine: check → available → downloading (progress, cancel) →
//! ready (verified) → installing, with typed failures. [`UpdatePhase::next`] is the only way phases
//! change; a step that does not fit the current phase is refused with [`UpdateError::Busy`] (or
//! [`UpdateError::NotAvailable`] in a build without a key) and changes nothing.

use super::error::{UpdateError, MAX_DOWNLOAD_BYTES};
use serde::Serialize;

/// An update found by a check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub version: String,
    pub current_version: String,
    /// Plain text (never markup), bounded.
    pub notes: String,
    /// RFC 3339.
    pub pub_date: Option<String>,
    /// Download size in bytes, when the release says.
    pub size: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum UpdatePhase {
    /// This build cannot verify updates (no public key): checking is off for good.
    Unavailable,
    /// Nothing checked yet in this run.
    Idle,
    Checking,
    UpToDate,
    Available {
        info: UpdateInfo,
    },
    Downloading {
        info: UpdateInfo,
        downloaded: u64,
        total: Option<u64>,
    },
    /// Downloaded and its signature verified.
    #[serde(rename_all = "camelCase")]
    Ready {
        info: UpdateInfo,
        /// Install when IMT quits instead of now.
        install_on_quit: bool,
    },
    Installing {
        info: UpdateInfo,
    },
    /// `info` is kept when there is something to retry.
    Failed {
        error: UpdateError,
        info: Option<UpdateInfo>,
    },
}

/// What happened (a request, or a result of work).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Check,
    Checked(Result<Option<UpdateInfo>, UpdateError>),
    Download,
    Progress { chunk: u64, total: Option<u64> },
    Downloaded(Result<(), UpdateError>),
    Cancel,
    InstallOnQuit(bool),
    Install,
    InstallFailed(UpdateError),
}

fn busy(detail: &str) -> UpdateError {
    UpdateError::Busy { detail: detail.into() }
}

impl UpdatePhase {
    /// The phase after `event`, or why it does not apply now.
    pub fn next(&self, event: Event) -> Result<UpdatePhase, UpdateError> {
        use UpdatePhase as P;
        Ok(match (self, event) {
            (P::Unavailable, _) => return Err(UpdateError::NotAvailable),
            (P::Idle | P::UpToDate | P::Available { .. } | P::Failed { .. }, Event::Check) => P::Checking,
            (P::Checking, Event::Check) => return Err(busy("IMT is already checking for updates.")),
            (P::Downloading { .. } | P::Ready { .. } | P::Installing { .. }, Event::Check) => {
                return Err(busy("An update is already being downloaded or installed."))
            }

            (P::Checking, Event::Checked(Ok(Some(info)))) => P::Available { info },
            (P::Checking, Event::Checked(Ok(None))) => P::UpToDate,
            (P::Checking, Event::Checked(Err(error))) => P::Failed { error, info: None },

            (P::Available { info }, Event::Download) => start_download(info),
            (P::Failed { error, info: Some(info) }, Event::Download) if error.retry_download() => start_download(info),

            (P::Downloading { info, downloaded, total }, Event::Progress { chunk, total: t }) => {
                let downloaded = downloaded.saturating_add(chunk);
                let total = t.or(*total);
                if downloaded > MAX_DOWNLOAD_BYTES || total.is_some_and(|t| t > MAX_DOWNLOAD_BYTES) {
                    too_large(info)
                } else {
                    P::Downloading {
                        info: info.clone(),
                        downloaded,
                        total,
                    }
                }
            }
            (P::Downloading { info, .. }, Event::Downloaded(Ok(()))) => P::Ready {
                info: info.clone(),
                install_on_quit: false,
            },
            (P::Downloading { info, .. }, Event::Downloaded(Err(error))) => P::Failed {
                error,
                info: Some(info.clone()),
            },
            (P::Downloading { info, .. } | P::Ready { info, .. }, Event::Cancel) => P::Available { info: info.clone() },

            (P::Ready { info, .. }, Event::InstallOnQuit(on)) => P::Ready {
                info: info.clone(),
                install_on_quit: on,
            },
            (P::Ready { info, .. }, Event::Install) => P::Installing { info: info.clone() },
            (P::Installing { info }, Event::InstallFailed(error)) => P::Failed {
                error,
                info: Some(info.clone()),
            },

            (_, Event::Download) => return Err(busy("There is no update to download.")),
            (_, Event::Cancel) => return Err(busy("There is no download to cancel.")),
            (_, Event::InstallOnQuit(_) | Event::Install) => return Err(busy("No verified update is ready to install.")),
            // Results arriving after the user moved on (e.g. progress after a cancel) are stale.
            (_, Event::Checked(_) | Event::Progress { .. } | Event::Downloaded(_) | Event::InstallFailed(_)) => {
                return Err(busy("This step no longer applies."))
            }
        })
    }

    /// Checks can start from here (also what the automatic check waits for).
    pub fn can_check(&self) -> bool {
        matches!(
            self,
            UpdatePhase::Idle | UpdatePhase::UpToDate | UpdatePhase::Available { .. } | UpdatePhase::Failed { .. }
        )
    }
}

fn start_download(info: &UpdateInfo) -> UpdatePhase {
    if info.size.is_some_and(|s| s > MAX_DOWNLOAD_BYTES) {
        return too_large(info);
    }
    UpdatePhase::Downloading {
        info: info.clone(),
        downloaded: 0,
        total: info.size,
    }
}

fn too_large(info: &UpdateInfo) -> UpdatePhase {
    UpdatePhase::Failed {
        error: UpdateError::TooLarge { limit: MAX_DOWNLOAD_BYTES },
        info: Some(info.clone()),
    }
}

/// Whether the automatic check should run now: it is on, nothing else is going on, IMT has been
/// running for a minute (not during start-up) and the last check is a day old.
pub fn auto_check_due(auto: bool, phase: &UpdatePhase, uptime_secs: u64, since_last_check_secs: Option<u64>) -> bool {
    const STARTUP_DELAY: u64 = 60;
    const INTERVAL: u64 = 24 * 60 * 60;
    auto && phase.can_check()
        && !matches!(phase, UpdatePhase::Available { .. })
        && uptime_secs >= STARTUP_DELAY
        && since_last_check_secs.is_none_or(|s| s >= INTERVAL)
}
