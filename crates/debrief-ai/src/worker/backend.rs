//! What a worker does, independent of the wire: load a model and generate text. The real worker implements it
//! with llama.cpp; the fake worker of the tests implements it with scripted behaviour.

use super::protocol::*;

/// Contract: methods never panic across the boundary (the server contains panics as errors), `chat` stops
/// early with `WireFinish::Cancelled` when `cancelled()` turns true, and nothing touches the network or any
/// file except the model path given to `load`.
pub trait WorkerBackend {
    fn load(&mut self, p: &LoadParams) -> Result<LoadResult, RpcError>;
    fn chat(&mut self, p: &ChatParams, delta: &mut dyn FnMut(&str), cancelled: &dyn Fn() -> bool) -> Result<ChatResult, RpcError>;
    fn unload(&mut self);
}

pub use debrief_ipc::process::current_rss_bytes;

#[cfg(test)]
mod tests {
    #[test]
    fn rss_is_reported() {
        assert!(super::current_rss_bytes() > 0);
    }
}
