//! Why a package was refused. Every error names what to fix; [`PackageError::code`] is the stable, machine-readable
//! reason the app passes to the UI.

/// Why a `.debriefplugin` package could not be read, verified, built or installed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PackageError {
    /// The file could not be read or written.
    #[error("{0}")]
    Io(String),
    /// The file is not a zip archive, or the archive is damaged.
    #[error("this is not a Debrief plugin package: {0}")]
    NotAPackage(String),
    /// Something in the package is larger than Debrief accepts.
    #[error("{what} is larger than the limit of {limit} bytes")]
    TooLarge {
        /// What exceeded its limit (`the package`, `bin/.../x.exe`, ...).
        what: String,
        /// The limit in bytes.
        limit: u64,
    },
    /// The manifest is missing a value or has an invalid one.
    #[error("manifest.toml: {0}")]
    InvalidManifest(String),
    /// The archive holds a file the manifest does not list (or one that may not be in a package).
    #[error("the package contains \"{0}\", which its manifest does not list")]
    UnlistedFile(String),
    /// A file the manifest lists is missing.
    #[error("the package lacks \"{0}\"")]
    MissingFile(String),
    /// An entry's name, kind or encryption is unsafe (path traversal, a link, an encrypted or duplicate entry).
    #[error("the package has an unsafe entry \"{0}\"")]
    UnsafeEntry(String),
    /// The signature does not match the contents, or the package was changed after it was signed.
    #[error("the package was changed after it was signed or its signature is damaged ({0}); do not install it")]
    Tampered(String),
    /// The package is unsigned and developer mode is off.
    #[error("the package is not signed; turn on developer mode in Settings to install unsigned plugins")]
    Unsigned,
    /// The package is signed with a key that is not trusted and developer mode is off.
    #[error(
        "the package is signed with key {key_id}, which is not a trusted publisher key; add the key in Settings, or turn on developer mode"
    )]
    UnknownKey {
        /// The id of the signing key.
        key_id: String,
    },
    /// The plugin was built for a plugin protocol this Debrief cannot serve.
    #[error("{0}")]
    Incompatible(String),
    /// The package has no binary for this computer.
    #[error("the package has no binary for this computer ({target}); it supports {supported}")]
    WrongTarget {
        /// This computer's target triple.
        target: String,
        /// The targets the package has binaries for.
        supported: String,
    },
    /// A key file or key text is not valid.
    #[error("{0}")]
    InvalidKey(String),
}

impl PackageError {
    /// The stable reason code (`tampered`, `unsigned`, `unknownKey`, ...).
    pub fn code(&self) -> &'static str {
        match self {
            PackageError::Io(_) => "io",
            PackageError::NotAPackage(_) => "notAPackage",
            PackageError::TooLarge { .. } => "tooLarge",
            PackageError::InvalidManifest(_) => "invalidManifest",
            PackageError::UnlistedFile(_) => "unlistedFile",
            PackageError::MissingFile(_) => "missingFile",
            PackageError::UnsafeEntry(_) => "unsafeEntry",
            PackageError::Tampered(_) => "tampered",
            PackageError::Unsigned => "unsigned",
            PackageError::UnknownKey { .. } => "unknownKey",
            PackageError::Incompatible(_) => "incompatible",
            PackageError::WrongTarget { .. } => "wrongTarget",
            PackageError::InvalidKey(_) => "invalidKey",
        }
    }
}

impl From<std::io::Error> for PackageError {
    fn from(e: std::io::Error) -> Self {
        PackageError::Io(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_error_has_a_code_and_a_message() {
        let all = [
            PackageError::Io("x".into()),
            PackageError::NotAPackage("x".into()),
            PackageError::TooLarge {
                what: "x".into(),
                limit: 1,
            },
            PackageError::InvalidManifest("x".into()),
            PackageError::UnlistedFile("x".into()),
            PackageError::MissingFile("x".into()),
            PackageError::UnsafeEntry("x".into()),
            PackageError::Tampered("x".into()),
            PackageError::Unsigned,
            PackageError::UnknownKey { key_id: "k".into() },
            PackageError::Incompatible("x".into()),
            PackageError::WrongTarget {
                target: "t".into(),
                supported: "u".into(),
            },
            PackageError::InvalidKey("x".into()),
        ];
        let codes: std::collections::HashSet<_> = all.iter().map(PackageError::code).collect();
        assert_eq!(codes.len(), all.len());
        assert!(all.iter().all(|e| !e.to_string().is_empty()));
        assert_eq!(PackageError::from(std::io::Error::other("disk")).code(), "io");
    }
}
