//! WASAPI exclusive mode & hardware volume control (Windows only).
//!
//! Bypasses cpal for exclusive-mode WASAPI streaming and provides
//! IAudioEndpointVolume for hardware volume control.

#![cfg(windows)]

use std::sync::atomic::{AtomicBool, Ordering};

use windows::core::BSTR;
use windows::core::GUID;
use windows::Win32::Foundation::*;
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::*;
use windows::Win32::Media::Multimedia::*;
use windows::Win32::System::Com::*;
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::Win32::UI::Shell::PropertiesSystem::PROPERTYKEY;

// ── GUIDs ──────────────────────────────────────────────────────

const CLSID_MMDEVICE_ENUMERATOR: GUID = GUID::from_u128(0xBCDE0395_E52F_467C_8E3D_C4579291692E);

// PKEY_Device_FriendlyName (from FunctionDiscovery, defined manually)
const PKEY_DEVICE_FRIENDLY_NAME: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0),
    pid: 14,
};

// WAVE_FORMAT_EXTENSIBLE = 0xFFFE
const WAVE_FORMAT_EXTENSIBLE_VAL: u16 = 0xFFFE;

// KSDATAFORMAT_SUBTYPE_PCM = {00000001-0000-0010-8000-00aa00389b71}
const KSDATAFORMAT_SUBTYPE_PCM_GUID: GUID = GUID::from_u128(0x00000001_0000_0010_8000_00aa00389b71);

// Native-DSD subformat used by Thesycon-based DAC drivers
// ({00000000-0000-0010-8000-00AA00389B71}): one DSD byte (8 bits, MSB
// first) per channel per frame, nSamplesPerSec = DSD bit rate. Windows has
// no standardized DSD WASAPI path — this convention is what Amanero/XMOS/
// Singxer class drivers accept, and the IsFormatSupported probe gates it:
// drivers that don't expose DSD simply reject the format and we fall back
// to DoP.
const KSDATAFORMAT_SUBTYPE_DSD_GUID: GUID = GUID::from_u128(0x00000000_0000_0010_8000_00aa00389b71);

// Channel masks (WAVE_SPEAKER_* bits)
const SPEAKER_FRONT_LEFT: u32 = 0x1;
const SPEAKER_FRONT_RIGHT: u32 = 0x2;
const SPEAKER_FRONT_CENTER: u32 = 0x4;
const SPEAKER_LOW_FREQUENCY: u32 = 0x8;
const SPEAKER_BACK_LEFT: u32 = 0x10;
const SPEAKER_BACK_RIGHT: u32 = 0x20;
const SPEAKER_SIDE_LEFT: u32 = 0x200;
const SPEAKER_SIDE_RIGHT: u32 = 0x400;

/// Standard WAVE speaker mask for a channel count (5.1 = 0x3F, 7.1 = 0x63F).
fn speaker_mask(channels: u16) -> u32 {
    match channels {
        1 => SPEAKER_FRONT_CENTER,
        2 => SPEAKER_FRONT_LEFT | SPEAKER_FRONT_RIGHT,
        6 => {
            SPEAKER_FRONT_LEFT
                | SPEAKER_FRONT_RIGHT
                | SPEAKER_FRONT_CENTER
                | SPEAKER_LOW_FREQUENCY
                | SPEAKER_BACK_LEFT
                | SPEAKER_BACK_RIGHT
        }
        8 => {
            SPEAKER_FRONT_LEFT
                | SPEAKER_FRONT_RIGHT
                | SPEAKER_FRONT_CENTER
                | SPEAKER_LOW_FREQUENCY
                | SPEAKER_BACK_LEFT
                | SPEAKER_BACK_RIGHT
                | SPEAKER_SIDE_LEFT
                | SPEAKER_SIDE_RIGHT
        }
        // Generic mask: the first `channels` speaker bits.
        n if n > 0 => (1u32 << n) - 1,
        _ => SPEAKER_FRONT_LEFT | SPEAKER_FRONT_RIGHT,
    }
}

// ── Error helpers ──────────────────────────────────────────────

fn win_err(e: windows::core::Error) -> String {
    e.to_string()
}

// ── WASAPI Exclusive Stream ────────────────────────────────────

pub struct WasapiExclusiveStream {
    audio_client: IAudioClient,
    render_client: IAudioRenderClient,
    buffer_size_frames: u32,
    pub sample_rate: u32,
    pub channels: u16,
    /// Negotiated sample container used for both the WASAPI format and the
    /// f32 → device conversion in `write_pending`.
    format: ExclusivePcmFormat,
    active: AtomicBool,
}

unsafe impl Send for WasapiExclusiveStream {}
unsafe impl Sync for WasapiExclusiveStream {}

/// Sample container for exclusive-mode WASAPI streaming.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExclusivePcmFormat {
    /// DoP: bit-exact 24-bit words left-aligned in a 32-bit container.
    Dop24,
    /// 24-bit PCM, 24 valid bits left-aligned in a 32-bit container.
    Pcm24,
    /// 16-bit integer PCM.
    Pcm16,
    /// 32-bit IEEE float.
    Float32,
    /// Native DSD: one DSD byte (8 bits, MSB first) per channel per frame;
    /// the engine delivers bytes as exact-integer f32 values (0..255).
    DsdNative,
}

impl WasapiExclusiveStream {
    /// Create a new WASAPI exclusive stream.
    /// Caller must have called `CoInitializeEx` on the current thread.
    pub fn new(
        device_name: &str,
        sample_rate: u32,
        channels: u16,
        use_pcm24: bool,
        native_dsd: bool,
    ) -> std::result::Result<Self, String> {
        let device_enumerator: IMMDeviceEnumerator =
            unsafe { CoCreateInstance(&CLSID_MMDEVICE_ENUMERATOR, None, CLSCTX_ALL) }
                .map_err(win_err)?;

        let device = find_device_by_name(&device_enumerator, device_name)?;

        let audio_client: IAudioClient =
            unsafe { device.Activate(CLSCTX_ALL, None) }.map_err(win_err)?;

        // WASAPI exclusive mode only accepts a format the device explicitly
        // supports; a blind Initialize almost always fails with
        // AUDCLNT_E_UNSUPPORTED_FORMAT (consumer devices rarely accept
        // 32-bit float exclusively). Probe with IsFormatSupported and walk
        // a quality-ordered format ladder instead.
        // DoP must keep its 24-bit PCM container bit-exact — no fallback.
        // Native DSD is probed with the DSD subformat alone: if the driver
        // doesn't expose it, the caller falls back to DoP/PCM.
        let candidates: &[ExclusivePcmFormat] = if native_dsd {
            &[ExclusivePcmFormat::DsdNative]
        } else if use_pcm24 {
            &[ExclusivePcmFormat::Dop24]
        } else {
            &[
                ExclusivePcmFormat::Pcm24,
                ExclusivePcmFormat::Pcm16,
                ExclusivePcmFormat::Float32,
            ]
        };

        let mut format: Option<ExclusivePcmFormat> = None;
        let mut last_probe_err: Option<windows::core::HRESULT> = None;
        for &candidate in candidates {
            let waveformatex = build_wave_format(sample_rate, channels, candidate);
            let probe = unsafe {
                audio_client.IsFormatSupported(
                    AUDCLNT_SHAREMODE_EXCLUSIVE,
                    &waveformatex as *const _ as *const WAVEFORMATEX,
                    None,
                )
            };
            // Exclusive mode requires an EXACT match (S_OK). S_FALSE means
            // "closest match returned" — Initialize would fail with that
            // format, so it must be treated as unsupported.
            if probe == windows::Win32::Foundation::S_OK {
                format = Some(candidate);
                break;
            }
            last_probe_err = Some(probe);
        }

        let format = format.ok_or_else(|| {
            let tried = candidates
                .iter()
                .map(|c| format!("{c:?}"))
                .collect::<Vec<_>>()
                .join(", ");
            let hr = last_probe_err
                .map(|e| format!(" (last HRESULT 0x{:08X})", e.0))
                .unwrap_or_default();
            format!(
                "no supported exclusive format at {sample_rate}Hz/{channels}ch — tried [{tried}]{hr}"
            )
        })?;
        log::info!(
            "WASAPI exclusive: negotiated format {format:?} at {sample_rate}Hz/{channels}ch"
        );

        let waveformatex = build_wave_format(sample_rate, channels, format);

        // POLLING mode: no EVENTCALLBACK flag, periodicity = 0, 200 ms
        // buffer. The probe harness showed this driver never consumes in
        // event-driven exclusive mode (padding stuck at full regardless of
        // configuration) but consumes reliably when polled.
        let hns_buffer_duration = 2_000_000; // 200 ms
        unsafe {
            audio_client.Initialize(
                AUDCLNT_SHAREMODE_EXCLUSIVE,
                0,
                hns_buffer_duration,
                0,
                &waveformatex as *const _ as *const WAVEFORMATEX,
                None,
            )
        }
        .map_err(|e| format!("IAudioClient::Initialize failed: {e}"))?;

        let buffer_size_frames = unsafe { audio_client.GetBufferSize() }.map_err(win_err)?;

        let render_client: IAudioRenderClient =
            unsafe { audio_client.GetService() }.map_err(win_err)?;

        Ok(Self {
            audio_client,
            render_client,
            buffer_size_frames,
            sample_rate,
            channels,
            format,
            active: AtomicBool::new(false),
        })
    }

    pub fn start(&self) -> std::result::Result<(), String> {
        if self.active.load(Ordering::SeqCst) {
            return Ok(());
        }
        unsafe { self.audio_client.Start() }
            .map_err(|e| format!("IAudioClient::Start failed: {e}"))?;
        self.active.store(true, Ordering::SeqCst);
        Ok(())
    }

    pub fn stop(&self) -> std::result::Result<(), String> {
        if !self.active.load(Ordering::SeqCst) {
            return Ok(());
        }
        unsafe { self.audio_client.Stop() }
            .map_err(|e| format!("IAudioClient::Stop failed: {e}"))?;
        self.active.store(false, Ordering::SeqCst);
        Ok(())
    }

    pub fn reset(&self) -> std::result::Result<(), String> {
        unsafe { self.audio_client.Reset() }.map_err(|e| format!("IAudioClient::Reset failed: {e}"))
    }

    /// Frames currently writable (buffer size minus padding). The pump
    /// polls this on a short interval — polling is the only mode this
    /// driver family consumes reliably in (event-driven exclusive wedges).
    pub fn poll_available(&self) -> std::result::Result<u32, String> {
        let padding = unsafe { self.audio_client.GetCurrentPadding() }.map_err(win_err)?;
        Ok(self.buffer_size_frames.saturating_sub(padding))
    }

    /// Write PCM f32 data previously accounted for via `poll_available`.
    /// Writes at most `data.len() / channels` frames; returns frames written.
    pub fn write_pending(&self, data: &[f32]) -> std::result::Result<u32, String> {
        let padding = unsafe { self.audio_client.GetCurrentPadding() }.map_err(win_err)?;

        let frames_available = self.buffer_size_frames.saturating_sub(padding);
        let frames_to_write =
            (data.len() / self.channels as usize).min(frames_available as usize) as u32;
        if frames_to_write == 0 {
            return Ok(0);
        }

        let buffer: *mut u8 =
            unsafe { self.render_client.GetBuffer(frames_to_write) }.map_err(win_err)?;

        let sample_count = (frames_to_write as usize) * (self.channels as usize);
        match self.format {
            ExclusivePcmFormat::Dop24 => {
                // 24 valid bits left-aligned in a 32-bit container: the DoP
                // words arrive as exact-integer f32 values from the engine —
                // shift them into place bit-exact, no scaling.
                let buffer_i32 =
                    unsafe { std::slice::from_raw_parts_mut(buffer as *mut i32, sample_count) };
                for (dst, &src) in buffer_i32.iter_mut().zip(data[..sample_count].iter()) {
                    *dst = (src as i32).wrapping_shl(8);
                }
            }
            ExclusivePcmFormat::DsdNative => {
                // Raw DSD: the engine delivers one byte (8 DSD bits) per
                // channel per frame as an exact-integer f32; copy out the
                // byte stream unmodified.
                let buffer_u8 = unsafe { std::slice::from_raw_parts_mut(buffer, sample_count) };
                for (dst, &src) in buffer_u8.iter_mut().zip(data[..sample_count].iter()) {
                    *dst = src.clamp(0.0, 255.0) as u8;
                }
            }
            ExclusivePcmFormat::Pcm24 => {
                // 24 valid bits left-aligned in a 32-bit container.
                let buffer_i32 =
                    unsafe { std::slice::from_raw_parts_mut(buffer as *mut i32, sample_count) };
                for (dst, &src) in buffer_i32.iter_mut().zip(data[..sample_count].iter()) {
                    let s = src.clamp(-1.0, 1.0);
                    *dst = ((s * 8_388_607.0) as i32).wrapping_shl(8);
                }
            }
            ExclusivePcmFormat::Pcm16 => {
                let buffer_i16 =
                    unsafe { std::slice::from_raw_parts_mut(buffer as *mut i16, sample_count) };
                for (dst, &src) in buffer_i16.iter_mut().zip(data[..sample_count].iter()) {
                    *dst = (src.clamp(-1.0, 1.0) * 32_767.0) as i16;
                }
            }
            ExclusivePcmFormat::Float32 => {
                let buffer_f32 =
                    unsafe { std::slice::from_raw_parts_mut(buffer as *mut f32, sample_count) };
                buffer_f32.copy_from_slice(&data[..sample_count]);
            }
        }

        unsafe { self.render_client.ReleaseBuffer(frames_to_write, 0) }
            .map_err(|e| format!("ReleaseBuffer failed: {e}"))?;

        Ok(frames_to_write)
    }

    /// Pre-fill the ENTIRE device buffer directly via GetBuffer, bypassing
    /// the padding check.
    ///
    /// REQUIRED before Start(): a freshly-Initialized exclusive stream
    /// reports padding == buffer_size (fully queued) even though nothing was
    /// written, so `write_pending` would refuse to write a single frame and
    /// the event-driven pump would wedge forever — device held, no sound.
    /// `data` should hold `buffer_size_frames * channels` samples; a shorter
    /// slice is zero-padded.
    pub fn prime(&self, data: &[f32]) -> std::result::Result<(), String> {
        unsafe {
            let buffer: *mut u8 = self
                .render_client
                .GetBuffer(self.buffer_size_frames)
                .map_err(win_err)?;
            let sample_count = self.buffer_size_frames as usize * self.channels as usize;
            self.fill_buffer(buffer, sample_count, data);
            self.render_client
                .ReleaseBuffer(self.buffer_size_frames, 0)
                .map_err(|e| format!("ReleaseBuffer failed: {e}"))?;
        }
        Ok(())
    }

    /// Convert `data` (f32 interleaved) into the negotiated sample container
    /// pointed to by `buffer`. `sample_count` is the sample (not byte) count.
    fn fill_buffer(&self, buffer: *mut u8, sample_count: usize, data: &[f32]) {
        match self.format {
            ExclusivePcmFormat::Dop24 => {
                // Bit-exact DoP words left-aligned in the 32-bit container.
                let b = unsafe { std::slice::from_raw_parts_mut(buffer as *mut i32, sample_count) };
                for (dst, &src) in b.iter_mut().zip(data.iter()) {
                    *dst = (src as i32).wrapping_shl(8);
                }
            }
            ExclusivePcmFormat::DsdNative => {
                let b = unsafe { std::slice::from_raw_parts_mut(buffer, sample_count) };
                for (dst, &src) in b.iter_mut().zip(data.iter()) {
                    *dst = src.clamp(0.0, 255.0) as u8;
                }
            }
            ExclusivePcmFormat::Pcm24 => {
                let b = unsafe { std::slice::from_raw_parts_mut(buffer as *mut i32, sample_count) };
                for (dst, &src) in b.iter_mut().zip(data.iter()) {
                    let s = src.clamp(-1.0, 1.0);
                    *dst = ((s * 8_388_607.0) as i32).wrapping_shl(8);
                }
            }
            ExclusivePcmFormat::Pcm16 => {
                let b = unsafe { std::slice::from_raw_parts_mut(buffer as *mut i16, sample_count) };
                for (dst, &src) in b.iter_mut().zip(data.iter()) {
                    *dst = (src.clamp(-1.0, 1.0) * 32_767.0) as i16;
                }
            }
            ExclusivePcmFormat::Float32 => {
                let b = unsafe { std::slice::from_raw_parts_mut(buffer as *mut f32, sample_count) };
                for (dst, &src) in b.iter_mut().zip(data.iter()) {
                    *dst = src;
                }
            }
        }
    }

    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }
    pub fn buffer_size_frames(&self) -> u32 {
        self.buffer_size_frames
    }
    /// Negotiated container code for the diagnostics panel
    /// (1 Dop24, 2 Pcm24, 3 Pcm16, 4 Float32, 5 Native DSD).
    pub fn format_code(&self) -> u8 {
        match self.format {
            ExclusivePcmFormat::Dop24 => 1,
            ExclusivePcmFormat::Pcm24 => 2,
            ExclusivePcmFormat::Pcm16 => 3,
            ExclusivePcmFormat::Float32 => 4,
            ExclusivePcmFormat::DsdNative => 5,
        }
    }
    /// Current device padding in frames (diagnostics).
    pub fn padding(&self) -> std::result::Result<u32, String> {
        unsafe { self.audio_client.GetCurrentPadding() }.map_err(win_err)
    }
}

impl Drop for WasapiExclusiveStream {
    fn drop(&mut self) {
        if self.active.load(Ordering::SeqCst) {
            let _ = self.stop();
        }
    }
}

// ── Hardware Volume Control ────────────────────────────────────

pub struct WasapiEndpointVolume {
    endpoint_volume: IAudioEndpointVolume,
}

impl Clone for WasapiEndpointVolume {
    fn clone(&self) -> Self {
        Self {
            endpoint_volume: self.endpoint_volume.clone(),
        }
    }
}

unsafe impl Send for WasapiEndpointVolume {}
unsafe impl Sync for WasapiEndpointVolume {}

impl WasapiEndpointVolume {
    /// Create a new endpoint volume controller.
    /// Caller must have called `CoInitializeEx` on the current thread.
    pub fn new(device_name: &str) -> std::result::Result<Self, String> {
        let device_enumerator: IMMDeviceEnumerator =
            unsafe { CoCreateInstance(&CLSID_MMDEVICE_ENUMERATOR, None, CLSCTX_ALL) }
                .map_err(win_err)?;

        let device = find_device_by_name(&device_enumerator, device_name)?;

        let endpoint_volume: IAudioEndpointVolume =
            unsafe { device.Activate(CLSCTX_ALL, None) }.map_err(win_err)?;

        Ok(Self { endpoint_volume })
    }

    pub fn set_master_volume(&self, level: f32) -> std::result::Result<(), String> {
        unsafe {
            self.endpoint_volume
                .SetMasterVolumeLevelScalar(level.clamp(0.0, 1.0), std::ptr::null())
        }
        .map_err(|e| format!("SetMasterVolumeLevelScalar failed: {e}"))
    }

    pub fn get_master_volume(&self) -> std::result::Result<f32, String> {
        unsafe { self.endpoint_volume.GetMasterVolumeLevelScalar() }
            .map_err(|e| format!("GetMasterVolumeLevelScalar failed: {e}"))
    }

    pub fn is_muted(&self) -> std::result::Result<bool, String> {
        let muted = unsafe { self.endpoint_volume.GetMute() }
            .map_err(|e| format!("GetMute failed: {e}"))?;
        Ok(muted.as_bool())
    }

    pub fn set_mute(&self, mute: bool) -> std::result::Result<(), String> {
        unsafe {
            self.endpoint_volume
                .SetMute(BOOL::from(mute), std::ptr::null())
        }
        .map_err(|e| format!("SetMute failed: {e}"))
    }
}

// ── Helpers ────────────────────────────────────────────────────

fn find_device_by_name(
    enumerator: &IMMDeviceEnumerator,
    name: &str,
) -> std::result::Result<IMMDevice, String> {
    let collection =
        unsafe { enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) }.map_err(win_err)?;

    let count = unsafe { collection.GetCount() }.map_err(win_err)?;

    for i in 0..count {
        let device = unsafe { collection.Item(i) }.map_err(win_err)?;

        let prop_store = unsafe { device.OpenPropertyStore(STGM_READ) }.map_err(win_err)?;

        let device_name = get_device_name(&prop_store)?;

        if device_name == name {
            return Ok(device);
        }
    }

    log::warn!("Device '{}' not found, falling back to default", name);
    unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole) }.map_err(win_err)
}

fn get_device_name(prop_store: &IPropertyStore) -> std::result::Result<String, String> {
    let value = unsafe { prop_store.GetValue(&PKEY_DEVICE_FRIENDLY_NAME) }.map_err(win_err)?;

    // PROPVARIANT → BSTR → String
    let bstr = BSTR::try_from(&value)
        .map_err(|e| format!("Failed to extract BSTR from PROPVARIANT: {e}"))?;
    Ok(bstr.to_string())
}

/// Build a WAVEFORMATEXTENSIBLE for exclusive-mode WASAPI.
fn build_wave_format(
    sample_rate: u32,
    channels: u16,
    format: ExclusivePcmFormat,
) -> WAVEFORMATEXTENSIBLE {
    let (bytes_per_sample, valid_bits, sub_format) = match format {
        ExclusivePcmFormat::Dop24 | ExclusivePcmFormat::Pcm24 => {
            (4u16, 24u16, KSDATAFORMAT_SUBTYPE_PCM_GUID)
        }
        ExclusivePcmFormat::Pcm16 => (2u16, 16u16, KSDATAFORMAT_SUBTYPE_PCM_GUID),
        ExclusivePcmFormat::Float32 => (4u16, 32u16, KSDATAFORMAT_SUBTYPE_IEEE_FLOAT),
        ExclusivePcmFormat::DsdNative => (1u16, 8u16, KSDATAFORMAT_SUBTYPE_DSD_GUID),
    };
    let block_align = channels * bytes_per_sample;
    let avg_bytes_per_sec = sample_rate * block_align as u32;

    let samples = WAVEFORMATEXTENSIBLE_0 {
        wValidBitsPerSample: valid_bits,
    };

    let channel_mask = speaker_mask(channels);

    WAVEFORMATEXTENSIBLE {
        Format: WAVEFORMATEX {
            wFormatTag: WAVE_FORMAT_EXTENSIBLE_VAL,
            nChannels: channels,
            nSamplesPerSec: sample_rate,
            nAvgBytesPerSec: avg_bytes_per_sec,
            nBlockAlign: block_align,
            wBitsPerSample: bytes_per_sample * 8,
            cbSize: 22,
        },
        Samples: samples,
        dwChannelMask: channel_mask,
        SubFormat: sub_format,
    }
}
