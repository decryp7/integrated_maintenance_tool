//! The LLM port, its test fake, tool calling for local models and lazy loading.

pub mod fake;
pub mod lazy;
pub mod prompt_format;
pub mod prompted;
pub mod provider;
pub mod tool_protocol;
pub mod types;
pub mod utf8;

pub use fake::{Reply, ScriptedProvider};
pub use lazy::{LazyProvider, LoadState, ModelLoader};
pub use prompt_format::PromptFormat;
pub use prompted::{Finish, GenParams, Generated, PromptedProvider, TextEngine};
pub use provider::LlmProvider;
pub use types::*;
