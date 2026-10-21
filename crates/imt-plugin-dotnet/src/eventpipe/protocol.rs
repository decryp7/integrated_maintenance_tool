//! The .NET diagnostics IPC protocol (what `dotnet-trace` speaks): requests
//! and replies as pure functions over bytes.
//!
//! Every message starts with a 20-byte header: magic `DOTNET_IPC_V1\0`, the
//! total size (`u16`, header included), command set, command id and a
//! reserved `u16`. Strings are a `u32` character count (including the NUL)
//! followed by UTF-16LE; an empty string has count 0.
//! Reference: dotnet/diagnostics `documentation/design-docs/ipc-protocol.md`.

use crate::clr;
use crate::clr::Preset;

pub const MAGIC: &[u8; 14] = b"DOTNET_IPC_V1\0";
pub const HEADER_LEN: usize = 20;
/// Largest reply accepted (the header's size field is a `u16`).
pub const MAX_REPLY: usize = u16::MAX as usize;

/// Command sets and ids.
pub mod commands {
    pub const EVENTPIPE: u8 = 0x02;
    pub const STOP_TRACING: u8 = 0x01;
    pub const COLLECT_TRACING_2: u8 = 0x03;
    pub const PROCESS: u8 = 0x04;
    pub const PROCESS_INFO: u8 = 0x00;
    pub const PROCESS_INFO_2: u8 = 0x04;
    pub const SERVER: u8 = 0xff;
    pub const OK: u8 = 0x00;
    pub const ERROR: u8 = 0xff;
}

/// NetTrace output format in `CollectTracing2`.
const FORMAT_NETTRACE: u32 = 1;
/// The runtime's buffer for the session (MB).
pub const BUFFER_MB: u32 = 256;

/// A problem with a reply.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProtocolError {
    #[error("the reply does not start with the diagnostics IPC header")]
    BadMagic,
    #[error("the reply is too short ({0} bytes)")]
    Short(usize),
    #[error("the runtime refused the request (HRESULT 0x{0:08x})")]
    Refused(u32),
    #[error("unexpected reply (command set 0x{0:02x}, id 0x{1:02x})")]
    Unexpected(u8, u8),
}

/// One provider of a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provider {
    pub name: String,
    pub keywords: u64,
    pub level: u32,
}

fn message(set: u8, id: u8, payload: &[u8]) -> Vec<u8> {
    let mut m = Vec::with_capacity(HEADER_LEN + payload.len());
    m.extend_from_slice(MAGIC);
    m.extend_from_slice(&((HEADER_LEN + payload.len()).min(u16::MAX as usize) as u16).to_le_bytes());
    m.push(set);
    m.push(id);
    m.extend_from_slice(&0u16.to_le_bytes());
    m.extend_from_slice(payload);
    m
}

fn string(out: &mut Vec<u8>, s: &str) {
    if s.is_empty() {
        out.extend_from_slice(&0u32.to_le_bytes());
        return;
    }
    let units: Vec<u16> = s.encode_utf16().chain([0]).collect();
    out.extend_from_slice(&(units.len() as u32).to_le_bytes());
    for u in units {
        out.extend_from_slice(&u.to_le_bytes());
    }
}

/// The providers of a preset (the sample profiler only with CPU samples).
pub fn providers(preset: Preset) -> Vec<Provider> {
    let (keywords, level) = preset.runtime();
    let mut v = vec![Provider {
        name: clr::RUNTIME.into(),
        keywords: keywords | clr::keywords::STACK,
        level: u32::from(level),
    }];
    if preset.samples() {
        v.push(Provider {
            name: clr::SAMPLE_PROFILER.into(),
            keywords: 0,
            level: u32::from(clr::level::INFORMATIONAL),
        });
    }
    v
}

/// `CollectTracing2`: start a session streaming NetTrace on this connection.
pub fn collect_tracing(providers: &[Provider], rundown: bool) -> Vec<u8> {
    let mut p = Vec::new();
    p.extend_from_slice(&BUFFER_MB.to_le_bytes());
    p.extend_from_slice(&FORMAT_NETTRACE.to_le_bytes());
    p.push(u8::from(rundown));
    p.extend_from_slice(&(providers.len() as u32).to_le_bytes());
    for pr in providers {
        p.extend_from_slice(&pr.keywords.to_le_bytes());
        p.extend_from_slice(&pr.level.to_le_bytes());
        string(&mut p, &pr.name);
        string(&mut p, "");
    }
    message(commands::EVENTPIPE, commands::COLLECT_TRACING_2, &p)
}

/// `StopTracing` of `session`.
pub fn stop_tracing(session: u64) -> Vec<u8> {
    message(commands::EVENTPIPE, commands::STOP_TRACING, &session.to_le_bytes())
}

/// `ProcessInfo2` (or `ProcessInfo` with `v2 = false`).
pub fn process_info(v2: bool) -> Vec<u8> {
    let id = if v2 { commands::PROCESS_INFO_2 } else { commands::PROCESS_INFO };
    message(commands::PROCESS, id, &[])
}

/// A reply header: (total size, command set, command id).
pub fn header(h: &[u8]) -> Result<(usize, u8, u8), ProtocolError> {
    if h.len() < HEADER_LEN {
        return Err(ProtocolError::Short(h.len()));
    }
    if &h[..14] != MAGIC {
        return Err(ProtocolError::BadMagic);
    }
    let size = usize::from(u16::from_le_bytes([h[14], h[15]]));
    if size < HEADER_LEN {
        return Err(ProtocolError::Short(size));
    }
    Ok((size, h[16], h[17]))
}

/// The payload of an OK reply, or the error the runtime sent.
pub fn ok_payload(set: u8, id: u8, payload: &[u8]) -> Result<&[u8], ProtocolError> {
    match (set, id) {
        (commands::SERVER, commands::OK) => Ok(payload),
        (commands::SERVER, commands::ERROR) => {
            let code = payload.get(..4).map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
            Err(ProtocolError::Refused(code))
        }
        (s, i) => Err(ProtocolError::Unexpected(s, i)),
    }
}

/// The session id of a `CollectTracing` / `StopTracing` OK reply.
pub fn session_id(payload: &[u8]) -> Result<u64, ProtocolError> {
    let b = payload.get(..8).ok_or(ProtocolError::Short(payload.len()))?;
    let mut a = [0u8; 8];
    a.copy_from_slice(b);
    Ok(u64::from_le_bytes(a))
}

/// `ProcessInfo` / `ProcessInfo2`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IpcProcessInfo {
    pub pid: u64,
    pub command_line: String,
    pub os: String,
    pub arch: String,
    pub entry_assembly: Option<String>,
    pub runtime_version: Option<String>,
}

struct Rd<'a>(&'a [u8]);

impl Rd<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], ProtocolError> {
        if n > self.0.len() {
            return Err(ProtocolError::Short(self.0.len()));
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }

    fn string(&mut self) -> Result<String, ProtocolError> {
        let b = self.take(4)?;
        let n = u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize;
        if n > MAX_REPLY {
            return Err(ProtocolError::Short(n));
        }
        let raw = self.take(n * 2)?;
        let units: Vec<u16> = raw
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .take_while(|&u| u != 0)
            .collect();
        Ok(String::from_utf16_lossy(&units))
    }
}

pub fn parse_process_info(payload: &[u8], v2: bool) -> Result<IpcProcessInfo, ProtocolError> {
    let mut r = Rd(payload);
    let pid = session_id(r.take(8)?)?;
    r.take(16)?; // runtime cookie
    let mut info = IpcProcessInfo {
        pid,
        command_line: r.string()?,
        os: r.string()?,
        arch: r.string()?,
        ..IpcProcessInfo::default()
    };
    if v2 {
        info.entry_assembly = Some(r.string()?).filter(|s| !s.is_empty());
        info.runtime_version = Some(r.string()?).filter(|s| !s.is_empty());
    }
    Ok(info)
}

/// Encoders of replies (tests and the fake server).
#[doc(hidden)]
pub mod reply {
    use super::*;

    pub fn ok(payload: &[u8]) -> Vec<u8> {
        message(commands::SERVER, commands::OK, payload)
    }

    pub fn error(hresult: u32) -> Vec<u8> {
        message(commands::SERVER, commands::ERROR, &hresult.to_le_bytes())
    }

    pub fn process_info(pid: u64, command_line: &str, arch: &str, version: &str) -> Vec<u8> {
        let mut p = pid.to_le_bytes().to_vec();
        p.extend_from_slice(&[7u8; 16]);
        string(&mut p, command_line);
        string(&mut p, "Windows");
        string(&mut p, arch);
        string(&mut p, "App");
        string(&mut p, version);
        ok(&p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_requests_like_the_diagnostics_client() {
        let m = stop_tracing(0x1122);
        assert_eq!(&m[..14], MAGIC);
        assert_eq!(u16::from_le_bytes([m[14], m[15]]) as usize, m.len());
        assert_eq!((m[16], m[17]), (0x02, 0x01));
        assert_eq!(&m[20..], &0x1122u64.to_le_bytes());
        let c = collect_tracing(&providers(Preset::Cpu), true);
        assert_eq!((c[16], c[17]), (0x02, 0x03));
        assert_eq!(u32::from_le_bytes(c[20..24].try_into().unwrap()), BUFFER_MB);
        assert_eq!(u32::from_le_bytes(c[24..28].try_into().unwrap()), 1, "NetTrace");
        assert_eq!(c[28], 1, "rundown");
        assert_eq!(u32::from_le_bytes(c[29..33].try_into().unwrap()), 2, "runtime + sample profiler");
        // keywords, level, then the provider name as a counted UTF-16 string.
        let n = u32::from_le_bytes(c[45..49].try_into().unwrap()) as usize;
        assert_eq!(n, clr::RUNTIME.len() + 1);
        assert_eq!(providers(Preset::HeapSnapshot).len(), 1, "no sampler for a heap snapshot");
        assert_eq!(process_info(true)[17], 0x04);
        assert_eq!(process_info(false)[17], 0x00);
    }

    #[test]
    fn parses_replies_and_errors() {
        let r = reply::ok(&9u64.to_le_bytes());
        let (size, set, id) = header(&r).unwrap();
        assert_eq!(size, r.len());
        assert_eq!(session_id(ok_payload(set, id, &r[20..]).unwrap()).unwrap(), 9);
        let e = reply::error(0x8013_1384);
        let (_, set, id) = header(&e).unwrap();
        assert_eq!(ok_payload(set, id, &e[20..]), Err(ProtocolError::Refused(0x8013_1384)));
        assert_eq!(ok_payload(0x02, 0x01, &[]), Err(ProtocolError::Unexpected(2, 1)));
        assert_eq!(header(b"short"), Err(ProtocolError::Short(5)));
        assert_eq!(header(&[0u8; 20]), Err(ProtocolError::BadMagic));
        let mut tiny = reply::ok(&[]);
        tiny[14] = 3;
        tiny[15] = 0;
        assert_eq!(header(&tiny), Err(ProtocolError::Short(3)));
        assert!(session_id(&[1, 2]).is_err());
        assert!(ProtocolError::Refused(1).to_string().contains("refused"));

        let p = reply::process_info(42, "app.exe --run", "x64", "10.0.1");
        let info = parse_process_info(&p[20..], true).unwrap();
        assert_eq!(info.pid, 42);
        assert_eq!(info.command_line, "app.exe --run");
        assert_eq!(info.arch, "x64");
        assert_eq!(info.runtime_version.as_deref(), Some("10.0.1"));
        assert_eq!(info.entry_assembly.as_deref(), Some("App"));
        let v1 = parse_process_info(&p[20..], false).unwrap();
        assert_eq!(v1.runtime_version, None);
        for n in 0..p.len() - 20 {
            assert!(parse_process_info(&p[20..20 + n], true).is_err() || n >= p.len() - 20);
        }
        let mut huge = 0u64.to_le_bytes().to_vec();
        huge.extend_from_slice(&[0; 16]);
        huge.extend_from_slice(&u32::MAX.to_le_bytes());
        assert!(parse_process_info(&huge, false).is_err());
    }
}
