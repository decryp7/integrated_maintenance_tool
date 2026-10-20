//! IMT's local AI layer. Inference runs in a separate worker process (`imt-ai-worker`, llama.cpp) so a native
//! crash cannot take IMT down; nothing about the logs leaves the machine (web pages reach the model only when the user hands them over
//! from the internal browser, as untrusted data). The LLM only proposes: every proposal is validated and previewed by `imt-core`
//! before the user can apply it.

pub mod access;
pub mod assistant;
pub mod browser;
pub mod catalog;
pub mod chat;
#[cfg(windows)]
mod dxgi;
pub mod error;
pub mod gguf;
pub mod hardware;
pub mod llm;
pub mod manual;
pub mod models;
pub mod net;
pub mod recommend;
pub mod runtime;
pub mod runtimes;
pub mod settings;
pub mod setup;
pub mod testing;
pub mod web;
pub mod worker;

pub use access::{StoreAccess, StoreRef};
pub use error::{AiError, Result};
