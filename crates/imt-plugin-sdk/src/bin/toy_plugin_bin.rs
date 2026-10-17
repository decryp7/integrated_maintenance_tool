//! The example plugin (`examples/toy_plugin.rs`) as a binary, so the tests and coverage runs (which
//! build no examples) can start it.

#[path = "../../examples/toy_plugin.rs"]
mod toy;

fn main() {
    toy::main();
}
