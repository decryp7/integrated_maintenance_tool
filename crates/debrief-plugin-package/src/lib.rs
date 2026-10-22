//! The `.debriefplugin` package of third-party Debrief plugins (ADR 0027): its format, reading and verifying it,
//! building and signing it, publisher keys and the trust decision. Pure library: no app state, no process
//! launching (the app installs and runs plugins with `debrief-plugin-host`).
//!
//! - [`manifest`]: `manifest.toml` ([`PackageManifest`]);
//! - [`archive`]: reading a package with limits ([`Package::open`]), its canonical digest and signature, and
//!   writing one ([`archive::build`]);
//! - [`keys`]: Ed25519 publisher keys ([`SecretKey`], [`PublicKey`]);
//! - [`trust`]: which packages may be installed ([`TrustSettings::decide`]).

#![deny(missing_docs)]

pub mod archive;
mod error;
pub mod keys;
pub mod manifest;
pub mod trust;

pub use archive::{Package, Signature};
pub use error::PackageError;
pub use keys::{PublicKey, SecretKey};
pub use manifest::PackageManifest;
pub use trust::{SignatureStatus, Trust, TrustSettings};
