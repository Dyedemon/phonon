//! Raw-WASAPI multi-configuration probe: find which exclusive-mode
//! initialization the device actually consumes audio with.
//! NOT shipped — diagnostics only.

#![allow(unused)]

use windows::core::GUID;
use windows::Win32::Foundation::*;
use windows::Win32::Media::Audio::*;
use windows::Win32::Media::KernelStreaming::KSDATAFORMAT_SUBTYPE_PCM;
use windows::Win32::System::Com::*;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::Shell::PropertiesSystem::{IPropertyStore, PROPERTYKEY};

const CLSID_MMDEVICE_ENUMERATOR: GUID = GUID::from_u128(0xBCDE0395_E52F_467C_8E3D_C4579291692E);
const PKEY_DEVICE_FRIENDLY_NAME: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0),
    pid: 14,
};

fn default_device() -> (String, IMMDevice) {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&CLSID_MMDEVICE_ENUMERATOR, None, CLSCTX_ALL).expect("enum");
        let dev = enumerator
            .GetDefaultAudioEndpoint(eRender, eConsole)
            .expect("dev");
        let store = dev.OpenPropertyStore(STGM_READ).expect("store");
        let v = store.GetValue(&PKEY_DEVICE_FRIENDLY_NAME).expect("val");
        let name = windows::core::BSTR::try_from(&v).expect("bstr").to_string();
        (name, dev)
    }
}

fn fmt_24in32(rate: u32, ch: u16) -> WAVEFORMATEXTENSIBLE {
    WAVEFORMATEXTENSIBLE {
        Format: WAVEFORMATEX {
            wFormatTag: 0xFFFE,
            nChannels: ch,
            nSamplesPerSec: rate,
            nAvgBytesPerSec: rate * ch as u32 * 4,
            nBlockAlign: ch * 4,
            wBitsPerSample: 32,
            cbSize: 22,
        },
        Samples: WAVEFORMATEXTENSIBLE_0 {
            wValidBitsPerSample: 24,
        },
        dwChannelMask: 0x3,
        SubFormat: KSDATAFORMAT_SUBTYPE_PCM,
    }
}

unsafe fn run_config(
    dev: &IMMDevice,
    label: &str,
    sharemode: AUDCLNT_SHAREMODE,
    flags: u32,
    buf_hns: i64,
    period_hns: i64,
    fmt: *const WAVEFORMATEX,
) {
    println!("\n=== {label} ===");
    let client: IAudioClient = dev.Activate(CLSCTX_ALL, None).expect("activate");
    let init = client.Initialize(sharemode, flags, buf_hns, period_hns, fmt, None);
    if init.is_err() {
        println!("  Initialize FAILED: 0x{:08X}", init.unwrap_err().code().0);
        return;
    }
    let buf_frames = client.GetBufferSize().expect("bufsize");
    println!("  Initialize OK, buffer={buf_frames} frames");

    let render: IAudioRenderClient = client.GetService().expect("render");
    let ch = (std::ptr::read_unaligned(std::ptr::addr_of!((*fmt).nChannels))) as usize;

    let buf_ptr = render.GetBuffer(buf_frames).expect("GetBuffer");
    let i32s = std::slice::from_raw_parts_mut(buf_ptr as *mut i32, buf_frames as usize * ch);
    for (i, dst) in i32s.iter_mut().enumerate() {
        let f = (i / ch) as f64;
        let s = 0.25 * (2.0 * std::f64::consts::PI * 440.0 * f / 48000.0).sin();
        *dst = ((s * 8_388_607.0) as i32) << 8;
    }
    render.ReleaseBuffer(buf_frames, 0).expect("ReleaseBuffer");
    println!("  primed {buf_frames} frames");

    let event = if flags & AUDCLNT_STREAMFLAGS_EVENTCALLBACK != 0 {
        let h = CreateEventW(None, false, false, None).expect("event");
        client.SetEventHandle(h).expect("SetEventHandle");
        Some(h)
    } else {
        None
    };

    client.Start().expect("Start");
    let mut paddings = Vec::new();
    for i in 0..10 {
        if let Some(h) = event {
            let _ = WaitForSingleObject(h, 300);
        } else {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let p = client.GetCurrentPadding().expect("padding");
        paddings.push(p);
        print!("    t={:2} p={p}", i * 50);
        if i > 0 {
            let d = paddings[i - 1] as i64 - p as i64;
            if d != 0 {
                print!(" ({d:+})");
            }
        }
        println!();
    }
    let _ = client.Stop();
    let _ = client.Reset();

    let moving = paddings.windows(2).any(|w| w[0] != w[1]);
    println!(
        "  => padding {}",
        if moving {
            "MOVES (consuming)"
        } else {
            "STUCK (not consuming)"
        }
    );
}

fn def_period_value(dev: &IMMDevice) -> i64 {
    unsafe {
        let client: IAudioClient = dev.Activate(CLSCTX_ALL, None).expect("activate");
        let mut def_period = 0i64;
        let mut min_period = 0i64;
        let _ = client.GetDevicePeriod(Some(&mut def_period), Some(&mut min_period));
        def_period
    }
}

fn main() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let (name, dev) = default_device();
    println!("device: {name:?}");

    unsafe {
        let probe_client: IAudioClient = dev.Activate(CLSCTX_ALL, None).expect("activate");
        let mut def_period = 0i64;
        let mut min_period = 0i64;
        let _ = probe_client.GetDevicePeriod(Some(&mut def_period), Some(&mut min_period));
        println!(
            "device period: {:.1}ms (min {:.1}ms)",
            def_period as f64 / 10_000.0,
            min_period as f64 / 10_000.0
        );
        let mix = probe_client.GetMixFormat().expect("mix");
        let mixref: *const WAVEFORMATEX = &*mix;
        let mr = std::ptr::read_unaligned(std::ptr::addr_of!((*mixref).nSamplesPerSec));
        let mch = std::ptr::read_unaligned(std::ptr::addr_of!((*mixref).nChannels));
        let mbits = std::ptr::read_unaligned(std::ptr::addr_of!((*mixref).wBitsPerSample));
        println!("mix: {mr}Hz {mch}ch {mbits}bits");

        // 1. Shared control (mix format) — validates the probe itself.
        run_config(
            &dev,
            "CONTROL: shared mode, mix format",
            AUDCLNT_SHAREMODE_SHARED,
            0,
            2_000_000,
            0,
            mix,
        );

        // 2. Exclusive 24-in-32, event 1s/1s (current production config)
        let f1 = Box::leak(Box::new(fmt_24in32(48000, 2)));
        run_config(
            &dev,
            "EXCL 24in32: event 1s/1s (current prod)",
            AUDCLNT_SHAREMODE_EXCLUSIVE,
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
            10_000_000,
            10_000_000,
            f1 as *mut WAVEFORMATEXTENSIBLE as *const WAVEFORMATEX,
        );

        // 3. Exclusive 24-in-32, event 200ms buf / device period
        let f2 = Box::leak(Box::new(fmt_24in32(48000, 2)));
        run_config(
            &dev,
            "EXCL 24in32: event 200ms buf / device period",
            AUDCLNT_SHAREMODE_EXCLUSIVE,
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
            2_000_000,
            def_period_value(&dev),
            f2 as *mut WAVEFORMATEXTENSIBLE as *const WAVEFORMATEX,
        );

        // 4. Exclusive 24-in-32, polling (no event), 200ms
        let f3 = Box::leak(Box::new(fmt_24in32(48000, 2)));
        run_config(
            &dev,
            "EXCL 24in32: polling 200ms buf",
            AUDCLNT_SHAREMODE_EXCLUSIVE,
            0,
            2_000_000,
            0,
            f3 as *mut WAVEFORMATEXTENSIBLE as *const WAVEFORMATEX,
        );

        // 5. Exclusive float (mix format) @48k, event 200ms/device period
        let mix_client: IAudioClient = dev.Activate(CLSCTX_ALL, None).expect("activate");
        let mixfmt = mix_client.GetMixFormat().expect("mix");
        let f4: *const WAVEFORMATEX = mixfmt;
        run_config(
            &dev,
            "EXCL float(mix fmt): event 200ms / device period",
            AUDCLNT_SHAREMODE_EXCLUSIVE,
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
            2_000_000,
            def_period_value(&dev),
            f4,
        );
    }
}
