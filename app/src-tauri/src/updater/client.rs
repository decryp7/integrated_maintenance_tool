//! The port to the update server ([`UpdateClient`]) and its adapter over `tauri-plugin-updater`
//! ([`PluginClient`]). The plugin downloads, verifies the minisign signature against the embedded
//! public key (and, with `requireSignedVersion`, that the signature was made for the announced
//! version) and installs; IMT validates the release information on top ([`super::manifest`]).

use super::error::{classify, Step, UpdateError};
use super::manifest;
use super::state::UpdateInfo;
use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;
use std::time::Duration;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
/// Called with each downloaded chunk's size and the total size when known.
pub type OnChunk = Box<dyn FnMut(u64, Option<u64>) + Send>;

/// Talks to the update server. Implementations keep the update found by the last check and the
/// verified bytes of the last download.
pub trait UpdateClient: Send + Sync {
    /// `Ok(None)`: up to date.
    fn check(&self) -> BoxFuture<'_, Result<Option<UpdateInfo>, UpdateError>>;
    /// Downloads and verifies the update found by the last check.
    fn download(&self, on_chunk: OnChunk) -> BoxFuture<'_, Result<(), UpdateError>>;
    /// Installs the verified download. On Windows this starts the installer and ends the process;
    /// `relaunch` says whether the installer starts IMT again.
    fn install(&self, relaunch: bool) -> Result<(), UpdateError>;
    /// Forgets the downloaded bytes.
    fn discard(&self);
}

/// How long a check or a stalled download waits for the server.
const TIMEOUT: Duration = Duration::from_secs(30);

pub struct PluginClient<R: tauri::Runtime> {
    app: tauri::AppHandle<R>,
    pubkey: String,
    found: Mutex<Option<tauri_plugin_updater::Update>>,
    bytes: Mutex<Option<Vec<u8>>>,
}

impl<R: tauri::Runtime> PluginClient<R> {
    pub fn new(app: tauri::AppHandle<R>, pubkey: String) -> PluginClient<R> {
        PluginClient {
            app,
            pubkey,
            found: Mutex::new(None),
            bytes: Mutex::new(None),
        }
    }

    fn found(&self) -> Option<tauri_plugin_updater::Update> {
        self.found.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
}

/// What IMT shows about an update the plugin found, from its validated release information.
pub fn describe(update: &tauri_plugin_updater::Update) -> Result<UpdateInfo, UpdateError> {
    let parsed = manifest::from_value(&update.raw_json).map_err(|detail| UpdateError::InvalidManifest { detail })?;
    let size = parsed.platform_by_url(update.download_url.as_str()).and_then(|p| p.size);
    Ok(UpdateInfo {
        version: update.version.clone(),
        current_version: update.current_version.clone(),
        notes: parsed.notes,
        pub_date: parsed.pub_date,
        size,
    })
}

impl<R: tauri::Runtime> UpdateClient for PluginClient<R> {
    fn check(&self) -> BoxFuture<'_, Result<Option<UpdateInfo>, UpdateError>> {
        Box::pin(async move {
            use tauri_plugin_updater::UpdaterExt;
            let updater = self
                .app
                .updater_builder()
                .pubkey(self.pubkey.clone())
                .timeout(TIMEOUT)
                .build()
                .map_err(|e| classify(&e, Step::Check))?;
            let found = updater.check().await.map_err(|e| classify(&e, Step::Check))?;
            let info = found.as_ref().map(describe).transpose()?;
            *self.found.lock().unwrap_or_else(|p| p.into_inner()) = found;
            self.discard();
            Ok(info)
        })
    }

    fn download(&self, mut on_chunk: OnChunk) -> BoxFuture<'_, Result<(), UpdateError>> {
        Box::pin(async move {
            let update = self.found().ok_or(UpdateError::Busy {
                detail: "There is no update to download.".into(),
            })?;
            let bytes = update
                .download(|n, total| on_chunk(n as u64, total), || {})
                .await
                .map_err(|e| classify(&e, Step::Download))?;
            *self.bytes.lock().unwrap_or_else(|p| p.into_inner()) = Some(bytes);
            Ok(())
        })
    }

    fn install(&self, relaunch: bool) -> Result<(), UpdateError> {
        let update = self.found().ok_or_else(|| UpdateError::InstallFailed {
            detail: "the update is no longer known".into(),
        })?;
        let bytes = self
            .bytes
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
            .ok_or_else(|| UpdateError::InstallFailed {
                detail: "the download is no longer available".into(),
            })?;
        // Verified again by the plugin before it installs.
        update
            .restart_after_install(relaunch)
            .install(&bytes)
            .map_err(|e| classify(&e, Step::Install))
    }

    fn discard(&self) {
        *self.bytes.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }
}
