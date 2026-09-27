//! Audio device management.
//!
//! Enumerates audio devices, handles exclusive mode, sample rate switching,
//! hot-plug detection, and device change notifications.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleRate, Stream, StreamConfig};
use phonon_codec::SampleRate as PhononSampleRate;
use rubato::FftFixedIn;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;
use tokio::sync::broadcast;

#[cfg(windows)]
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

/// Information about an audio device.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DeviceInfo {
    pub name: String,
    pub id: String,
    pub is_default: bool,
    pub supported_sample_rates: Vec<PhononSampleRate>,
    pub max_channels: u16,
    pub supports_exclusive: bool,
}

/// Event emitted when device state changes.
#[derive(Debug, Clone)]
pub enum DeviceEvent {
    DeviceAdded(DeviceInfo),
    DeviceRemoved(String),
    DeviceChanged(DeviceInfo),
    DefaultDeviceChanged(DeviceInfo),
}

/// Manages audio device enumeration and hot-plug detection.
pub struct DeviceManager {
    host: cpal::Host,
    current_device: Option<Device>,
    current_stream: Option<Stream>,
    event_tx: broadcast::Sender<DeviceEvent>,
    /// Last known device list for hotplug detection.
    last_devices: Arc<Mutex<Vec<String>>>,
    /// Hotplug monitor stop signal.
    hotplug_stop: Arc<AtomicBool>,
    /// Handle for the hotplug monitor thread (so we can join on stop).
    hotplug_thread: Option<JoinHandle<()>>,
    /// Whether to attempt exclusive mode (Windows WASAPI).
    exclusive_mode: bool,
    /// Stop signal for WASAPI exclusive thread.
    #[cfg(windows)]
    wasapi_stop: Arc<AtomicBool>,
    /// Handle for the WASAPI exclusive thread (so we can join before restarting).
    #[cfg(windows)]
    wasapi_thread: Option<JoinHandle<()>>,
    /// Live stream telemetry for the diagnostics panel.
    diagnostics: Arc<AudioDiagnostics>,
}

/// Live audio-stream telemetry, safe to read/update from any thread.
#[derive(Default)]
pub struct AudioDiagnostics {
    /// 0 = shared, 1 = exclusive (windows).
    pub mode: AtomicU8,
    /// Negotiated container: 0 none, 1 Dop24, 2 Pcm24, 3 Pcm16, 4 Float32.
    pub format: AtomicU8,
    pub buffer_frames: AtomicU32,
    /// Device-side queued frames, refreshed by the exclusive pump.
    pub padding_frames: AtomicU32,
    /// Times the output callback found the ring buffer empty mid-playback.
    pub underruns: AtomicU32,
}

impl AudioDiagnostics {
    pub fn format_label(&self) -> &'static str {
        match self.format.load(Ordering::Relaxed) {
            1 => "DoP (DSD over PCM)",
            2 => "PCM 24-bit",
            3 => "PCM 16-bit",
            4 => "Float 32-bit",
            5 => "Native DSD",
            _ => "",
        }
    }
    pub fn mode_label(&self) -> &'static str {
        match self.mode.load(Ordering::Relaxed) {
            1 => "exclusive",
            _ => "shared",
        }
    }
}

impl DeviceManager {
    /// Create a new device manager.
    pub fn new() -> Result<Self, DeviceError> {
        let host = cpal::default_host();
        let (event_tx, _) = broadcast::channel(32);

        Ok(Self {
            host,
            current_device: None,
            current_stream: None,
            event_tx,
            last_devices: Arc::new(Mutex::new(Vec::new())),
            hotplug_stop: Arc::new(AtomicBool::new(false)),
            hotplug_thread: None,
            exclusive_mode: false,
            #[cfg(windows)]
            wasapi_stop: Arc::new(AtomicBool::new(false)),
            #[cfg(windows)]
            wasapi_thread: None,
            diagnostics: Arc::new(AudioDiagnostics::default()),
        })
    }

    /// Subscribe to device events.
    pub fn subscribe(&self) -> broadcast::Receiver<DeviceEvent> {
        self.event_tx.subscribe()
    }

    /// List all available output devices.
    pub fn list_devices(&self) -> Result<Vec<DeviceInfo>, DeviceError> {
        let devices = self
            .host
            .output_devices()
            .map_err(|e| DeviceError::Enumeration(e.to_string()))?;

        let default_device = self
            .host
            .default_output_device()
            .map(|d| d.name().unwrap_or_default());

        let mut result = Vec::new();

        for device in devices {
            let name = device.name().unwrap_or_else(|_| "Unknown".into());
            let is_default = default_device.as_deref() == Some(&name);

            result.push(DeviceInfo {
                id: name.clone(),
                name,
                is_default,
                supported_sample_rates: Vec::new(),
                max_channels: 2,
                supports_exclusive: cfg!(target_os = "windows"),
            });
        }

        Ok(result)
    }

    /// Get the default output device.
    pub fn default_device(&self) -> Result<DeviceInfo, DeviceError> {
        let device = self
            .host
            .default_output_device()
            .ok_or(DeviceError::NoDevice)?;

        let name = device.name().unwrap_or_else(|_| "Unknown".into());

        Ok(DeviceInfo {
            id: name.clone(),
            name,
            is_default: true,
            supported_sample_rates: Vec::new(),
            max_channels: 2,
            supports_exclusive: cfg!(target_os = "windows"),
        })
    }

    /// Set the preferred device name for future playback.
    pub fn set_default_device(&mut self, device_name: &str) -> Result<(), DeviceError> {
        let device = self.find_device(device_name)?;
        self.current_device = Some(device);
        Ok(())
    }

    /// Get the name of the currently selected device, if any.
    pub fn current_device_name(&self) -> Option<String> {
        self.current_device.as_ref().and_then(|d| d.name().ok())
    }

    /// Live stream telemetry for the diagnostics panel.
    pub fn diagnostics(&self) -> Arc<AudioDiagnostics> {
        self.diagnostics.clone()
    }

    /// Get the name of the default output device (always available if any audio device exists).
    pub fn default_device_name(&self) -> Option<String> {
        self.host
            .default_output_device()
            .and_then(|d| d.name().ok())
    }

    /// Clear the currently selected device (called when device is removed).
    pub fn clear_current_device(&mut self) {
        self.current_device = None;
        self.current_stream = None;
    }

    /// Get full info for the currently selected device, if any.
    pub fn current_device_info(&self) -> Option<DeviceInfo> {
        self.current_device.as_ref().and_then(|d| {
            let name = d.name().ok()?;
            Some(DeviceInfo {
                id: name.clone(),
                name,
                is_default: false,
                supported_sample_rates: Vec::new(),
                max_channels: 2,
                supports_exclusive: cfg!(target_os = "windows"),
            })
        })
    }

    /// Open a device for playback with the given sample rate and channel count.
    pub fn open_device(
        &mut self,
        device_name: &str,
        _sample_rate: PhononSampleRate,
        _channels: u16,
        #[cfg_attr(not(windows), allow(unused_variables))] exclusive: bool,
    ) -> Result<(), DeviceError> {
        let device = self.find_device(device_name)?;

        #[cfg(windows)]
        {
            self.exclusive_mode = exclusive;
        }

        // Close existing stream
        self.current_stream = None;
        self.current_device = Some(device);

        Ok(())
    }

    /// Stop the current output stream, including any exclusive mode thread.
    pub fn stop_output(&mut self) {
        self.current_stream = None;
        #[cfg(windows)]
        {
            self.wasapi_stop.store(true, Ordering::SeqCst);
            if let Some(handle) = self.wasapi_thread.take() {
                log::info!("Waiting for WASAPI exclusive thread to finish...");
                let _ = handle.join();
            }
        }
    }

    /// Start playback with a callback that fills the output buffer.
    ///
    /// `use_pcm24` selects a 24-bit integer PCM transport for the WASAPI
    /// exclusive stream (samples arrive as exact-integer f32 values and are
    /// shifted into the 24-in-32 container). Required for DoP output;
    /// ignored in shared mode (the mixer only accepts float).
    /// `native_dsd` requests the raw-DSD container (bytes as exact-integer
    /// f32) — exclusive-only, like `use_pcm24`.
    pub fn start_playback<F>(
        &mut self,
        sample_rate: PhononSampleRate,
        channels: u16,
        #[cfg_attr(not(windows), allow(unused_variables))] use_pcm24: bool,
        #[cfg_attr(not(windows), allow(unused_variables))] native_dsd: bool,
        mut callback: F,
    ) -> Result<(), DeviceError>
    where
        F: FnMut(&mut [f32]) + Send + 'static,
    {
        // Stop any existing stream
        self.stop_output();

        #[cfg(windows)]
        if self.exclusive_mode {
            let mut cb = Some(callback);
            match self.start_wasapi_exclusive(sample_rate, channels, use_pcm24, native_dsd, &mut cb)
            {
                Ok(()) => return Ok(()),
                Err(e) => {
                    // Raw transports (DoP / native DSD) are meaningless in
                    // shared mode — the mixer would corrupt the encoded
                    // words. Propagate the error so the caller can retry
                    // with a different transport instead of emitting noise.
                    if use_pcm24 || native_dsd {
                        return Err(DeviceError::Stream(format!(
                            "exclusive open failed for raw transport: {e}"
                        )));
                    }
                    log::warn!(
                        "WASAPI exclusive mode failed: {e}. Falling back to shared mode. \
                         Try a different sample rate or check if your DAC supports exclusive mode."
                    );
                    self.exclusive_mode = false;
                    callback = cb.take().unwrap();
                    // Fall through to shared mode
                }
            }
        }

        let device = self.current_device.as_ref().ok_or(DeviceError::NoDevice)?;

        // Verify the requested sample rate is supported; if not, find a supported one.
        let actual_rate = if self.supports_rate(sample_rate) {
            sample_rate
        } else {
            let fallback = if sample_rate < 44100 { 44100 } else { 48000 };
            log::warn!(
                "Sample rate {}Hz not supported by device, trying {}Hz",
                sample_rate,
                fallback
            );
            // Find a rate the device actually supports
            if let Ok(configs) = device.supported_output_configs() {
                let mut best = fallback;
                for cfg in configs {
                    let min = cfg.min_sample_rate().0;
                    let max = cfg.max_sample_rate().0;
                    if fallback >= min && fallback <= max {
                        best = fallback;
                        break;
                    }
                    // Pick the closest rate within range
                    if sample_rate >= min && sample_rate <= max {
                        best = sample_rate;
                        break;
                    }
                    best = max; // Use the max supported rate as last resort
                }
                best
            } else {
                fallback
            }
        };

        let config = StreamConfig {
            channels: channels as cpal::ChannelCount,
            sample_rate: SampleRate(actual_rate),
            buffer_size: cpal::BufferSize::Default,
        };

        // Shared mode telemetry (exclusive path stamps its own in the pump).
        self.diagnostics.mode.store(0, Ordering::Relaxed);
        self.diagnostics.format.store(4, Ordering::Relaxed); // float mixer
        self.diagnostics.buffer_frames.store(0, Ordering::Relaxed);
        self.diagnostics.padding_frames.store(0, Ordering::Relaxed);

        let stream = device
            .build_output_stream(
                &config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    callback(data);
                },
                |err| {
                    log::error!("Audio stream error: {}", err);
                },
                None,
            )
            .map_err(|e| DeviceError::Stream(e.to_string()))?;

        stream
            .play()
            .map_err(|e| DeviceError::Stream(e.to_string()))?;
        self.current_stream = Some(stream);

        Ok(())
    }

    /// Start WASAPI exclusive mode playback.
    /// Uses a channel to confirm the thread started successfully before
    /// returning Ok; if the thread fails, the callback is recovered for
    /// shared-mode fallback.
    #[cfg(windows)]
    fn start_wasapi_exclusive<F>(
        &mut self,
        sample_rate: PhononSampleRate,
        channels: u16,
        use_pcm24: bool,
        native_dsd: bool,
        callback: &mut Option<F>,
    ) -> Result<(), DeviceError>
    where
        F: FnMut(&mut [f32]) + Send + 'static,
    {
        let cb = callback
            .take()
            .ok_or(DeviceError::Stream("callback already consumed".into()))?;
        // Share the callback via Arc<Mutex<>> so the thread can take it after
        // setup succeeds, and we can recover it if setup fails.
        let shared_cb = Arc::new(Mutex::new(cb));
        let thread_cb = shared_cb.clone();

        let device_name = self.current_device_name().ok_or(DeviceError::NoDevice)?;
        let diagnostics = self.diagnostics.clone();

        let stop = Arc::new(AtomicBool::new(false));
        self.wasapi_stop = stop.clone();
        let stop_for_thread = stop.clone();

        let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();

        // Spawn the WASAPI thread: COM init, stream creation, and the
        // entire audio loop all happen on the same thread to avoid
        // COM apartment violations that cause STATUS_STACK_BUFFER_OVERRUN.
        let handle = std::thread::spawn(move || {
            // Initialize COM on this thread (MTA for multi-threaded access)
            let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            if hr.is_err() {
                log::error!("WASAPI thread: CoInitializeEx failed: {hr:?}");
                let _ = tx.send(Err(format!("CoInitializeEx failed: {hr:?}")));
                return;
            }

            let stream = match crate::wasapi::WasapiExclusiveStream::new(
                &device_name,
                sample_rate,
                channels,
                use_pcm24,
                native_dsd,
            ) {
                Ok(s) => s,
                Err(e) => {
                    log::error!("WASAPI exclusive stream creation failed: {e}");
                    let _ = tx.send(Err(e));
                    unsafe {
                        CoUninitialize();
                    }
                    return;
                }
            };

            // PRIME the buffer before Start(). A freshly-Initialized
            // exclusive stream reports padding == buffer_size even though
            // nothing was written, so write_pending would refuse to write —
            // prime() bypasses that via direct GetBuffer. Without this the
            // event-driven pump wedges: device held, callback never runs,
            // no sound, progress frozen.
            let buf_size = stream.buffer_size_frames() as usize * channels as usize;
            {
                let mut prime_buf = vec![0.0f32; buf_size];
                thread_cb.lock().unwrap()(&mut prime_buf);
                if let Err(e) = stream.prime(&prime_buf) {
                    log::error!("WASAPI exclusive prime failed: {e}");
                    let _ = tx.send(Err(e));
                    unsafe {
                        CoUninitialize();
                    }
                    return;
                }
            }

            // Stamp telemetry: exclusive + negotiated container + buffer.
            diagnostics.mode.store(1, Ordering::Relaxed);
            diagnostics
                .format
                .store(stream.format_code(), Ordering::Relaxed);
            diagnostics
                .buffer_frames
                .store(stream.buffer_size_frames(), Ordering::Relaxed);

            if let Err(e) = stream.start() {
                log::error!("WASAPI exclusive start failed: {e}");
                let _ = tx.send(Err(e));
                unsafe {
                    CoUninitialize();
                }
                return;
            }

            // Signal success — caller will now keep the callback and not fall back
            if tx.send(Ok(())).is_err() {
                // Caller dropped the receiver — abort
                unsafe {
                    CoUninitialize();
                }
                return;
            }

            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut buf = vec![0.0f32; buf_size];
                loop {
                    if stop_for_thread.load(Ordering::SeqCst) {
                        break;
                    }
                    // POLLING pump: query writable frames every ~5 ms. The
                    // probe harness proved this device family never consumes
                    // in event-driven exclusive mode (padding stuck at full),
                    // so polling is the only reliable driver here.
                    let available = match stream.poll_available() {
                        Ok(n) => n as usize,
                        Err(e) => {
                            log::error!("WASAPI exclusive poll error: {e}");
                            break;
                        }
                    };
                    diagnostics.padding_frames.store(
                        (buf_size / channels as usize - available) as u32,
                        Ordering::Relaxed,
                    );
                    if available == 0 {
                        std::thread::sleep(std::time::Duration::from_millis(2));
                        continue;
                    }
                    let need = (available * channels as usize).min(buf.len());
                    thread_cb.lock().unwrap()(&mut buf[..need]);
                    if let Err(e) = stream.write_pending(&buf[..need]) {
                        log::error!("WASAPI exclusive write error: {e}");
                        break;
                    }
                    // Yield briefly; the 200 ms buffer gives ~40 cycles of slack.
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            }));

            if let Err(e) = result {
                log::error!("WASAPI exclusive thread panicked: {:?}", e);
            }

            let _ = stream.stop();
            // stream is dropped here, releasing COM resources
            unsafe {
                CoUninitialize();
            }
        });

        // Wait for the thread to report setup success/failure.
        // If it fails, recover the callback for shared-mode fallback.
        match rx.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(Ok(())) => {
                self.wasapi_thread = Some(handle);
                Ok(())
            }
            Ok(Err(e)) => {
                // Thread failed during setup — recover callback
                let _ = handle.join();
                if let Ok(mutex) = Arc::try_unwrap(shared_cb) {
                    *callback = Some(mutex.into_inner().unwrap());
                }
                Err(DeviceError::Stream(e))
            }
            Err(_) => {
                // Timeout or channel disconnected
                stop.store(true, Ordering::SeqCst);
                let _ = handle.join();
                if let Ok(mutex) = Arc::try_unwrap(shared_cb) {
                    *callback = Some(mutex.into_inner().unwrap());
                }
                Err(DeviceError::Stream(
                    "WASAPI thread startup timed out".into(),
                ))
            }
        }
    }

    /// Enable or disable exclusive mode.
    /// Requires restarting playback to take effect.
    pub fn set_exclusive_mode(&mut self, exclusive: bool) {
        self.exclusive_mode = exclusive;
    }

    /// Check if exclusive mode is currently active.
    pub fn is_exclusive(&self) -> bool {
        self.exclusive_mode
    }

    /// Find a device by name.
    fn find_device(&self, name: &str) -> Result<Device, DeviceError> {
        let devices = self
            .host
            .output_devices()
            .map_err(|e| DeviceError::Enumeration(e.to_string()))?;

        for device in devices {
            if let Ok(device_name) = device.name() {
                if device_name == name {
                    return Ok(device);
                }
            }
        }

        // Fallback to default device
        self.host
            .default_output_device()
            .ok_or(DeviceError::DeviceNotFound(name.to_string()))
    }

    /// Check if the device supports the given sample rate.
    pub fn supports_rate(&self, rate: PhononSampleRate) -> bool {
        if let Some(ref device) = self.current_device {
            if let Ok(configs) = device.supported_output_configs() {
                for cfg in configs {
                    let min = cfg.min_sample_rate().0;
                    let max = cfg.max_sample_rate().0;
                    if rate >= min && rate <= max {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Find the best output sample rate for a given input rate.
    /// Returns the closest supported rate, preferring exact match then higher rates.
    pub fn best_output_rate(&self, input_rate: PhononSampleRate) -> PhononSampleRate {
        // Common sample rates in order of preference
        let common_rates = [44100, 48000, 88200, 96000, 176400, 192000, 352800, 384000];

        // Check if input rate is supported
        if self.supports_rate(input_rate) {
            return input_rate;
        }

        // Find the closest supported rate from common rates
        if let Some(ref device) = self.current_device {
            if let Ok(mut configs) = device.supported_output_configs() {
                if let Some(cfg) = configs.next() {
                    let min = cfg.min_sample_rate().0;
                    let max = cfg.max_sample_rate().0;

                    // Try higher rates first (less quality loss)
                    for &r in &common_rates {
                        if r >= input_rate && r >= min && r <= max {
                            return r;
                        }
                    }
                    // Fall back to lower rates
                    for &r in common_rates.iter().rev() {
                        if r <= input_rate && r >= min && r <= max {
                            return r;
                        }
                    }
                    // If nothing matches, return the max supported
                    return max;
                }
            }
        }
        // Default fallback
        48000
    }

    /// Create a resampler for converting between sample rates.
    pub fn create_resampler(
        input_rate: PhononSampleRate,
        output_rate: PhononSampleRate,
        channels: usize,
    ) -> Result<FftFixedIn<f32>, DeviceError> {
        FftFixedIn::new(input_rate as usize, output_rate as usize, 1024, 1, channels)
            .map_err(|e| DeviceError::Resampler(e.to_string()))
    }

    // ── Hot-plug detection ──────────────────────────────────────

    /// Check for device changes since last poll.
    /// Returns a list of DeviceEvent if devices were added or removed.
    pub fn check_hotplug(&self) -> Vec<DeviceEvent> {
        let current = match self.list_devices() {
            Ok(devices) => devices,
            Err(_) => return Vec::new(),
        };

        let current_names: Vec<String> = current.iter().map(|d| d.name.clone()).collect();
        let mut last = self.last_devices.lock().unwrap();
        let mut events = Vec::new();

        // Detect added devices
        for name in &current_names {
            if !last.contains(name) {
                if let Some(info) = current.iter().find(|d| &d.name == name) {
                    log::info!("Device added: {}", name);
                    events.push(DeviceEvent::DeviceAdded(info.clone()));
                }
            }
        }

        // Detect removed devices
        for name in last.iter() {
            if !current_names.contains(name) {
                log::info!("Device removed: {}", name);
                events.push(DeviceEvent::DeviceRemoved(name.clone()));
            }
        }

        // Update last known list
        *last = current_names;

        // Emit events via broadcast channel
        for event in &events {
            let _ = self.event_tx.send(event.clone());
        }

        events
    }

    /// Start a background monitor thread that periodically polls for device changes.
    /// The callback is invoked with each DeviceEvent.
    pub fn start_hotplug_monitor<F>(&mut self, interval_ms: u64, mut callback: F)
    where
        F: FnMut(DeviceEvent) + Send + 'static,
    {
        self.hotplug_stop.store(false, Ordering::SeqCst);

        let last_devices = self.last_devices.clone();
        let stop = self.hotplug_stop.clone();

        let handle = thread::spawn(move || {
            let host = cpal::default_host();

            // Initialize last known list
            {
                let mut last = last_devices.lock().unwrap();
                if let Ok(devices) = host.output_devices() {
                    *last = devices.filter_map(|d| d.name().ok()).collect();
                }
            }

            loop {
                if stop.load(Ordering::SeqCst) {
                    break;
                }

                thread::sleep(Duration::from_millis(interval_ms));

                // Poll current devices
                let current_names: Vec<String> = match host.output_devices() {
                    Ok(devices) => devices.filter_map(|d| d.name().ok()).collect(),
                    Err(_) => continue,
                };

                let mut last = last_devices.lock().unwrap();

                // Check for added devices
                for name in &current_names {
                    if !last.contains(name) {
                        log::info!("[hotplug] Device added: {}", name);
                        callback(DeviceEvent::DeviceAdded(DeviceInfo {
                            name: name.clone(),
                            id: name.clone(),
                            is_default: false,
                            supported_sample_rates: vec![44100, 48000],
                            max_channels: 2,
                            supports_exclusive: cfg!(target_os = "windows"),
                        }));
                    }
                }

                // Check for removed devices
                for name in last.iter() {
                    if !current_names.contains(name) {
                        log::info!("[hotplug] Device removed: {}", name);
                        callback(DeviceEvent::DeviceRemoved(name.clone()));
                    }
                }

                *last = current_names;
            }
        });

        self.hotplug_thread = Some(handle);
    }

    /// Stop the hotplug monitor thread and join it.
    pub fn stop_hotplug_monitor(&mut self) {
        self.hotplug_stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.hotplug_thread.take() {
            // Join with timeout using a channel-based rendezvous.
            let (tx, rx) = mpsc::channel();
            thread::spawn(move || {
                let _ = handle.join();
                let _ = tx.send(());
            });
            match rx.recv_timeout(Duration::from_millis(1000)) {
                Ok(_) => log::info!("[hotplug] Monitor thread joined successfully"),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    log::warn!("[hotplug] Monitor thread join timed out, detaching")
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    log::warn!("[hotplug] Monitor thread join channel disconnected")
                }
            }
        }
    }
}

// SAFETY: On Windows (WASAPI), cpal::Stream is thread-safe.
// On other platforms, the DeviceManager is always wrapped in Arc<Mutex<>>.
unsafe impl Send for DeviceManager {}
unsafe impl Sync for DeviceManager {}

/// Errors for device management.
#[derive(Debug, thiserror::Error)]
pub enum DeviceError {
    #[error("No audio device available")]
    NoDevice,
    #[error("Device not found: {0}")]
    DeviceNotFound(String),
    #[error("Device enumeration error: {0}")]
    Enumeration(String),
    #[error("Stream error: {0}")]
    Stream(String),
    #[error("Resampler error: {0}")]
    Resampler(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_device_error_display() {
        let err = DeviceError::NoDevice;
        assert_eq!(err.to_string(), "No audio device available");

        let err = DeviceError::DeviceNotFound("TestDAC".into());
        assert!(err.to_string().contains("TestDAC"));
    }

    #[test]
    fn test_device_info_defaults() {
        let info = DeviceInfo {
            name: "Test".into(),
            id: "test-id".into(),
            is_default: true,
            supported_sample_rates: vec![44100, 48000, 96000],
            max_channels: 2,
            supports_exclusive: true,
        };
        assert!(info.is_default);
        assert_eq!(info.max_channels, 2);
        assert_eq!(info.supported_sample_rates.len(), 3);
    }
}
