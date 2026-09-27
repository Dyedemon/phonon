//! End-to-end verification of the production polling exclusive stream.
//! NOT shipped — diagnostics only.

use phonon_core::device::DeviceManager;
use phonon_core::wasapi::WasapiExclusiveStream;
use std::f64::consts::PI;

fn main() {
    println!("== production exclusive stream verification (polling) ==");
    let dm = DeviceManager::new().expect("DeviceManager");
    let devices = dm.list_devices().expect("list");
    let name = devices
        .iter()
        .find(|d| d.is_default)
        .or_else(|| devices.first())
        .expect("no device")
        .name
        .clone();
    println!("device: {name:?}");

    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_MULTITHREADED,
        );
    }

    let rate: u32 = 48000;
    let ch: u16 = 2;
    let stream = match WasapiExclusiveStream::new(&name, rate, ch, false, false) {
        Ok(s) => {
            println!(
                "created: negotiated, buffer={} frames",
                s.buffer_size_frames()
            );
            s
        }
        Err(e) => {
            println!("CREATE FAILED: {e}");
            return;
        }
    };

    // Prime with a 440 Hz sine (direct GetBuffer).
    let frames = stream.buffer_size_frames() as usize;
    let prime: Vec<f32> = (0..frames * ch as usize)
        .map(|i| {
            let f = (i / ch as usize) as f64;
            (0.25 * (2.0 * PI * 440.0 * f / rate as f64).sin()) as f32
        })
        .collect();
    if let Err(e) = stream.prime(&prime) {
        println!("PRIME FAILED: {e}");
        return;
    }
    println!("primed full buffer");

    if let Err(e) = stream.start() {
        println!("START FAILED: {e}");
        return;
    }
    println!("started; polling for 2 seconds (should hear a 440 Hz tone):");

    // Feed continuously for 2 s using poll_available + write_pending.
    let mut written = 0usize;
    let start = std::time::Instant::now();
    while start.elapsed().as_millis() < 2000 {
        let avail = stream.poll_available().unwrap_or(0) as usize;
        if avail == 0 {
            std::thread::sleep(std::time::Duration::from_millis(2));
            continue;
        }
        let chunk: Vec<f32> = (0..avail * ch as usize)
            .map(|i| {
                let f = ((written + i) / ch as usize) as f64;
                (0.25 * (2.0 * PI * 440.0 * f / rate as f64).sin()) as f32
            })
            .collect();
        let w = stream.write_pending(&chunk).unwrap_or(0);
        written += w as usize;
        std::thread::sleep(std::time::Duration::from_millis(4));
    }
    let _ = stream.stop();
    println!(
        "wrote {written} samples in 2 s (expected ~{}/{})",
        2 * rate as usize * ch as usize,
        rate as usize * ch as usize
    );
    println!(
        "verdict: {}",
        if written > rate as usize {
            "device consumed audio — production path WORKS"
        } else {
            "device consumed nothing — still broken"
        }
    );
}
