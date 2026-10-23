//! Entities in event text: the things a message or exception mentions that an analyst searches for by name
//! (executables and libraries, file names and paths, hosts and addresses, URLs, exception types, error codes,
//! GUIDs). Pure and bounded: [`extract`] reads at most [`MAX_SCAN_CHARS`] characters of each text, yields at most
//! [`MAX_PER_EVENT`] entities per event, clips names to [`MAX_NAME_CHARS`] and never panics on any input.
//!
//! Recognised by shape (no dictionaries of products): a token is an IP address (`std::net` parsing), a GUID, an
//! HRESULT-like `0x8...` code, a known errno or Windows error name, an exception type (`...Exception`,
//! `...Error`), an HTTP status after `status`/`HTTP`/`code`, a path (Windows drive or UNC, POSIX), a file name
//! with a known extension (`.exe` is an executable, `.dll`/`.so`/`.dylib`/`.sys` a library), a host (dotted labels
//! ending in a common top-level domain), or a URL (scheme, host and path; user info, query and fragment are dropped
//! because they hold secrets). Paths with spaces are found when quoted. The names are what the logs wrote.

mod classify;
mod scan;

pub use scan::extract;

use serde::{Deserialize, Serialize};

/// Most characters of a text that are read.
pub const MAX_SCAN_CHARS: usize = 4096;
/// Most entities one event yields.
pub const MAX_PER_EVENT: usize = 64;
/// Longest entity name (characters); longer ones are not entities.
pub const MAX_NAME_CHARS: usize = 256;

/// What an entity is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EntityKind {
    /// A program (`.exe`, or a `name[pid]` process tag).
    Executable,
    /// A library or driver (`.dll`, `.so`, `.dylib`, `.sys`).
    Library,
    /// A file name with a known extension.
    File,
    /// A file path.
    Path,
    Host,
    Ip,
    Url,
    /// An exception or error type.
    Exception,
    /// An HRESULT, errno name, Windows error name or HTTP status.
    ErrorCode,
    Guid,
}

impl EntityKind {
    /// The stable id stored and shown.
    pub fn id(self) -> &'static str {
        match self {
            EntityKind::Executable => "executable",
            EntityKind::Library => "library",
            EntityKind::File => "file",
            EntityKind::Path => "path",
            EntityKind::Host => "host",
            EntityKind::Ip => "ip",
            EntityKind::Url => "url",
            EntityKind::Exception => "exception",
            EntityKind::ErrorCode => "errorCode",
            EntityKind::Guid => "guid",
        }
    }

    /// The kind with `id`.
    pub fn from_id(id: &str) -> Option<EntityKind> {
        use EntityKind::*;
        [Executable, Library, File, Path, Host, Ip, Url, Exception, ErrorCode, Guid]
            .into_iter()
            .find(|k| k.id() == id)
    }
}

/// One thing mentioned.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Entity {
    pub kind: EntityKind,
    pub name: String,
}

/// The entities of one event, each once, in order of appearance.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Extracted {
    pub entities: Vec<Entity>,
    /// GUIDs seen (all of them, each once); [`Self::entities`] also lists them, so a caller can cap how many it keeps.
    pub guids: u32,
}

#[cfg(test)]
mod tests;
