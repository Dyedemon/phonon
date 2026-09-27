//! Phonon Core - Audio engine kernel.
//!
//! Provides device management, PCM output, ring buffer,
//! gapless playback, volume control, and DSP chain.

pub mod device;
pub mod dsd;
pub mod dsp;
pub mod engine;
pub mod eq;
pub mod peq;
pub mod ringbuf;
pub mod volume;
#[cfg(windows)]
pub mod wasapi;

pub use device::DeviceManager;
pub use dsd::{DeltaSigmaModulator, DsdMode};
pub use dsp::{DspChain, DspProcessorInfo, WasmDspProcessor};
pub use engine::{
    default_time_stretch_factory, time_stretch_factory, AppEvent, EngineConfig, EngineError,
    PlaybackEngine, PlaybackMode, PlaybackState, PluginTimeStretcher, QueueItem, ResamplerQuality,
    TimeStretch, TimeStretchFactory, TimeStretchMode, EVENT_TX,
};
pub use eq::EqualizerProcessor;
pub use peq::{PeqProcessor, PeqSnapshot, PeqUpdateError, PeqUpdateHandle};
pub use ringbuf::RingBuffer;
pub use volume::VolumeControl;
