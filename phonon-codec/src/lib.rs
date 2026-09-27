//! Phonon Codec - Unified audio decoding layer
//!
//! Provides format-agnostic audio decoding, metadata extraction,
//! CUE sheet parsing, and DSD support.

pub mod cue;
pub mod decoder;
pub mod dsd;
pub mod metadata;
pub mod types;

pub use decoder::AudioDecoder;
pub use metadata::{write_metadata_to_file, PartialMeta};
pub use types::*;
