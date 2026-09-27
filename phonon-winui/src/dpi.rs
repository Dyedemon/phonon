use std::sync::atomic::{AtomicI32, Ordering};

use windows::Win32::Graphics::Gdi::{GetDC, GetDeviceCaps, ReleaseDC, LOGPIXELSY};

static DPI: AtomicI32 = AtomicI32::new(0);

/// Returns the current monitor DPI (cached after first call).
pub fn get_dpi() -> i32 {
    let v = DPI.load(Ordering::Relaxed);
    if v != 0 {
        return v;
    }
    let v = unsafe {
        let hdc = GetDC(None);
        let d = GetDeviceCaps(hdc, LOGPIXELSY);
        let _ = ReleaseDC(None, hdc);
        if d <= 0 {
            96
        } else {
            d
        }
    };
    DPI.store(v, Ordering::Relaxed);
    v
}

/// Scale a logical pixel value (at 96 DPI) to the current DPI.
#[inline]
pub fn s(v: i32) -> i32 {
    v * get_dpi() / 96
}

/// Initialize DPI awareness.  Must be called before any window creation.
pub fn init_dpi_awareness() {
    unsafe {
        let _ = windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
            windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        );
    }
}
