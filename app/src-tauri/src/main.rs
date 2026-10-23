// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// The engine allocates per event while importing and analysing; mimalloc is faster than the Windows heap
/// there (ADR 0014 performance notes).
#[global_allocator]
static GLOBAL: debrief_alloc::MiMalloc = debrief_alloc::MiMalloc;

fn main() {
    debrief_alloc::tune();
    debrief_app_lib::run();
}
