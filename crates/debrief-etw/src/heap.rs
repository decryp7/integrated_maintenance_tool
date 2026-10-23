//! The Windows heap provider's classic events (`Heap` class, the heap tracing that WPR/xperf `-heap` and PerfView
//! `/OSHeapProcess` enable for one process), as laid out in TraceEvent's `HeapTraceProviderTraceEventParser`.
//! Pointer-sized fields follow the event header's 32/64-bit flag. Decoders return `None` for payloads too short
//! for their fields; none panics.

use crate::kernel::Reader;
use debrief_plugin_profiling::nettrace::ext::HeapOp;

/// {222962AB-6180-4B88-A825-346B75F2A24A}: the heap trace provider (and the event class of its events).
pub const PROVIDER: u128 = 0x222962ab_6180_4b88_a825_346b75f2a24a;

/// Opcodes of the heap events Debrief reads.
pub mod opcodes {
    pub const ALLOC: u8 = 33;
    pub const REALLOC: u8 = 34;
    pub const DESTROY: u8 = 35;
    pub const FREE: u8 = 36;
}

/// Decodes a heap event (`None`: another opcode, or a payload too short).
pub fn decode(opcode: u8, p: &[u8], ptr: usize) -> Option<HeapOp> {
    let mut r = Reader::new(p);
    match opcode {
        opcodes::ALLOC => {
            let heap = r.ptr(ptr)?;
            let size = r.ptr(ptr)?;
            let address = r.ptr(ptr)?;
            Some(HeapOp::Alloc { heap, address, size })
        }
        opcodes::REALLOC => {
            let heap = r.ptr(ptr)?;
            let new_address = r.ptr(ptr)?;
            let old_address = r.ptr(ptr)?;
            let size = r.ptr(ptr)?;
            Some(HeapOp::ReAlloc {
                heap,
                new_address,
                old_address,
                size,
            })
        }
        opcodes::FREE => {
            let heap = r.ptr(ptr)?;
            let address = r.ptr(ptr)?;
            Some(HeapOp::Free { heap, address })
        }
        _ => None,
    }
}

/// Payload encoders (tests, fuzz seeds): the layouts [`decode`] reads, with a source id.
#[doc(hidden)]
pub mod encode {
    use super::*;

    fn ptr(p: &mut Vec<u8>, v: u64, size: usize) {
        p.extend_from_slice(&v.to_le_bytes()[..size]);
    }

    /// The opcode and payload of `op`.
    pub fn op(op: &HeapOp, size: usize) -> (u8, Vec<u8>) {
        let mut p = Vec::new();
        let opcode = match *op {
            HeapOp::Alloc { heap, address, size: n } => {
                for v in [heap, n, address] {
                    ptr(&mut p, v, size);
                }
                opcodes::ALLOC
            }
            HeapOp::ReAlloc {
                heap,
                new_address,
                old_address,
                size: n,
            } => {
                for v in [heap, new_address, old_address, n, 0] {
                    ptr(&mut p, v, size);
                }
                opcodes::REALLOC
            }
            HeapOp::Free { heap, address } => {
                for v in [heap, address] {
                    ptr(&mut p, v, size);
                }
                opcodes::FREE
            }
        };
        p.extend_from_slice(&1u32.to_le_bytes());
        (opcode, p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_every_heap_event_for_both_pointer_sizes_and_never_panics() {
        let ops = [
            HeapOp::Alloc {
                heap: 0x10,
                address: 0x2000,
                size: 64,
            },
            HeapOp::ReAlloc {
                heap: 0x10,
                new_address: 0x3000,
                old_address: 0x2000,
                size: 128,
            },
            HeapOp::Free {
                heap: 0x10,
                address: 0x3000,
            },
        ];
        for s in [4usize, 8] {
            for op in ops {
                let (opcode, p) = encode::op(&op, s);
                assert_eq!(decode(opcode, &p, s), Some(op));
                for n in 0..p.len().saturating_sub(4) {
                    if n < p.len() - 4 {
                        let _ = decode(opcode, &p[..n], s);
                    }
                }
                assert!(decode(opcode, &p[..s], s).is_none());
            }
        }
        assert!(decode(opcodes::DESTROY, &[0; 16], 8).is_none());
    }
}
