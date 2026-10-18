//! The attach configuration (`ICLRProfiling::AttachProfiler` client data) and
//! the messages on the agent's two named pipes.
//!
//! IMT creates both pipes; the agent connects as a client:
//! - `\\.\pipe\<base>-ctl`: IMT → agent [`Command`]s, framed `[len u32 LE][payload]`.
//! - `\\.\pipe\<base>-data`: agent → IMT [`DataFrame`]s: profile records, heap
//!   records and [`Event`]s, framed `[channel u8][len u32 LE][payload]`.

use crate::wire::{Dec, Enc};
use crate::{FormatError, MAX_FRAME};

const CONFIG_MAGIC: &[u8; 4] = b"IMTA";
const CONFIG_VERSION: u8 = 1;

/// Sampling interval bounds.
pub const MIN_INTERVAL_US: u32 = 1_000;
pub const MAX_INTERVAL_US: u32 = 1_000_000;

/// What the agent records besides heap snapshots on request.
pub mod features {
    /// Periodic stack samples of every managed thread (CPU and wait time).
    pub const SAMPLING: u32 = 0x1;
    /// Thrown exceptions with their stacks.
    pub const EXCEPTIONS: u32 = 0x2;
    /// GC pauses (runtime suspensions for GC) on the timeline.
    pub const GC_PAUSES: u32 = 0x4;
    pub const ALL: u32 = SAMPLING | EXCEPTIONS | GC_PAUSES;
}

/// What IMT tells the agent when it attaches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachConfig {
    /// Pipe base name: `imt-clr-` and 32 lowercase hex digits.
    pub pipe: String,
    pub interval_us: u32,
    /// [`features`] bits.
    pub features: u32,
    /// Bytes the agent may buffer while IMT reads slower than it records.
    pub max_queue_bytes: u64,
    /// Objects in one heap snapshot; more are dropped and the snapshot is marked truncated.
    pub max_objects: u64,
}

/// Whether `s` is a pipe base name IMT generates.
pub fn valid_pipe_base(s: &str) -> bool {
    s.strip_prefix("imt-clr-")
        .is_some_and(|hex| hex.len() == 32 && hex.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
}

impl AttachConfig {
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Enc::new();
        e.raw(CONFIG_MAGIC)
            .u8(CONFIG_VERSION)
            .str(&self.pipe)
            .u(u64::from(self.interval_us))
            .u(u64::from(self.features))
            .u(self.max_queue_bytes)
            .u(self.max_objects);
        e.buf
    }

    /// Parses and validates client data; the agent refuses to attach on error.
    pub fn decode(bytes: &[u8]) -> Result<AttachConfig, FormatError> {
        let d = &mut Dec::new(bytes);
        if d.take(4).map_err(|_| FormatError::NotThisFormat)? != CONFIG_MAGIC {
            return Err(FormatError::NotThisFormat);
        }
        let version = d.u8()?;
        if version != CONFIG_VERSION {
            return Err(FormatError::UnsupportedVersion(u16::from(version)));
        }
        let c = AttachConfig {
            pipe: d.str()?,
            interval_us: d.u32()?,
            features: d.u32()?,
            max_queue_bytes: d.u()?,
            max_objects: d.u()?,
        };
        if !valid_pipe_base(&c.pipe) {
            return Err(FormatError::Invalid("pipe name is not an IMT pipe"));
        }
        if !(MIN_INTERVAL_US..=MAX_INTERVAL_US).contains(&c.interval_us) {
            return Err(FormatError::Invalid("sampling interval is out of range"));
        }
        if c.features & !features::ALL != 0 {
            return Err(FormatError::Invalid("unknown feature bits"));
        }
        if !(1 << 20..=1 << 34).contains(&c.max_queue_bytes) || c.max_objects == 0 {
            return Err(FormatError::Invalid("buffer or object limit is out of range"));
        }
        Ok(c)
    }
}

/// IMT → agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Take a heap snapshot (induced full GC + heap walk).
    Snapshot,
    /// Stop recording, flush and detach the profiler.
    Stop,
    Ping,
}

impl Command {
    pub fn encode(self) -> Vec<u8> {
        let code = match self {
            Command::Snapshot => 1u8,
            Command::Stop => 2,
            Command::Ping => 3,
        };
        let mut v = 1u32.to_le_bytes().to_vec();
        v.push(code);
        v
    }

    pub fn decode(payload: &[u8]) -> Result<Command, FormatError> {
        match payload {
            [1] => Ok(Command::Snapshot),
            [2] => Ok(Command::Stop),
            [3] => Ok(Command::Ping),
            _ => Err(FormatError::Invalid("unknown command")),
        }
    }
}

/// Agent → IMT notifications.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The agent is attached and recording.
    Hello {
        pid: u32,
        process: String,
        runtime: String,
        pointer_bits: u8,
        agent_version: String,
    },
    SnapshotStarted,
    SnapshotDone {
        objects: u64,
        truncated: bool,
    },
    SnapshotFailed {
        message: String,
    },
    /// Recording ended; the agent detaches next.
    Stopped,
    /// Something went wrong in the agent; recording may continue.
    Error {
        message: String,
    },
    Pong,
}

impl Event {
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Enc::new();
        match self {
            Event::Hello {
                pid,
                process,
                runtime,
                pointer_bits,
                agent_version,
            } => {
                e.u8(1)
                    .u(u64::from(*pid))
                    .str(process)
                    .str(runtime)
                    .u8(*pointer_bits)
                    .str(agent_version);
            }
            Event::SnapshotStarted => {
                e.u8(2);
            }
            Event::SnapshotDone { objects, truncated } => {
                e.u8(3).u(*objects).bool(*truncated);
            }
            Event::SnapshotFailed { message } => {
                e.u8(4).str(message);
            }
            Event::Stopped => {
                e.u8(5);
            }
            Event::Error { message } => {
                e.u8(6).str(message);
            }
            Event::Pong => {
                e.u8(7);
            }
        }
        e.buf
    }

    pub fn decode(payload: &[u8]) -> Result<Event, FormatError> {
        let d = &mut Dec::new(payload);
        Ok(match d.u8()? {
            1 => Event::Hello {
                pid: d.u32()?,
                process: d.str()?,
                runtime: d.str()?,
                pointer_bits: d.u8()?,
                agent_version: d.str()?,
            },
            2 => Event::SnapshotStarted,
            3 => Event::SnapshotDone {
                objects: d.u()?,
                truncated: d.bool()?,
            },
            4 => Event::SnapshotFailed { message: d.str()? },
            5 => Event::Stopped,
            6 => Event::Error { message: d.str()? },
            7 => Event::Pong,
            _ => return Err(FormatError::Invalid("unknown event")),
        })
    }
}

/// Channels on the data pipe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    /// A framed [`crate::ProfileRecord`].
    Profile = 0,
    /// A framed [`crate::HeapRecord`] of the snapshot in progress.
    Heap = 1,
    /// An [`Event`].
    Event = 2,
}

/// Frames a payload for the data pipe.
pub fn data_frame(channel: Channel, payload: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(payload.len() + 5);
    v.push(channel as u8);
    v.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    v.extend_from_slice(payload);
    v
}

/// Reads the header of a data-pipe frame: the channel and payload length.
pub fn data_frame_header(h: &[u8; 5]) -> Result<(Channel, usize), FormatError> {
    let channel = match h[0] {
        0 => Channel::Profile,
        1 => Channel::Heap,
        2 => Channel::Event,
        _ => return Err(FormatError::Invalid("unknown data channel")),
    };
    let len = u32::from_le_bytes([h[1], h[2], h[3], h[4]]) as usize;
    if len > MAX_FRAME {
        return Err(FormatError::Invalid("data frame is larger than the protocol allows"));
    }
    Ok((channel, len))
}

/// Reads the length prefix of a command frame.
pub fn command_len(h: &[u8; 4]) -> Result<usize, FormatError> {
    let len = u32::from_le_bytes(*h) as usize;
    if len == 0 || len > 64 {
        return Err(FormatError::Invalid("command frame has a bad length"));
    }
    Ok(len)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> AttachConfig {
        AttachConfig {
            pipe: format!("imt-clr-{}", "0123456789abcdef".repeat(2)),
            interval_us: 10_000,
            features: features::ALL,
            max_queue_bytes: 64 << 20,
            max_objects: 20_000_000,
        }
    }

    #[test]
    fn attach_config_round_trips_and_is_validated() {
        let c = config();
        assert_eq!(AttachConfig::decode(&c.encode()).unwrap(), c);
        assert_eq!(AttachConfig::decode(b"xx").unwrap_err(), FormatError::NotThisFormat);
        let mut v = c.encode();
        v[4] = 9;
        assert_eq!(AttachConfig::decode(&v).unwrap_err(), FormatError::UnsupportedVersion(9));
        for bad in [
            AttachConfig {
                pipe: "evil".into(),
                ..config()
            },
            AttachConfig {
                pipe: format!("imt-clr-{}", "G".repeat(32)),
                ..config()
            },
            AttachConfig {
                interval_us: 10,
                ..config()
            },
            AttachConfig {
                features: 0x80,
                ..config()
            },
            AttachConfig {
                max_queue_bytes: 1,
                ..config()
            },
            AttachConfig {
                max_objects: 0,
                ..config()
            },
        ] {
            assert!(
                matches!(AttachConfig::decode(&bad.encode()), Err(FormatError::Invalid(_))),
                "{bad:?}"
            );
        }
        assert!(AttachConfig::decode(&c.encode()[..10]).is_err());
    }

    #[test]
    fn commands_and_events_round_trip() {
        for c in [Command::Snapshot, Command::Stop, Command::Ping] {
            let v = c.encode();
            let len = command_len(&[v[0], v[1], v[2], v[3]]).unwrap();
            assert_eq!(Command::decode(&v[4..4 + len]).unwrap(), c);
        }
        assert!(Command::decode(&[9]).is_err());
        assert!(command_len(&[0, 0, 0, 0]).is_err());
        assert!(command_len(&[0, 1, 0, 0]).is_err());
        for e in [
            Event::Hello {
                pid: 7,
                process: "a.exe".into(),
                runtime: "4.8".into(),
                pointer_bits: 64,
                agent_version: "0.1.0".into(),
            },
            Event::SnapshotStarted,
            Event::SnapshotDone {
                objects: 12,
                truncated: true,
            },
            Event::SnapshotFailed { message: "x".into() },
            Event::Stopped,
            Event::Error { message: "y".into() },
            Event::Pong,
        ] {
            assert_eq!(Event::decode(&e.encode()).unwrap(), e);
        }
        assert!(Event::decode(&[42]).is_err());
        assert!(Event::decode(&[]).is_err());
    }

    #[test]
    fn data_frames_carry_their_channel() {
        let f = data_frame(Channel::Heap, b"abc");
        let h: [u8; 5] = f[..5].try_into().unwrap();
        assert_eq!(data_frame_header(&h).unwrap(), (Channel::Heap, 3));
        assert_eq!(&f[5..], b"abc");
        assert!(data_frame_header(&[9, 0, 0, 0, 0]).is_err());
        assert!(data_frame_header(&[0, 0xff, 0xff, 0xff, 0xff]).is_err());
        assert_eq!(data_frame_header(&[0, 0, 0, 0, 0]).unwrap().0, Channel::Profile);
        assert_eq!(data_frame_header(&[2, 0, 0, 0, 0]).unwrap().0, Channel::Event);
    }
}
