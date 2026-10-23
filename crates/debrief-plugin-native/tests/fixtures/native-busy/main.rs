//! `native-busy`: a native process to profile in Debrief's tests (ADR 0022). No C/C++ toolchain is needed: it is
//! plain Rust compiled to native code with a PDB.
//!
//! `native-busy <mode> [seconds]` with mode `busy` (a CPU-bound loop in `native_busy_spin`), `locks` (threads
//! contending for one lock in `native_busy_contend`), `alloc` (a heap that keeps growing in `native_busy_grow`) or
//! `all`. It prints `ready <pid>` once working and exits after `seconds` (default 60) or when its standard input
//! closes, so a test that dies does not leave it running.

use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[inline(never)]
fn native_busy_spin(stop: &AtomicBool) -> u64 {
    let mut acc = 0x9e37_79b9_7f4a_7c15u64;
    while !stop.load(Ordering::Relaxed) {
        for _ in 0..10_000 {
            acc = std::hint::black_box(acc.rotate_left(13) ^ acc.wrapping_mul(0x2545_f491_4f6c_dd1d));
        }
    }
    acc
}

#[inline(never)]
fn native_busy_contend(lock: &Mutex<u64>, stop: &AtomicBool) {
    while !stop.load(Ordering::Relaxed) {
        let mut guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        // Hold the lock long enough that the other threads wait for it.
        let until = Instant::now() + Duration::from_millis(5);
        while Instant::now() < until {
            *guard = std::hint::black_box(guard.wrapping_add(1));
        }
    }
}

#[inline(never)]
fn native_busy_grow(stop: &AtomicBool) -> usize {
    let mut kept: Vec<Box<[u8]>> = Vec::new();
    let mut n = 0usize;
    while !stop.load(Ordering::Relaxed) {
        // Allocate, keep every fourth block: the heap grows.
        let block = vec![n as u8; 4096].into_boxed_slice();
        if n.is_multiple_of(4) {
            kept.push(block);
        }
        n += 1;
        if kept.len() > 200_000 {
            kept.clear();
        }
        std::thread::sleep(Duration::from_micros(200));
    }
    kept.len()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(String::as_str).unwrap_or("busy");
    let seconds: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(60);
    let stop = Arc::new(AtomicBool::new(false));
    let mut workers = Vec::new();
    if matches!(mode, "busy" | "all") {
        let s = stop.clone();
        workers.push(std::thread::spawn(move || {
            native_busy_spin(&s);
        }));
    }
    if matches!(mode, "locks" | "all") {
        let lock = Arc::new(Mutex::new(0u64));
        for _ in 0..4 {
            let (s, l) = (stop.clone(), lock.clone());
            workers.push(std::thread::spawn(move || native_busy_contend(&l, &s)));
        }
    }
    if matches!(mode, "alloc" | "all") {
        let s = stop.clone();
        workers.push(std::thread::spawn(move || {
            native_busy_grow(&s);
        }));
    }
    if workers.is_empty() {
        eprintln!("usage: native-busy busy|locks|alloc|all [seconds]");
        std::process::exit(2);
    }
    println!("ready {}", std::process::id());
    let _ = std::io::stdout().flush();
    // Stop when stdin closes (the test ended) or the time is up.
    let s = stop.clone();
    std::thread::spawn(move || {
        let mut line = String::new();
        let _ = std::io::stdin().lock().read_line(&mut line);
        s.store(true, Ordering::Relaxed);
    });
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while !stop.load(Ordering::Relaxed) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    stop.store(true, Ordering::Relaxed);
    for w in workers {
        let _ = w.join();
    }
}
