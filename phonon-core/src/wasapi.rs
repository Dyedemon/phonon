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
use windows::Win32::System::Threading::*;
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
const KSDATAFORMAT_SUBTYPE_PCM_GUID: GUID =
    GUID::from_u128(0x00000001_0000_0010_8000_00aa00389b71);

// Channel masks
const SPEAKER_FRONT_LEFT: u32 = 0x1;
const SPEAKER_FRONT_RIGHT: u32 = 0x2;
const SPEAKER_FRONT_CENTER: u32 = 0x4;

// ── Error helpers ──────────────────────────────────────────────

fn win_err(e: windows::core::Error) -> String {
    e.to_string()
}

// ── WASAPI Exclusive Stream ────────────────────────────────────

pub struct WasapiExclusiveStream {
    audio_client: IAudioClient,
    render_client: IAudioRenderClient,
    event_handle: HANDLE,
    buffer_size_frames: u32,
    pub sample_rate: u32,
    pub channels: u16,
    /// 24-bit PCM transport (24 valid bits left-aligned in a 32-bit
    /// container) instead of IEEE float. Required for DoP: the DAC must
    /// receive the raw 24-bit DoP words, not floats.
    use_pcm24: bool,
    active: AtomicBool,
}

unsafe impl Send for WasapiExclusiveStream {}
unsafe impl Sync for WasapiExclusiveStream {}

impl WasapiExclusiveStream {
    /// Create a new WASAPI exclusive stream.
    /// Caller must have called `CoInitializeEx` on the current thread.
    pub fn new(
        device_name: &str,
        sample_rate: u32,
        channels: u16,
        use_pcm24: bool,
    ) -> std::result::Result<Self, String> {
        let device_enumerator: IMMDeviceEnumerator =
            unsafe { CoCreateInstance(&CLSID_MMDEVICE_ENUMERATOR, None, CLSCTX_ALL) }
                .map_err(win_err)?;

        let device = find_device_by_name(&device_enumerator, device_name)?;

        let audio_client: IAudioClient =
            unsafe { device.Activate(CLSCTX_ALL, None) }.map_err(win_err)?;

        let waveformatex = build_wave_format(sample_rate, channels, use_pcm24);

        let hns_buffer_duration = 10_000_000; // 1 second
        unsafe {
            audio_client.Initialize(
                AUDCLNT_SHAREMODE_EXCLUSIVE,
                AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
                hns_buffer_duration,
                hns_buffer_duration,
                &waveformatex as *const _ as *const WAVEFORMATEX,
                None,
            )
        }
        .map_err(|e| format!("IAudioClient::Initialize failed: {e}"))?;

        let buffer_size_frames = unsafe { audio_client.GetBufferSize() }.map_err(win_err)?;

        let render_client: IAudioRenderClient =
            unsafe { audio_client.GetService() }.map_err(win_err)?;

        let event_handle = unsafe { CreateEventW(None, false, false, None) }.map_err(win_err)?;

        unsafe { audio_client.SetEventHandle(event_handle) }
            .map_err(|e| format!("SetEventHandle failed: {e}"))?;

        Ok(Self {
            audio_client,
            render_client,
            event_handle,
            buffer_size_frames,
            sample_rate,
            channels,
            use_pcm24,
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

    /// Write PCM f32 data. Blocks until buffer available or timeout.
    pub fn write(&self, data: &[f32], timeout_ms: u32) -> std::result::Result<u32, String> {
        match self.wait_for_space(timeout_ms)? {
            None => Ok(0),
            Some(_) => self.write_pending(data),
        }
    }

    /// Wait for the device buffer event and report how many frames can be
    /// written. `Ok(None)` means the wait timed out (no space report).
    /// Splitting this from the actual write lets the pump pull exactly the
    /// writable amount from the engine, so no samples are ever dropped.
    pub fn wait_for_space(
        &self,
        timeout_ms: u32,
    ) -> std::result::Result<Option<u32>, String> {
        let wait_result = unsafe { WaitForSingleObject(self.event_handle, timeout_ms) };
        if wait_result == WAIT_TIMEOUT {
            return Ok(None);
        }
        if wait_result != WAIT_OBJECT_0 {
            return Err(format!("WaitForSingleObject failed: {wait_result:?}"));
        }

        let padding = unsafe { self.audio_client.GetCurrentPadding() }.map_err(win_err)?;
        Ok(Some(self.buffer_size_frames.saturating_sub(padding)))
    }

    /// Write PCM f32 data previously accounted for via `wait_for_space`.
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
        if self.use_pcm24 {
            // 24 valid bits left-aligned in a 32-bit container: shift the
            // integer sample into bits 31..8. DoP samples arrive as
            // exact-integer f32 values from the engine.
            let buffer_i32 =
                unsafe { std::slice::from_raw_parts_mut(buffer as *mut i32, sample_count) };
            for (dst, &src) in buffer_i32
                .iter_mut()
                .zip(data[..sample_count].iter())
            {
                *dst = (src as i32).wrapping_shl(8);
            }
        } else {
            let buffer_f32 =
                unsafe { std::slice::from_raw_parts_mut(buffer as *mut f32, sample_count) };
            buffer_f32.copy_from_slice(&data[..sample_count]);
        }

        unsafe { self.render_client.ReleaseBuffer(frames_to_write, 0) }
            .map_err(|e| format!("ReleaseBuffer failed: {e}"))?;

        Ok(frames_to_write)
    }

    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }
    pub fn event_handle(&self) -> HANDLE {
        self.event_handle
    }
    pub fn buffer_size_frames(&self) -> u32 {
        self.buffer_size_frames
    }
}

impl Drop for WasapiExclusiveStream {
    fn drop(&mut self) {
        if self.active.load(Ordering::SeqCst) {
            let _ = self.stop();
        }
        if !self.event_handle.is_invalid() {
            unsafe {
                let _ = CloseHandle(self.event_handle);
            }
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
///
/// `use_pcm24` selects a 24-bit integer PCM format (24 valid bits
/// left-aligned in a 32-bit container) — required for DoP — instead of the
/// default IEEE float format.
fn build_wave_format(sample_rate: u32, channels: u16, use_pcm24: bool) -> WAVEFORMATEXTENSIBLE {
    let bytes_per_sample = 4u16;
    let block_align = channels * bytes_per_sample;
    let avg_bytes_per_sec = sample_rate * block_align as u32;

    let samples = WAVEFORMATEXTENSIBLE_0 {
        wValidBitsPerSample: if use_pcm24 { 24 } else { 32 },
    };

    let channel_mask = match channels {
        1 => SPEAKER_FRONT_CENTER,
        2 => SPEAKER_FRONT_LEFT | SPEAKER_FRONT_RIGHT,
        _ => SPEAKER_FRONT_LEFT | SPEAKER_FRONT_RIGHT,
    };

    WAVEFORMATEXTENSIBLE {
        Format: WAVEFORMATEX {
            wFormatTag: WAVE_FORMAT_EXTENSIBLE_VAL,
            nChannels: channels,
            nSamplesPerSec: sample_rate,
            nAvgBytesPerSec: avg_bytes_per_sec,
            nBlockAlign: block_align,
            wBitsPerSample: 32,
            cbSize: 22,
        },
        Samples: samples,
        dwChannelMask: channel_mask,
        SubFormat: if use_pcm24 {
            KSDATAFORMAT_SUBTYPE_PCM_GUID
        } else {
            KSDATAFORMAT_SUBTYPE_IEEE_FLOAT
        },
    }
}
