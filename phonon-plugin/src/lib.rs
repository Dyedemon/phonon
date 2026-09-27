//! Phonon Plugin - WebAssembly plugin system.
//!
//! Provides WASM plugin runtime using wasmtime, supporting
//! Source Provider, DSP Processor, and Decoder plugin types.

pub mod builtin;
pub mod host;
#[cfg(feature = "runtime")]
pub mod runtime;
pub mod types;

pub use builtin::{all as list_builtins, BuiltinPlugin};
#[cfg(feature = "runtime")]
pub use runtime::{merge_plugin_layers, PluginRuntime};
pub use types::*;
