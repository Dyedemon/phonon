//! Phonon Source - Source provider abstraction layer.
//!
//! Defines the IAudioSource trait and provides implementations
//! for local file sources, network streams, and playlist management.

pub mod local;
pub mod network;
pub mod playlist;
pub mod traits;

pub use local::LocalFileSource;
pub use network::NetworkStreamSource;
pub use playlist::{
    parse_m3u, parse_pls, parse_xspf, Playlist, PlaylistEntry, PlaylistError, PlaylistFormat,
};
pub use traits::*;
