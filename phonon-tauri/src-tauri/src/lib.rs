//! Phonon Tauri - Desktop application backend.
//!
//! Provides the Tauri command layer and event system
//! for the Phonon audio framework.

pub mod commands;
pub mod events;
pub mod state;

#[cfg(target_os = "windows")]
use commands::PlaybackStateInfo;
use state::AppState;
use std::sync::Mutex;
use std::time::Duration;
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::Emitter;
use tauri::Listener;
use tauri::Manager;
use tauri::WindowEvent;

#[cfg(target_os = "windows")]
static TASKBAR_HWND: Mutex<Option<windows::Win32::Foundation::HWND>> = Mutex::new(None);

#[cfg(target_os = "windows")]
static TRAY_APP_HANDLE: Mutex<Option<tauri::AppHandle>> = Mutex::new(None);

#[cfg(target_os = "windows")]
static TASKBAR_INITIALIZED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Tracks the current play/pause icon handle so we can destroy the previous
/// one when replacing it. The taskbar does NOT copy the icon — the handle
/// must stay alive as long as the button uses it.
#[cfg(target_os = "windows")]
static TASKBAR_PLAY_ICON: Mutex<Option<windows::Win32::UI::WindowsAndMessaging::HICON>> =
    Mutex::new(None);

/// Original window procedure saved before we replaced it with `phonon_wndproc`.
/// We must call `CallWindowProcW` with this for every message so Tauri's
/// internal message handling (and the subclass chain) keeps working.
///
/// Uses `AtomicIsize` instead of `Mutex` because `CallWindowProcW` synchronously
/// dispatches messages, which re-enters `phonon_wndproc` on the same thread —
/// a `Mutex` would deadlock instantly. Atomic load/store has no lock held during
/// the call, so re-entry is safe.
#[cfg(target_os = "windows")]
static ORIGINAL_WNDPROC: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

/// Custom WM_USER message to trigger a taskbar play icon update from the main thread.
/// wParam: 1 = playing, 0 = paused
#[cfg(target_os = "windows")]
const WM_PHONON_UPDATE_TASKBAR: u32 = 0x0400 + 100; // WM_USER + 100

/// Custom WM_USER message to verify the subclass is installed and receiving messages.
#[cfg(target_os = "windows")]
const WM_PHONON_TASKBAR_PING: u32 = 0x0400 + 101; // WM_USER + 101

#[cfg(target_os = "windows")]
pub fn update_taskbar_play_icon(playing: bool) {
    use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
    if let Ok(Some(hwnd)) = TASKBAR_HWND.lock().map(|h| *h) {
        unsafe {
            let _ = PostMessageW(
                hwnd,
                WM_PHONON_UPDATE_TASKBAR,
                windows::Win32::Foundation::WPARAM(if playing { 1 } else { 0 }),
                windows::Win32::Foundation::LPARAM(0),
            );
        }
    }
}

#[cfg(target_os = "windows")]
unsafe fn do_update_taskbar_icon(hwnd: windows::Win32::Foundation::HWND, playing: bool) {
    use windows::core::GUID;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Shell::{
        ITaskbarList3, THUMBBUTTON, THUMBBUTTONFLAGS, THUMBBUTTONMASK,
    };

    let initialized = TASKBAR_INITIALIZED.load(std::sync::atomic::Ordering::Relaxed);
    if !initialized || hwnd.0 == 0 {
        return;
    }

    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        if let Ok(taskbar_list) = CoCreateInstance::<_, ITaskbarList3>(
            &GUID::from_u128(0x56FDF344_FD6D_11d0_958A_006097C9A090),
            None,
            CLSCTX_ALL,
        ) {
            let _ = taskbar_list.HrInit();

            let icon = if playing {
                create_media_icon(1) // Pause
            } else {
                create_media_icon(0) // Play
            };

            let mut buttons: [THUMBBUTTON; 1] = std::mem::zeroed();
            buttons[0].iId = 101;
            buttons[0].dwMask = THUMBBUTTONMASK(0x2 | 0x8); // THB_ICON | THB_FLAGS
            buttons[0].dwFlags = THUMBBUTTONFLAGS(0); // THBF_ENABLED
            buttons[0].hIcon = icon;

            let _ = taskbar_list.ThumbBarUpdateButtons(hwnd, &buttons);

            // The taskbar does NOT copy the icon — the handle must stay alive
            // as long as the button uses it. Destroy the PREVIOUS icon (if any)
            // and store the new one for later cleanup.
            if let Ok(mut guard) = TASKBAR_PLAY_ICON.lock() {
                if let Some(old_icon) = guard.take() {
                    use windows::Win32::UI::WindowsAndMessaging::DestroyIcon;
                    let _ = DestroyIcon(old_icon);
                }
                *guard = Some(icon);
            }
        }
    }
}

/// Legacy subclass proc — kept as a fallback in case SetWindowLongPtrW
/// approach needs to be reverted. Currently unused; phonon_wndproc is
/// installed instead via SetWindowLongPtrW for more reliable message routing.
#[cfg(target_os = "windows")]
#[allow(dead_code)]
unsafe extern "system" fn thumb_subclass_proc(
    hwnd: windows::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
    _uidsubclass: usize,
    _dwrefdata: usize,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::UI::Shell::DefSubclassProc;
    use windows::Win32::UI::WindowsAndMessaging::WM_COMMAND;
    if msg == WM_COMMAND {
        let hiword = ((wparam.0 >> 16) & 0xFFFF) as u32;
        let id = (wparam.0 & 0xFFFF) as u32;
        // THBN_CLICKED: accept both 0x1800 (Windows SDK shobjidl.h) and 0x8000
        // (observed on some Windows builds) for maximum compatibility.
        log::info!("[taskbar] WM_COMMAND: hiword=0x{:X}, id={}", hiword, id);
        if hiword == 0x1800 || hiword == 0x8000 {
            log::info!("[taskbar] THBN_CLICKED (0x{:X}): id={}", hiword, id);
            let handle_clone = TRAY_APP_HANDLE.lock().ok().and_then(|g| g.clone());
            if let Some(handle) = handle_clone {
                let event_name = match id {
                    100 => Some("tb-prev"),
                    101 => Some("tb-play-pause"),
                    102 => Some("tb-next"),
                    _ => {
                        log::warn!("[taskbar] Unknown button id: {}", id);
                        None
                    }
                };
                if let Some(name) = event_name {
                    match handle.emit(name, ()) {
                        Ok(_) => log::info!("[taskbar] Emitted event: {}", name),
                        Err(e) => log::error!("[taskbar] Failed to emit event {}: {}", name, e),
                    }
                }
            } else {
                log::error!("[taskbar] TRAY_APP_HANDLE is None or locked when button clicked");
            }
        }
    } else if msg == WM_PHONON_UPDATE_TASKBAR {
        let playing = wparam.0 != 0;
        do_update_taskbar_icon(hwnd, playing);
        return windows::Win32::Foundation::LRESULT(0);
    } else if msg == WM_PHONON_TASKBAR_PING {
        // Ping message used to verify subclass is installed and receiving messages
        log::info!("[taskbar] Subclass ping received — subclass is active");
        return windows::Win32::Foundation::LRESULT(0x4F4B); // "OK"
    }
    DefSubclassProc(hwnd, msg, wparam, lparam)
}

/// Executes the playback action triggered by a thumbnail toolbar button.
///
/// Called from a dedicated `phonon-tb-task` worker thread so engine file I/O
/// and decoder setup never block the UI/WndProc thread.
#[cfg(target_os = "windows")]
fn run_taskbar_button_action(handle: &tauri::AppHandle, _id: u32, action: &str) {
    // Get AppState from the handle — same way Tauri commands do it.
    // Log whether state() actually resolves: if AppState was never
    // registered by manage() during setup, state() would panic —
    // catch_unwind so a panic here doesn't crash the worker thread on
    // every button click.
    //
    // Declare a typed named-struct outside the closure to avoid
    // catch_unwind's fallible type inference from collapsing the
    // outer Result wrapper over the (result, snapshot) tuple.
    #[allow(non_camel_case_types)]
    struct taskbar__EngOutcome {
        r: Result<(), String>,
        s: Option<(String, String)>,
    }
    let boxed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || -> taskbar__EngOutcome {
            let state: tauri::State<AppState> = handle.state::<AppState>();
            use phonon_core::engine::PlaybackState;
            let engine_state_before = format!("{:?}", state.engine.state());
            let queue_len = state.engine.queue_len();
            let inner: Result<(), String> = match action {
                "previous" => {
                    log::info!(
                        "[taskbar-wndproc] engine.previous() — queue_len={} state_before={}",
                        queue_len,
                        engine_state_before
                    );
                    state.engine.previous();
                    // previous() auto-starts playback when paused/stopped (mirrors
                    // next()), so the taskbar icon must flip to "pause".
                    crate::update_taskbar_play_icon(true);
                    let after = format!("{:?}", state.engine.state());
                    log::info!(
                        "[taskbar-wndproc] engine.previous() DONE — state_after={}",
                        after
                    );
                    Ok(())
                }
                "next" => {
                    log::info!(
                        "[taskbar-wndproc] engine.next() — queue_len={} state_before={}",
                        queue_len,
                        engine_state_before
                    );
                    state.engine.next();
                    // next() auto-starts playback when paused/stopped — flip icon.
                    crate::update_taskbar_play_icon(true);
                    let after = format!("{:?}", state.engine.state());
                    log::info!(
                        "[taskbar-wndproc] engine.next() DONE — state_after={}",
                        after
                    );
                    Ok(())
                }
                "toggle_play_pause" => {
                    log::info!(
                        "[taskbar-wndproc] toggle: state_before={}, queue_len={}",
                        engine_state_before,
                        queue_len
                    );
                    match state.engine.state() {
                        PlaybackState::Playing => {
                            state.engine.pause();
                            crate::update_taskbar_play_icon(false);
                            log::info!("[taskbar-wndproc] toggle → PAUSED ✓");
                        }
                        PlaybackState::Paused => {
                            state.engine.resume();
                            crate::update_taskbar_play_icon(true);
                            log::info!("[taskbar-wndproc] toggle → RESUMED ✓");
                        }
                        PlaybackState::Idle | PlaybackState::Stopped => {
                            if queue_len == 0 {
                                // Queue is empty — can't start playback.
                                // Notify frontend so it can open a file
                                // dialog or show "no tracks" message.
                                // This is NOT an error, just nothing to play.
                                log::info!(
                                "[taskbar-wndproc] toggle → queue empty, emitting taskbar-play-empty-queue"
                            );
                                let _ = handle.emit("taskbar-play-empty-queue", ());
                            } else {
                                if let Err(e) = state.engine.start_playback() {
                                    log::error!(
                                        "[taskbar-wndproc] toggle → start_playback FAILED: {}",
                                        e
                                    );
                                    let after = format!("{:?}", state.engine.state());
                                    return taskbar__EngOutcome {
                                        r: Err(format!("start_playback: {}", e)),
                                        s: Some((engine_state_before, after)),
                                    };
                                }
                                crate::update_taskbar_play_icon(true);
                                log::info!("[taskbar-wndproc] toggle → STARTED from Idle ✓");
                            }
                        }
                    }
                    Ok(())
                }
                _ => Ok(()),
            };
            let after = format!("{:?}", state.engine.state());
            taskbar__EngOutcome {
                r: inner,
                s: Some((engine_state_before, after)),
            }
        },
    ));
    let (exec_result, state_snapshot) = match boxed {
        Ok(taskbar__EngOutcome { r, s }) => (r, s),
        Err(panic) => {
            let msg = if let Some(s) = panic.downcast_ref::<&str>() {
                s.to_string()
            } else if let Some(s) = panic.downcast_ref::<String>() {
                s.clone()
            } else {
                String::from("<Boxed panic payload>")
            };
            log::error!("[taskbar-wndproc] PANIC during engine call: {}", msg);
            (Err(format!("panic: {}", msg)), None)
        }
    };

    match &exec_result {
        Ok(()) => log::info!(
            "[taskbar-wndproc] Action {} SUCCESS — state transition: {:?}",
            action,
            state_snapshot
        ),
        Err(e) => log::error!(
            "[taskbar-wndproc] Action {} FAILED: {} — state snapshot: {:?}",
            action,
            e,
            state_snapshot
        ),
    }

    // Also emit the original event as a convenience, in case other
    // front-end code (play state UI, etc.) relies on it — playback
    // is already handled above, so emit failures won't break
    // buttons.
    let event_name = match action {
        "previous" => "tb-prev",
        "toggle_play_pause" => "tb-play-pause",
        "next" => "tb-next",
        _ => "",
    };
    if !event_name.is_empty() {
        let _ = handle.emit(event_name, ());
    }

    // ── Optimistic UI update ────────────────────────────────────────────
    // Push current playback snapshot to the frontend *immediately* after
    // the action completes (milliseconds), instead of waiting for the
    // 3s polling loop. This eliminates the laggy feel when switching
    // songs: "music is already playing but the title still shows the
    // previous song" the user reported. This snapshot is authoritative
    // because it comes directly from the engine (which already updated
    // current_track as part of previous/next()).
    if exec_result.is_ok() {
        if let Ok(snapshot_state) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let s: tauri::State<AppState> = handle.state::<AppState>();
            PlaybackStateInfo {
                state: format!("{:?}", s.engine.state()),
                position_secs: s.engine.position(),
                duration_secs: s.engine.duration(),
                buffer_fill: s.engine.buffer_fill(),
                current_track: s.engine.current_track(),
                queue_length: s.engine.queue_len(),
                speed: s.engine.get_speed(),
            }
        })) {
            // Suppress snapshot emit while frontend is reloading (HMR/F5).
            // The taskbar WndProc runs on the UI thread independently of
            // the audio engine — even after engine.stop(), Windows may
            // still deliver one final THBN_CLICKED message.
            let reloading = handle
                .state::<AppState>()
                .frontend_reloading
                .load(std::sync::atomic::Ordering::SeqCst);
            if !reloading {
                let _ = handle.emit("playback-state-snapshot", &snapshot_state);
            }
            log::debug!(
                "[taskbar-wndproc] Pushed optimistic snapshot: state={} track={:?}",
                snapshot_state.state,
                snapshot_state.current_track
            );
        }
    }
}

/// Top-level window procedure installed via `SetWindowLongPtrW(GWLP_WNDPROC)`.
///
/// This is more reliable than `SetWindowSubclass` because we are guaranteed
/// to receive every message *before* Tauri's internal handlers. If Tauri's
/// own subclass chain ever swallows `WM_COMMAND`, the subclass-based
/// `thumb_subclass_proc` would never see it — this WndProc always does.
///
/// All unhandled messages are forwarded to the original WndProc via
/// `CallWindowProcW`, preserving Tauri's full functionality.
#[cfg(target_os = "windows")]
unsafe extern "system" fn phonon_wndproc(
    hwnd: windows::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::UI::WindowsAndMessaging::{
        CallWindowProcW, DefWindowProcW, WM_ACTIVATE, WM_COMMAND, WM_SIZE,
    };

    // First-N general message sample: proves this WndProc is receiving traffic
    // at all without flooding info level. In normal usage the guard timer and
    // WM_COMMAND logs are enough; only crank RUST_LOG=debug if you need this.
    let is_interesting = msg == WM_COMMAND || msg == WM_SIZE || msg == WM_ACTIVATE;
    if !is_interesting {
        let sample = WNDPROC_MSG_SAMPLE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if sample < 60 {
            log::debug!(
                "[taskbar-wndproc] Message #{:02}: msg=0x{:04X} wparam=0x{:X} lparam=0x{:X} hwnd={:?}",
                sample, msg, wparam.0, lparam.0, hwnd
            );
        }
    }

    // Confirm that the calling HWND matches the HWND we installed on. In
    // Tauri's multi-HWND architecture the procedure sometimes gets invoked
    // on a sibling window — the buttons' WM_COMMAND is sent only to the
    // owner root HWND we saved in TASKBAR_HWND, so a mismatch here (for
    // WM_COMMAND specifically) is a strong signal we installed on the
    // wrong handle.
    if let Ok(Some(expected_hwnd)) = TASKBAR_HWND.lock().map(|g| *g) {
        if expected_hwnd != hwnd {
            // Normal for non-WM_COMMAND traffic from sibling Tauri windows.
            // Only log this during early sample window so logs stay readable.
            let sample = WNDPROC_MSG_SAMPLE.load(std::sync::atomic::Ordering::Relaxed);
            if sample < 60 {
                log::trace!(
                    "[taskbar-wndproc] HWND mismatch: called on {:?}, TASKBAR_HWND={:?}, msg=0x{:04X}",
                    hwnd, expected_hwnd, msg
                );
            }
        }
    }

    if msg == WM_COMMAND {
        let hiword = ((wparam.0 >> 16) & 0xFFFF) as u32;
        let id = (wparam.0 & 0xFFFF) as u32;
        log::info!(
            "[taskbar-wndproc] WM_COMMAND: hiword=0x{:X}, id={}",
            hiword,
            id
        );
        // Match by button ID only — the THBN_CLICKED notification code is
        // officially 0x1800 but some Windows builds send 0x8000 or other values.
        // Since IDs 100/101/102 are exclusively used by our thumbnail buttons
        // (Tauri's own command IDs are in a different range), matching by ID
        // alone is safe and more robust than filtering by hiword.
        let action = match id {
            100 => Some("previous"),
            101 => Some("toggle_play_pause"),
            102 => Some("next"),
            _ => None,
        };
        if let Some(action) = action {
            log::info!(
                "[taskbar-wndproc] Button {} clicked (hiword=0x{:X}, hwnd={:?}), executing {}",
                id,
                hiword,
                hwnd,
                action
            );

            // ── Debounce: drop duplicate same-button clicks within 80 ms ──
            // The thumbnail toolbar is known to occasionally double-post
            // WM_COMMAND on some Windows builds / third-party shell hooks.
            // Combined with the old "frontend re-invokes Rust" bug (now fixed)
            // a single user click could toggle twice: Playing → Paused → Playing
            // which looked like "buttons do nothing".
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0);
            let mut debounce_skip = false;
            if let Ok(mut guard) = TASKBAR_LAST_SEEN.lock() {
                if let Some((last_id, last_ms)) = *guard {
                    if last_id == id && now_ms.saturating_sub(last_ms) < TASKBAR_DEBOUNCE_MS {
                        debounce_skip = true;
                        log::warn!(
                            "[taskbar-wndproc] Debounce DROP: id={} fired twice within {}ms (gap={}ms)",
                            id, TASKBAR_DEBOUNCE_MS, now_ms.saturating_sub(last_ms)
                        );
                    }
                }
                if !debounce_skip {
                    *guard = Some((id, now_ms));
                }
            }
            if debounce_skip {
                let proc_addr = ORIGINAL_WNDPROC.load(std::sync::atomic::Ordering::Acquire);
                if proc_addr != 0 {
                    let orig_proc: windows::Win32::UI::WindowsAndMessaging::WNDPROC =
                        std::mem::transmute(proc_addr);
                    return CallWindowProcW(orig_proc, hwnd, msg, wparam, lparam);
                }
                return windows::Win32::Foundation::LRESULT(0);
            }

            // Execute playback commands DIRECTLY on the engine via AppState.
            // This bypasses the emit→listen IPC round-trip which was losing
            // events: WM_COMMAND fires inside phonon_wndproc on the UI thread,
            // but emit broadcasts through Tauri's event system and may get
            // dropped if the WebView event loop is busy or listeners haven't
            // registered yet. Calling engine methods from Rust is 100% reliable
            // and instant — no IPC, no event loss.
            let handle_clone = TRAY_APP_HANDLE.lock().ok().and_then(|g| g.clone());
            if let Some(handle) = handle_clone {
                // ── Run heavy work OFF the UI thread ─────────────────────────
                // Engine calls can do file I/O (start_playback reads cues,
                // opens decoders) and briefly locks the audio device. Running
                // them synchronously on the UI thread freezes window painting
                // for ~100-500 ms; we dispatch the work onto a dedicated OS
                // thread (WndProc is a raw native callback — we cannot use
                // Tauri's async runtime from within it because there is no
                // tokio Runtime context available here).
                //
                // `handle` and `action_owned` are both cheap to clone:
                // AppHandle is ref-counted internally, and emits queue their
                // payload over a channel so they're also thread-safe.
                let action_owned = action.to_string();
                let handle_for_thread = handle.clone();
                std::thread::spawn(move || {
                    run_taskbar_button_action(&handle_for_thread, id, &action_owned);
                });
            }
        }
    } else if msg == WM_PHONON_UPDATE_TASKBAR {
        let playing = wparam.0 != 0;
        do_update_taskbar_icon(hwnd, playing);
        return windows::Win32::Foundation::LRESULT(0);
    } else if msg == WM_PHONON_TASKBAR_PING {
        log::info!("[taskbar-wndproc] Ping received — WndProc is active");
        return windows::Win32::Foundation::LRESULT(0x4F4B); // "OK"
    }

    // Forward to the original WndProc (Tauri's handler + any subclass chain).
    // Atomic load — no lock held during CallWindowProcW, so synchronous message
    // re-entry (SendMessage inside the original proc) cannot deadlock.
    let proc_addr = ORIGINAL_WNDPROC.load(std::sync::atomic::Ordering::Acquire);
    if proc_addr != 0 {
        let orig_proc: windows::Win32::UI::WindowsAndMessaging::WNDPROC =
            std::mem::transmute(proc_addr);
        return CallWindowProcW(orig_proc, hwnd, msg, wparam, lparam);
    }
    // Fallback if original WndProc was never saved.
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

#[cfg(target_os = "windows")]
unsafe fn create_media_icon(kind: i32) -> windows::Win32::UI::WindowsAndMessaging::HICON {
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, SelectObject, BITMAPINFO,
        BITMAPINFOHEADER, DIB_RGB_COLORS, HBITMAP, HDC,
    };
    use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, ICONINFO};

    const SIZE: i32 = 24;

    unsafe fn create_empty_bm(size: i32) -> (HBITMAP, HDC, *mut u8) {
        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = size;
        bmi.bmiHeader.biHeight = -size; // top-down
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = 0; // BI_RGB

        let hdc = CreateCompatibleDC(None);
        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let hbm =
            CreateDIBSection(hdc, &bmi, DIB_RGB_COLORS, &mut bits, None, 0).unwrap_or_default();
        SelectObject(hdc, hbm);
        // Zero out pixels (fully transparent)
        let pixel_count = (size * size) as usize;
        let pixels = bits as *mut u32;
        for i in 0..pixel_count {
            *pixels.add(i) = 0x00000000; // ARGB: fully transparent
        }
        (hbm, hdc, bits as *mut u8)
    }

    unsafe fn set_pixel(bits: *mut u8, x: i32, y: i32, size: i32, color: u32) {
        if x < 0 || y < 0 || x >= size || y >= size {
            return;
        }
        let offset = (y * size + x) as isize * 4;
        *(bits.offset(offset) as *mut u32) = color;
    }

    unsafe fn fill_triangle(
        bits: *mut u8,
        size: i32,
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        color: u32,
    ) {
        // Simple triangle rasterization using bounding box scanline
        let min_x = x0.min(x1).min(x2);
        let max_x = x0.max(x1).max(x2);
        let min_y = y0.min(y1).min(y2);
        let max_y = y0.max(y1).max(y2);

        let edge = |px: i32, py: i32, ex0: i32, ey0: i32, ex1: i32, ey1: i32| -> bool {
            (px - ex0) * (ey1 - ey0) - (py - ey0) * (ex1 - ex0) >= 0
        };

        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let e1 = edge(x, y, x0, y0, x1, y1);
                let e2 = edge(x, y, x1, y1, x2, y2);
                let e3 = edge(x, y, x2, y2, x0, y0);
                if e1 && e2 && e3 {
                    set_pixel(bits, x, y, size, color);
                }
            }
        }
    }

    unsafe fn fill_rect(bits: *mut u8, size: i32, rx: i32, ry: i32, rw: i32, rh: i32, color: u32) {
        for y in ry..ry + rh {
            for x in rx..rx + rw {
                set_pixel(bits, x, y, size, color);
            }
        }
    }

    let (hbm_color, hdc, bits) = unsafe { create_empty_bm(SIZE) };
    // white color, fully opaque: ARGB = 0xFFFFFFFF
    let color: u32 = 0xFFFFFFFF;

    unsafe {
        match kind {
            // Play: right-pointing triangle (counter-clockwise vertex order)
            0 => {
                let cx = SIZE / 2;
                let cy = SIZE / 2;
                let h = 7;
                let w = 8;
                fill_triangle(
                    bits,
                    SIZE,
                    cx - w / 2,
                    cy - h,
                    cx - w / 2,
                    cy + h,
                    cx + w / 2,
                    cy,
                    color,
                );
            }
            // Pause: two vertical bars
            1 => {
                let cx = SIZE / 2;
                let cy = SIZE / 2;
                let bar_w = 4;
                let bar_h = 12;
                let gap = 4;
                fill_rect(
                    bits,
                    SIZE,
                    cx - gap / 2 - bar_w,
                    cy - bar_h / 2,
                    bar_w,
                    bar_h,
                    color,
                );
                fill_rect(
                    bits,
                    SIZE,
                    cx + gap / 2,
                    cy - bar_h / 2,
                    bar_w,
                    bar_h,
                    color,
                );
            }
            // Previous: left arrow + vertical bar
            2 => {
                let cx = SIZE / 2;
                let cy = SIZE / 2;
                // bar
                fill_rect(bits, SIZE, cx - 7, cy - 6, 2, 12, color);
                // triangle pointing left (counter-clockwise in screen coords)
                let tx = cx - 3;
                let h = 6;
                let w = 7;
                fill_triangle(
                    bits,
                    SIZE,
                    tx + w / 2,
                    cy - h,
                    tx - w / 2,
                    cy,
                    tx + w / 2,
                    cy + h,
                    color,
                );
            }
            // Next: right arrow + vertical bar
            3 => {
                let cx = SIZE / 2;
                let cy = SIZE / 2;
                // bar
                fill_rect(bits, SIZE, cx + 5, cy - 6, 2, 12, color);
                // triangle pointing right (counter-clockwise)
                let tx = cx + 1;
                let h = 6;
                let w = 7;
                fill_triangle(
                    bits,
                    SIZE,
                    tx - w / 2,
                    cy - h,
                    tx - w / 2,
                    cy + h,
                    tx + w / 2,
                    cy,
                    color,
                );
            }
            _ => {}
        }
    }

    // Create mask bitmap - for 32bpp icons with alpha, mask is still required
    // but can be monochrome; we use the color bitmap's alpha channel
    let (hbm_mask, hdc_mask, _) = unsafe { create_empty_bm(SIZE) };

    let mut icon_info: ICONINFO = unsafe { std::mem::zeroed() };
    icon_info.fIcon = true.into();
    icon_info.hbmColor = hbm_color;
    icon_info.hbmMask = hbm_mask;

    let hicon = unsafe { CreateIconIndirect(&icon_info).unwrap_or_default() };

    // Cleanup
    unsafe {
        DeleteObject(hbm_color);
        DeleteObject(hbm_mask);
        DeleteDC(hdc);
        DeleteDC(hdc_mask);
    }

    hicon
}

#[cfg(target_os = "windows")]
const TASKBAR_TIMER_ID: usize = 42424;

/// Guard timer that periodically verifies our WndProc is still installed.
/// Tauri (or its WebView controller) may re-subclass the top-level window
/// after we call SetWindowLongPtrW, silently replacing our procedure. This
/// causes button clicks to appear visually but never reach phonon_wndproc.
/// The guard runs every 5 s during the first 30 s of the window lifetime,
/// then every 60 s afterwards.
#[cfg(target_os = "windows")]
const TASKBAR_GUARD_TIMER_ID: usize = 42425;

/// How many guard checks to perform at the high-frequency 5 s cadence.
#[cfg(target_os = "windows")]
static TASKBAR_GUARD_CHECKS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Virtual address of phonon_wndproc. We compare this against
/// GetWindowLongPtrW(GWLP_WNDPROC) in the guard timer to detect when Tauri
/// or another component has overwritten our WndProc pointer.
#[cfg(target_os = "windows")]
static PHONON_WNDPROC_ADDR: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

/// Sampling counter for general (non-WM_COMMAND) messages. We log the first
/// N messages so the user can verify phonon_wndproc is receiving *any*
/// traffic at all, without spamming the log on every paint/mouse event.
#[cfg(target_os = "windows")]
static WNDPROC_MSG_SAMPLE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

#[cfg(target_os = "windows")]
static TASKBAR_RETRY_COUNT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Debounce guard for thumbnail toolbar WM_COMMAND messages: last (button_id,
/// timestamp_ms) processed. If we see the same ID again within
/// TASKBAR_DEBOUNCE_MS we skip it — defends against Windows accidentally
/// double-posting THBN_CLICKED on some builds. 30 ms is tight enough to not
/// affect real rapid clicks (human double-click gap is ~100 ms) but still
/// catches the <5 ms duplicates the OS sometimes injects.
#[cfg(target_os = "windows")]
static TASKBAR_LAST_SEEN: std::sync::Mutex<Option<(u32, u128)>> = std::sync::Mutex::new(None);
#[cfg(target_os = "windows")]
const TASKBAR_DEBOUNCE_MS: u128 = 30;

#[cfg(target_os = "windows")]
unsafe extern "system" fn taskbar_timer_proc(
    hwnd: windows::Win32::Foundation::HWND,
    _msg: u32,
    _id: usize,
    _time: u32,
) {
    use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};
    // One-shot: kill the timer immediately
    let _ = KillTimer(hwnd, TASKBAR_TIMER_ID);
    // Now we're on the main thread — safe to call SetWindowSubclass
    log::info!(
        "[taskbar] Timer fired, initializing thumbnail toolbar for hwnd {:?}",
        hwnd
    );
    match init_taskbar_thumbnail(hwnd) {
        Ok(_) => log::info!("[taskbar] Thumbnail toolbar initialized successfully"),
        Err(e) => {
            let retry = TASKBAR_RETRY_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if retry < 5 {
                log::warn!(
                    "[taskbar] Init failed ({}), retrying in 500ms: {}",
                    retry + 1,
                    e
                );
                let _ = SetTimer(hwnd, TASKBAR_TIMER_ID, 500, Some(taskbar_timer_proc));
            } else {
                log::error!("[taskbar] Init failed after 5 retries, giving up: {}", e);
            }
        }
    }
}

/// Guard timer callback — re-installs phonon_wndproc if Tauri or another
/// component overwrote GWLP_WNDPROC after our initial SetWindowLongPtrW.
/// This is the #1 most likely reason the user reports "buttons can be clicked
/// visually but never trigger playback". See PHONON_WNDPROC_ADDR for the
/// expected pointer value.
#[cfg(target_os = "windows")]
unsafe extern "system" fn taskbar_guard_timer_proc(
    hwnd: windows::Win32::Foundation::HWND,
    _msg: u32,
    _id: usize,
    _time: u32,
) {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, KillTimer, SetTimer, SetWindowLongPtrW, GWLP_WNDPROC,
    };

    if !TASKBAR_INITIALIZED.load(std::sync::atomic::Ordering::Relaxed) {
        // Thumbbar not yet initialized — keep checking every 1 s.
        let _ = SetTimer(
            hwnd,
            TASKBAR_GUARD_TIMER_ID,
            1000,
            Some(taskbar_guard_timer_proc),
        );
        return;
    }

    let expected = PHONON_WNDPROC_ADDR.load(std::sync::atomic::Ordering::Acquire);
    if expected == 0 {
        return;
    }
    let actual = GetWindowLongPtrW(hwnd, GWLP_WNDPROC);
    if actual == expected {
        // WndProc still ours — all good. Transition to 60 s cadence after
        // the first 6 checks (30 s) to avoid long-term overhead.
        let checks = TASKBAR_GUARD_CHECKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let interval = if checks < 6 { 5000u32 } else { 60_000u32 };
        let _ = SetTimer(
            hwnd,
            TASKBAR_GUARD_TIMER_ID,
            interval,
            Some(taskbar_guard_timer_proc),
        );
        log::trace!("[taskbar-guard] WndProc OK (checks={})", checks + 1);
        return;
    }

    // MISMATCH — someone overwrote our WndProc. Save the NEW original and
    // re-install phonon_wndproc on top. This recovers the message handler
    // chain without losing any earlier (Tauri) messages.
    log::warn!(
        "[taskbar-guard] WndProc OVERWRITTEN: expected=0x{:X}, actual=0x{:X}. Re-installing phonon_wndproc now.",
        expected, actual
    );
    ORIGINAL_WNDPROC.store(actual, std::sync::atomic::Ordering::Release);
    windows::Win32::Foundation::SetLastError(windows::Win32::Foundation::WIN32_ERROR(0));
    let result = SetWindowLongPtrW(hwnd, GWLP_WNDPROC, expected);
    if result == 0 {
        let err = windows::Win32::Foundation::GetLastError().0;
        if err != 0 {
            log::error!("[taskbar-guard] Re-install FAILED: GetLastError={}", err);
            // One last try in 2 s.
            let _ = SetTimer(
                hwnd,
                TASKBAR_GUARD_TIMER_ID,
                2000,
                Some(taskbar_guard_timer_proc),
            );
            return;
        }
    }
    log::info!(
        "[taskbar-guard] WndProc RE-INSTALLED successfully. Old original was 0x{:X}",
        actual
    );
    // Re-verify by reading back
    let verify = GetWindowLongPtrW(hwnd, GWLP_WNDPROC);
    if verify == expected {
        log::info!("[taskbar-guard] Post-fix verification PASSED");
    } else {
        log::error!(
            "[taskbar-guard] Post-fix verification FAILED: got 0x{:X}",
            verify
        );
    }
    // Keep guarding
    let checks = TASKBAR_GUARD_CHECKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let interval = if checks < 6 { 5000u32 } else { 60_000u32 };
    let _ = SetTimer(
        hwnd,
        TASKBAR_GUARD_TIMER_ID,
        interval,
        Some(taskbar_guard_timer_proc),
    );
    let _ = KillTimer(hwnd, _id);
}

#[cfg(target_os = "windows")]
unsafe fn init_taskbar_thumbnail(
    hwnd: windows::Win32::Foundation::HWND,
) -> Result<(), Box<dyn std::error::Error>> {
    use windows::core::GUID;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Shell::{
        ITaskbarList3, THUMBBUTTON, THUMBBUTTONFLAGS, THUMBBUTTONMASK,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetAncestor, IsWindowVisible, GA_ROOTOWNER};

    // Prevent double initialization — ThumbBarAddButtons can only be called once per window
    if TASKBAR_INITIALIZED.load(std::sync::atomic::Ordering::Relaxed) {
        log::info!("[taskbar] Already initialized, skipping");
        return Ok(());
    }

    // ── Critical: use the TOP-LEVEL ROOT window, not a child/control HWND ─────
    // Tauri's window.hwnd() on Windows may return a child HWND of the WebView
    // controller or a sub-window in the Tauri internal HWND tree. The taskbar
    // thumbnail toolbar (ITaskbarList3) MUST be attached to the actual top-level
    // window that owns the taskbar button. If we call ThumbBarAddButtons on a
    // child HWND, the buttons APPEAR visually (because COM accepts the call on
    // behalf of the root) — BUT the THBN_CLICKED WM_COMMAND notifications are
    // sent to the ROOT window, not the child. This exactly matches the user's
    // symptom: "buttons can be clicked visually (UI feedback) but no action
    // happens". GetAncestor(GA_ROOTOWNER) walks up to the real top-level window.
    let root_hwnd = GetAncestor(hwnd, GA_ROOTOWNER);
    let effective_hwnd = if root_hwnd.0 != 0 { root_hwnd } else { hwnd };
    log::info!(
        "[taskbar] Input HWND={:?}, effective ROOT HWND={:?} (same={})",
        hwnd,
        effective_hwnd,
        effective_hwnd == hwnd
    );
    let hwnd = effective_hwnd;

    // Verify the window is visible — ThumbBarAddButtons requires the window to have a taskbar button
    if !IsWindowVisible(hwnd).as_bool() {
        log::warn!("[taskbar] Window not yet visible, deferring initialization");
        return Err(Box::new(std::io::Error::new(
            std::io::ErrorKind::Other,
            "Window not visible",
        )));
    }

    // ── Diagnostic: dump root HWND identity. Users sometimes report "other
    // apps work but Phonon doesn't": verifying the class name and extended
    // style confirms we're operating on the real top-level owner window that
    // owns the taskbar button — not a popup / WebView child / control HWND.
    {
        use std::ffi::OsString;
        use std::os::windows::ffi::OsStringExt;
        use windows::Win32::UI::WindowsAndMessaging::{
            GetClassNameW, GetWindowLongW, GetWindowTextLengthW, GetWindowTextW, GWL_EXSTYLE,
            GWL_STYLE, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
        };
        let mut buf: [u16; 256] = [0u16; 256];
        let len = GetClassNameW(hwnd, &mut buf);
        let class_name = if len > 0 {
            let slice = &buf[..len as usize];
            OsString::from_wide(slice).to_string_lossy().into_owned()
        } else {
            String::from("<unknown>")
        };
        let title_len = GetWindowTextLengthW(hwnd);
        let title = if title_len > 0 {
            let mut tbuf = vec![0u16; (title_len + 1) as usize];
            let got = GetWindowTextW(hwnd, &mut tbuf);
            if got > 0 {
                OsString::from_wide(&tbuf[..got as usize])
                    .to_string_lossy()
                    .into_owned()
            } else {
                String::from("")
            }
        } else {
            String::from("")
        };
        let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
        let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        let has_appwindow = (ex_style & WS_EX_APPWINDOW.0 as u32) != 0;
        let has_toolwindow = (ex_style & WS_EX_TOOLWINDOW.0 as u32) != 0;
        log::info!(
            "[taskbar] Root HWND identity: class=\"{}\", title=\"{}\", style=0x{:X}, ex_style=0x{:X}, WS_EX_APPWINDOW={}, WS_EX_TOOLWINDOW={}",
            class_name, title, style, ex_style, has_appwindow, has_toolwindow
        );
        if has_toolwindow && !has_appwindow {
            log::warn!(
                "[taskbar] Root HWND has WS_EX_TOOLWINDOW without WS_EX_APPWINDOW — Windows may hide this window from the taskbar, ThumbBarAddButtons can silently fail."
            );
        }
    }

    // COM must be initialized on this thread before creating COM objects.
    let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

    log::info!("[taskbar] Creating ITaskbarList3 instance...");
    let taskbar_list: ITaskbarList3 = CoCreateInstance(
        &GUID::from_u128(0x56FDF344_FD6D_11d0_958A_006097C9A090),
        None,
        CLSCTX_ALL,
    )?;

    taskbar_list.HrInit()?;
    log::info!("[taskbar] ITaskbarList3 initialized");

    // Create custom minimal-style media icons
    let prev_hicon = create_media_icon(2); // Previous
    let play_hicon = create_media_icon(0); // Play
    let next_hicon = create_media_icon(3); // Next

    let mut buttons: [THUMBBUTTON; 3] = std::mem::zeroed();

    // Helper to write a tooltip string into the szTip array.
    fn set_tip(tip: &mut [u16; 260], text: &str) {
        for (i, c) in text.encode_utf16().take(259).enumerate() {
            tip[i] = c;
        }
    }

    // Previous button
    buttons[0].iId = 100;
    buttons[0].dwMask = THUMBBUTTONMASK(0x2 | 0x4 | 0x8); // THB_ICON | THB_TOOLTIP | THB_FLAGS
    buttons[0].dwFlags = THUMBBUTTONFLAGS(0x00); // THBF_ENABLED
    buttons[0].hIcon = prev_hicon;
    set_tip(&mut buttons[0].szTip, "上一首");

    // Play/Pause button
    buttons[1].iId = 101;
    buttons[1].dwMask = THUMBBUTTONMASK(0x2 | 0x4 | 0x8); // THB_ICON | THB_TOOLTIP | THB_FLAGS
    buttons[1].dwFlags = THUMBBUTTONFLAGS(0x00); // THBF_ENABLED
    buttons[1].hIcon = play_hicon;
    set_tip(&mut buttons[1].szTip, "播放 / 暂停");

    // Next button
    buttons[2].iId = 102;
    buttons[2].dwMask = THUMBBUTTONMASK(0x2 | 0x4 | 0x8); // THB_ICON | THB_TOOLTIP | THB_FLAGS
    buttons[2].dwFlags = THUMBBUTTONFLAGS(0x00); // THBF_ENABLED
    buttons[2].hIcon = next_hicon;
    set_tip(&mut buttons[2].szTip, "下一首");

    log::info!("[taskbar] Calling ThumbBarAddButtons for hwnd {:?}", hwnd);
    if let Err(e) = taskbar_list.ThumbBarAddButtons(hwnd, &buttons) {
        log::error!("[taskbar] ThumbBarAddButtons failed: {}", e);
        return Err(Box::new(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("ThumbBarAddButtons failed: {}", e),
        )));
    }
    log::info!("[taskbar] ThumbBarAddButtons succeeded");

    // Install our WndProc via SetWindowLongPtrW. This is more reliable than
    // SetWindowSubclass because we receive every message *before* Tauri's
    // internal handlers — if Tauri's subclass chain ever swallows WM_COMMAND,
    // the subclass-based approach would silently lose button clicks.
    log::info!("[taskbar] Installing WndProc via SetWindowLongPtrW...");
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SendMessageW, SetTimer, SetWindowLongPtrW, GWLP_WNDPROC,
    };
    let our_proc_addr = phonon_wndproc as *const () as usize as isize;
    PHONON_WNDPROC_ADDR.store(our_proc_addr, std::sync::atomic::Ordering::Release);
    unsafe {
        windows::Win32::Foundation::SetLastError(windows::Win32::Foundation::WIN32_ERROR(0));
    }
    let original_proc = SetWindowLongPtrW(hwnd, GWLP_WNDPROC, our_proc_addr);
    if original_proc == 0 {
        let err = unsafe { windows::Win32::Foundation::GetLastError() };
        // SetWindowLongPtrW returns 0 on failure, but 0 is also a valid previous
        // value — check GetLastError to distinguish.
        if err.0 != 0 {
            log::error!("[taskbar] SetWindowLongPtrW failed: GetLastError={}", err.0);
            return Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("SetWindowLongPtrW failed: {}", err.0),
            )));
        }
    }
    // ── Critical verification: read GWLP_WNDPROC back and ensure it matches
    // our expected address. Tauri may immediately chain another subclass on
    // top of us during setup, which would overwrite our pointer.
    let verify_proc = GetWindowLongPtrW(hwnd, GWLP_WNDPROC);
    if verify_proc == our_proc_addr {
        log::info!(
            "[taskbar] SetWindowLongPtrW verify OK: installed at 0x{:X}, original was 0x{:X}",
            our_proc_addr,
            original_proc
        );
    } else {
        log::warn!(
            "[taskbar] SetWindowLongPtrW verify MISMATCH: expected=0x{:X}, installed_but_read_back=0x{:X}. Tauri already chained another subclass on top — guard timer will re-assert.",
            our_proc_addr, verify_proc
        );
    }
    // Save the original WndProc so we can forward messages to it.
    // Atomic store — see ORIGINAL_WNDPROC comment for why this must be lock-free.
    ORIGINAL_WNDPROC.store(original_proc, std::sync::atomic::Ordering::Release);
    log::info!("[taskbar] Original WndProc saved at 0x{:X}", original_proc);

    // Verify our WndProc is receiving messages by sending a ping.
    let ping_result = SendMessageW(
        hwnd,
        WM_PHONON_TASKBAR_PING,
        windows::Win32::Foundation::WPARAM(0),
        windows::Win32::Foundation::LPARAM(0),
    );
    if ping_result.0 == 0x4F4B {
        log::info!("[taskbar] WndProc ping OK — WndProc is active");
    } else {
        log::warn!("[taskbar] WndProc ping returned 0x{:X} — WndProc may not be active (Tauri may have wrapped it already)", ping_result.0);
    }

    TASKBAR_INITIALIZED.store(true, std::sync::atomic::Ordering::Relaxed);
    if let Ok(mut guard) = TASKBAR_HWND.lock() {
        *guard = Some(hwnd);
    }
    // Track the initial play icon so do_update_taskbar_icon can destroy it
    // when replacing it with a pause icon (or vice versa).
    if let Ok(mut guard) = TASKBAR_PLAY_ICON.lock() {
        *guard = Some(play_hicon);
    }

    // ── Start the WndProc guard timer. This re-installs phonon_wndproc if
    // Tauri / WebView ever overwrites GWLP_WNDPROC after this point. This is
    // the single most important fix for "buttons appear but never do anything"
    // because it recovers from late subclassing. The guard fires every 5 s
    // for the first 30 s, then every 60 s.
    let _ = unsafe {
        SetTimer(
            hwnd,
            TASKBAR_GUARD_TIMER_ID,
            5000,
            Some(taskbar_guard_timer_proc),
        )
    };
    log::info!("[taskbar] Guard timer (5 s → 60 s) started on root HWND");

    Ok(())
}

static CLI_ARGS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Store CLI args for later use during setup.
pub fn set_cli_args(args: Vec<String>) {
    CLI_ARGS.lock().unwrap().extend(args);
}

fn take_cli_args() -> Vec<String> {
    CLI_ARGS.lock().unwrap().drain(..).collect()
}

/// Persist the main window's geometry (position/size + monitor scale factor).
/// Ignored when maximized/minimized (keeps the un-maximized restore bounds;
/// -32000-style minimized coordinates are rejected). Called on window close,
/// hide-to-tray and app quit so the restored geometry never goes stale —
/// previously only the window-X path saved it, making tray-quit sessions
/// restore stale bounds on next launch.
pub(crate) fn save_main_window_geometry(
    app: &tauri::AppHandle,
) -> Option<crate::state::AppSettings> {
    let win = app.get_webview_window("main")?;
    let is_maximized = win.is_maximized().unwrap_or(false);
    let is_minimized = win.is_minimized().unwrap_or(false);
    let scale = win
        .current_monitor()
        .ok()
        .flatten()
        .map(|m| m.scale_factor())
        .unwrap_or(1.0);
    let state = app.state::<AppState>();
    let snapshot = {
        let mut s = state.settings.lock().unwrap();
        if !is_maximized && !is_minimized {
            if let (Ok(pos), Ok(size)) = (win.outer_position(), win.inner_size()) {
                if pos.x > -1000 && pos.y > -1000 && size.width >= 400 && size.height >= 300 {
                    s.main_window_x = pos.x as f64;
                    s.main_window_y = pos.y as f64;
                    s.main_window_width = size.width as f64;
                    s.main_window_height = size.height as f64;
                    s.main_window_position_set = true;
                    s.main_window_scale = scale;
                }
            }
        }
        s.main_window_maximized = is_maximized;
        s.clone()
    };
    if let Err(e) = commands::save_settings_sync(&snapshot) {
        log::error!("[window geometry] save_settings_sync failed: {}", e);
    }
    Some(snapshot)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // --console: attach to the launching terminal (or allocate a new one)
    // so logs print live even in release builds, whose windows_subsystem
    // would otherwise have no console at all. The log plugin adds a Stdout
    // target for this case below.
    #[cfg(windows)]
    if std::env::args().any(|a| a == "--console") {
        use windows::Win32::System::Console::{AllocConsole, AttachConsole, ATTACH_PARENT_PROCESS};
        unsafe {
            if AttachConsole(ATTACH_PARENT_PROCESS).is_err() {
                let _ = AllocConsole();
            }
        }
    }

    // tauri_plugin_log 会设置全局 logger，不能再用 env_logger（否则 panic: "attempted to set a logger after the logging system was already initialized"）
    let app_state = AppState::new().expect("Failed to initialize AppState");

    // NOTE: load_settings_sync() is now called in .setup() where app_data_dir()
    // is available, ensuring correct paths in packaged builds.

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_log::Builder::default().level(log::LevelFilter::Trace)
            // --console: 附着/分配一个控制台窗口，日志实时打印到终端
            // （release 版默认 windows_subsystem 无控制台）。未传时日志
            // 仍写文件 + webview 控制台。
            .targets({
                use tauri_plugin_log::{Target, TargetKind};
                if std::env::args().any(|a| a == "--console") {
                    vec![
                        Target::new(TargetKind::Stdout),
                        Target::new(TargetKind::LogDir { file_name: None }),
                        Target::new(TargetKind::Webview),
                    ]
                } else {
                    vec![
                        Target::new(TargetKind::LogDir { file_name: None }),
                        Target::new(TargetKind::Webview),
                    ]
                }
            })
            // WASM 编译栈（wasmtime/cranelift）的内部 TRACE 日志必须屏蔽：
            // cranelift 在寄存器分配前格式化指令时会命中虚拟寄存器的
            // unreachable!()（cranelift-codegen external.rs，上游 issue
            // bytecodealliance/wasmtime#10529），panic 会杀死编译线程，
            // 导致 `tauri:dev` 加载 plugins/*.wasm 时直接崩溃（exit 101）。
            // 这些逐指令 TRACE 也是插件编译动辄数十秒的主因。
            .filter(|meta| {
                let t = meta.target();
                !t.starts_with("cranelift") && !t.starts_with("wasmtime") && !t.starts_with("regalloc2")
            })
            .build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // Second instance launched — focus the existing main window instead.
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.show();
                let _ = win.unminimize();
                let _ = win.set_focus();
            }
        }))
        // Suppress "[TAURI] Couldn't find callback id" warnings on page reload.
        // Runs BEFORE core.js and any queued webview.eval calls, which is critical
        // because Rust's pending invoke responses arrive as queued eval and would
        // otherwise fire before any on-page-load handler could patch things.
        // Two layers: (1) filter console.warn for the exact Tauri message,
        // (2) patch Map.prototype.get so Tauri's callbacks Map returns a noop
        // for missing ids (prevents the warn from firing at its source).
        .plugin(
            tauri::plugin::Builder::<tauri::Wry>::new("phonon-callback-guard")
                .js_init_script(
                    r#"
(function(){
  if (!console.warn.__phononPatched) {
    var origWarn = console.warn.bind(console);
    var patched = function(){
      try {
        if (arguments.length > 0 && typeof arguments[0] === 'string' &&
            arguments[0].indexOf("[TAURI] Couldn't find callback id") !== -1) return;
      } catch(e) {}
      return origWarn.apply(console, arguments);
    };
    patched.__phononOrig = origWarn;
    patched.__phononPatched = true;
    console.warn = patched;
  }
  if (!Map.prototype.get.__phononPatched) {
    var realMapGet = Map.prototype.get;
    var noop = function(){};
    Map.prototype.get = function(key){
      var result = realMapGet.call(this, key);
      if (result !== undefined) return result;
      try {
        var it = window.__TAURI_INTERNALS__;
        if (it && it.callbacks === this) return noop;
      } catch(e) {}
      return undefined;
    };
    Map.prototype.get.__phononPatched = true;
  }
})();
"#,
                )
                .build(),
        )
        .manage(app_state)
        // Page-load hook: stop playback and suppress events on navigation start,
        // re-enable event emission when the new page is ready. The JS-side
        // warning filter is already in place via the plugin's init script.
        // Only applies to the MAIN window — auxiliary windows (desktop-lyrics,
        // tray-popup) must not interfere with playback state.
        .on_page_load(|app, payload| {
            let url_path = payload.url().path();
            let is_aux_window = url_path.ends_with("desktop-lyrics.html")
                || url_path.ends_with("tray-popup.html");
            if is_aux_window {
                return;
            }
            match payload.event() {
                tauri::webview::PageLoadEvent::Started => {
                    let state = app.state::<AppState>();
                    state
                        .frontend_reloading
                        .store(true, std::sync::atomic::Ordering::SeqCst);
                    state.engine.stop();
                    #[cfg(target_os = "windows")]
                    crate::update_taskbar_play_icon(false);
                    log::info!("[page-load] Frontend reloading — events suppressed, playback stopped");
                }
                tauri::webview::PageLoadEvent::Finished => {
                    let state = app.state::<AppState>();
                    state
                        .frontend_reloading
                        .store(false, std::sync::atomic::Ordering::SeqCst);
                    log::info!("[page-load] Frontend ready — events resumed");
                    if let Some(win) = app.get_webview_window("main") {
                        let maximized = state
                            .settings
                            .lock()
                            .map(|s| s.main_window_maximized)
                            .unwrap_or(false);
                        if maximized {
                            let w_clone = win.clone();
                            tauri::async_runtime::spawn(async move {
                                tokio::time::sleep(std::time::Duration::from_millis(120)).await;
                                let _ = w_clone.maximize();
                            });
                        }
                    }
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            // Playback
            commands::play,
            commands::toggle_play_pause,
            commands::pause,
            commands::stop,
            commands::prepare_for_reload,
            commands::next,
            commands::previous,
            commands::seek,
            commands::set_volume,
            commands::get_volume_mode,
            commands::get_platform,
            commands::get_build_info,
            commands::get_volume_level,
            commands::sync_volume_from_device,
            commands::get_audio_diagnostics,
            commands::get_playback_state,
            commands::set_playback_speed,
            commands::get_playback_speed,
            commands::set_playback_mode,
            commands::get_playback_mode,
            // Device
            commands::list_devices,
            commands::set_device,
            commands::get_current_device,
            // Queue
            commands::add_to_queue,
            commands::add_cue_to_queue,
            commands::remove_from_queue,
            commands::get_queue,
            commands::clear_queue,
            commands::reorder_queue,
            commands::play_index,
            // Playlist
            commands::save_playlist,
            commands::load_playlist,
            commands::create_playlist,
            commands::delete_playlist,
            commands::switch_playlist,
            commands::switch_system_playlist,
            commands::list_system_playlists,
            commands::list_playlists,
            commands::get_active_playlist,
            commands::rename_playlist,
            commands::reorder_playlists,
            commands::save_current_to_playlist,
            commands::reset_to_defaults,
            commands::clean_user_data,
            // DSP
            commands::list_dsp_processors,
            commands::enable_dsp,
            commands::disable_dsp,
            commands::reorder_dsp,
            commands::get_dsp_latency,
            commands::add_wasm_dsp,
            commands::remove_wasm_dsp,
            commands::get_smart_effect_mode,
            commands::set_smart_effect_mode,
            commands::get_surround_sound_mode,
            commands::set_surround_sound_mode,
            // Plugin
            commands::list_plugins,
            commands::enable_plugin,
            commands::disable_plugin,
            commands::scan_plugins,
            commands::remove_plugin,
            commands::reload_plugin,
            commands::get_plugin_config,
            commands::set_plugin_config,
            commands::call_plugin_command,
            commands::toggle_plugin_in_dsp,
            commands::load_time_stretch_plugin,
            commands::unload_time_stretch_plugin,
            commands::import_plugin,
            commands::get_plugin_dir,
            commands::open_plugin_dir,
            // Settings
            commands::get_settings,
            commands::take_settings_corrupt_notice,
            commands::update_settings,
            commands::set_replay_gain,
            commands::get_replay_gain,
            commands::set_volume_mode,
            commands::get_exclusive_mode,
            commands::set_exclusive_mode,
            commands::set_buffer_size,
            commands::get_buffer_size,
            commands::set_resampler_quality,
            commands::get_resampler_quality,
            commands::set_theme,
            commands::get_theme,
            commands::set_debug_log,
            commands::get_debug_log,
            commands::open_log_dir,
            commands::get_lyrics_api_url,
            commands::set_lyrics_api_url,
            commands::set_startup_behavior,
            commands::get_startup_behavior,
            commands::save_session,
            commands::load_session,
            commands::set_spectrum_settings,
            commands::get_spectrum_settings,
            // EQ
            commands::get_eq_state,
            commands::set_eq_band,
            commands::set_eq_mode,
            commands::load_eq_preset,
            commands::save_eq_preset,
            commands::delete_eq_preset,
            commands::add_eq_band,
            commands::remove_eq_band,
            commands::set_geq_band_count,
            commands::set_eq_preamp,
            // Hotplug Notifications
            commands::get_hotplug_notifications,
            commands::set_hotplug_notifications,
            commands::get_close_to_tray,
            commands::set_close_to_tray,
            // ReplayGain Pre-amp
            commands::get_replay_gain_preamp,
            commands::set_replay_gain_preamp,
            // Spectrum Color Scheme
            commands::get_spectrum_colorscheme,
            commands::set_spectrum_colorscheme,
            // Plugin Limits
            commands::get_plugin_limits,
            commands::set_plugin_limits,
            // Language
            commands::get_language,
            commands::set_language,
            // Bit Depth / Sample Rate
            commands::set_bit_depth,
            commands::get_bit_depth,
            commands::set_output_sample_rate,
            commands::get_output_sample_rate,
            // DSD Mode
            commands::set_dsd_mode,
            commands::get_dsd_mode,
            // Events
            commands::start_hotplug_monitor,
            // DevTools — synthetic hotplug event injector (dev-only; release rejects)
            commands::debug_inject_hotplug_event,
            // Metadata
            commands::get_track_metadata,
            commands::scan_folder,
            commands::get_album_art,
            commands::get_lyrics,
            commands::load_lrc_file,
            commands::search_lyrics,
            commands::search_lyrics_by_keyword,
            commands::fetch_lyrics_by_id,
            commands::parse_lrc_text,
            commands::scan_lyric_sources,
            commands::read_lyric_script,
            commands::fetch_url,
            commands::scan_vis_sources,
            commands::read_vis_script,
            commands::play_url,
            commands::open_desktop_lyrics,
            commands::close_desktop_lyrics,
            commands::get_desktop_lyrics_settings,
            commands::set_desktop_lyrics_settings,
            commands::save_desktop_lyrics_position,
            commands::resolve_desktop_lyrics_bg_path,
            commands::save_desktop_lyrics_bg,
            commands::reset_desktop_lyrics_settings,
            commands::save_main_window_position,
            // Window title (taskbar thumbnail header)
            commands::set_window_title,
            // Media Library
            commands::library_set_roots,
            commands::library_get_roots,
            commands::library_scan,
            commands::library_search,
            commands::library_get_tracks,
            commands::library_get_track,
            commands::library_get_track_by_path,
            commands::library_toggle_favorite,
            commands::library_set_rating,
            commands::library_record_play,
            commands::library_edit_metadata,
            commands::library_set_track_cover,
            commands::library_cleanup_deleted,
            commands::library_batch_scan_replaygain,
            commands::library_list_albums,
            commands::library_list_artists,
            commands::library_list_genres,
            commands::library_get_thumbnail,
            // Plan A Task A5: 6 new library_* commands (partial/bytes variants
            // coexist with the existing per-field library_edit_metadata +
            // library_set_track_cover commands — frontend imports under
            // the suffixed names).
            commands::library_get_tracks_by_paths,
            commands::library_search_suggest,
            commands::library_edit_metadata_partial,
            commands::library_set_track_cover_bytes,
            commands::library_try_recover_corrupt,
            commands::library_write_metadata_back_to_file,
            // Folder Sync (per-playlist auto-sync)
            commands::sync_folder,
            commands::unsync_folder,
            commands::get_synced_folders,
            commands::sync_folder_for_playlist,
            // App
            commands::quit_app,
            commands::set_ignore_cursor_events,
            // PEQ (Parametric EQ) — general-purpose biquad EQ control
            commands::set_peq_bands,
            commands::get_peq_state,
            commands::set_peq_bypass,
        ])
        .setup(|app| {
            let handle = app.handle().clone();

            // Stamp the running build into every log — identifies the exact
            // commit a user report (or an installed copy) came from.
            log::info!(
                "Phonon {} build={} sha={}",
                app.package_info().version,
                env!("CARGO_PKG_VERSION"),
                env!("PHONON_BUILD_SHA")
            );

            // ── Set global data dir (app_data_dir) ─────────────────
            // Must happen before any settings/session/library path resolution
            // so packaged builds write to the correct user data directory.
            if let Ok(data_dir) = app.path().app_data_dir() {
                commands::set_data_dir(data_dir.clone());

                // ── Switch plugin runtime to user data dir ─────────
                // In packaged builds, plugins live under %APPDATA%\Phonon\plugins
                // so users can drop .wasm files there without admin rights.
                let plugins_dir = data_dir.join("plugins");
                let state = app.state::<AppState>();
                // Save the original project plugin dir before switching.
                // Only read in dev builds — release builds switch straight
                // to the data dir, so the binding only exists there.
                #[cfg(debug_assertions)]
                let original_dir = state.plugin_runtime.plugin_dir();
                if let Err(e) = state.plugin_runtime.set_plugin_dir(&plugins_dir) {
                    log::warn!("Failed to set plugin dir to {:?}: {}", plugins_dir, e);
                } else {
                    log::info!("Plugin directory set to: {:?}", plugins_dir);
                }
                // In dev mode, also scan the project's plugins/ folder so
                // plugins built alongside the source tree are auto-discovered.
                #[cfg(debug_assertions)]
                {
                    if original_dir.exists() && original_dir != plugins_dir {
                        let dir_str = original_dir.display().to_string();
                        state.plugin_runtime.add_scan_dir(original_dir);
                        log::info!("Added dev plugin scan dir: {}", dir_str);
                    }
                }

                // Re-scan now that the final directory layout is in place.
                // set_plugin_dir above already rescans, but the dev extra
                // scan dir is added afterwards — without this final pass the
                // cached plugin list stays empty until the user presses 扫描
                // (the 2s watcher never fires because an empty data dir has
                // no changes to report).
                if let Err(e) = state.plugin_runtime.scan() {
                    log::warn!("Post-setup plugin scan failed: {}", e);
                } else {
                    let _ = handle.emit("plugins-changed", "setup");
                }
            }

            // ── Load settings BEFORE session ───────────────────────
            // settings.json takes precedence over AppState::new() defaults;
            // load_session later does an additional legacy merge if needed.
            {
                let state = app.state::<AppState>();
                match commands::load_settings_sync() {
                    Ok(Some(mut loaded)) => {
                        // Apply persisted audio settings to the engine before storing
                        state.engine.set_bit_depth(loaded.bit_depth);
                        state.engine.set_output_sample_rate(loaded.output_sample_rate);
                        state.engine.set_dsd_mode(commands::dsd_mode_from_u8(loaded.dsd_mode));
                        // Exclusive mode is NEVER restored from settings: it
                        // always starts OFF and must be re-enabled manually.
                        // Rationale: a session that died while holding the
                        // device (or a DAC that rejects the exclusive format)
                        // would otherwise come back muted on every launch.
                        // Overwrite the persisted value so settings.json
                        // matches reality from the first run.
                        if loaded.exclusive_mode {
                            log::info!(
                                "[startup] exclusive_mode was persisted ON — forcing OFF (exclusive always starts disabled)"
                            );
                            loaded.exclusive_mode = false;
                            let _ = commands::save_settings_sync(&loaded);
                        }
                        state.engine.set_exclusive(false);
                        state.device_manager.lock().unwrap().set_exclusive_mode(false);
                        // Restore last selected output device
                        if let Some(ref dev_id) = loaded.hotplug.last_output_device_id {
                            let mut dm = state.device_manager.lock().unwrap();
                            if let Err(e) = dm.set_default_device(dev_id) {
                                log::warn!("[startup] Failed to restore output device '{}': {}", dev_id, e);
                            } else {
                                log::info!("[startup] Restored output device: {}", dev_id);
                            }
                        }
                        *state.settings.lock().unwrap() = loaded;
                        // Restore EQ state from saved settings
                        commands::restore_eq_state(&state);
                    }
                    Ok(None) => {
                        // No settings file yet — fresh install, defaults are fine.
                    }
                    Err(e) => {
                        // Settings file is corrupt — back it up, fall back to defaults, notify user.
                        log::error!("settings.json corrupt: {}", e);
                        if let Ok(path) = commands::settings_path() {
                            let bak = path.with_extension("json.corrupt");
                            let _ = std::fs::rename(&path, &bak);
                        }
                        // Notify the frontend (best-effort; may arrive before listeners are registered)
                        let _ = app.emit("phonon:toast", serde_json::json!({
                            "msg": format!("设置文件已损坏，已恢复为默认设置。备份：settings.json.corrupt"),
                            "level": "warn"
                        }));
                        // Also store in state so frontend can pick it up on init
                        state.settings_corrupt_notice
                            .lock()
                            .unwrap()
                            .replace(format!("设置文件已损坏，已恢复为默认设置。\n错误：{}", e));
                    }
                }
            }

            // ── Load session BEFORE window setup ───────────────────
            // This ensures window positions are restored from session.json
            {
                let state = app.state::<AppState>();
                let _ = commands::load_session(state);
            }

            // ── Restore folder watchers from session ──────────────
            // load_session restores the synced_folders map but can't
            // create watchers (no AppHandle). We do it here.
            {
                let bindings: std::collections::HashMap<String, String> = app
                    .state::<AppState>()
                    .synced_folders
                    .lock()
                    .unwrap()
                    .clone();
                for (playlist, folder) in &bindings {
                    if let Err(e) = commands::sync_folder(
                        app.handle().clone(),
                        app.state::<AppState>(),
                        playlist.clone(),
                        folder.clone(),
                    ) {
                        log::warn!(
                            "[folder-sync] Failed to restore watcher for '{}': {}",
                            folder,
                            e
                        );
                    }
                }
            }

            // ── Set window icon (dev mode) ──────────────────────
            let dev_icon_path =
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("icons/icon.png");
            let app_icon = if dev_icon_path.exists() {
                tauri::image::Image::from_path(&dev_icon_path).ok()
            } else {
                None
            };
            if let (Some(window), Some(ref icon)) = (app.get_webview_window("main"), &app_icon) {
                let _ = window.set_icon(icon.clone());
            }

            // ── Close → Minimize to Tray / Save position ────────────
            if let Some(window) = app.get_webview_window("main") {
                // Set up Windows taskbar thumbnail toolbar.
                // Defer the call slightly so the window is fully initialized
                // and has a taskbar button before we add thumbbar buttons.
                // Calling ThumbBarAddButtons too early (during setup) can
                // silently fail on Windows.
                #[cfg(target_os = "windows")]
                {
                    if let Ok(mut guard) = TRAY_APP_HANDLE.lock() {
                        *guard = Some(handle.clone());
                    }
                    let hwnd_result = window.hwnd();
                    if let Ok(hwnd) = hwnd_result {
                        // Use SetTimer to schedule initialization on the main thread.
                        // SetWindowSubclass must be called from the thread that owns
                        // the window, and the setup callback runs on the main thread.
                        // The timer fires after 300ms on the main thread's message loop.
                        // Convert to our windows-0.54 HWND (Tauri may use a different version).
                        let hwnd_ptr = hwnd.0 as isize;
                        unsafe {
                            use windows::Win32::UI::WindowsAndMessaging::SetTimer;
                            let hwnd_054 = windows::Win32::Foundation::HWND(hwnd_ptr);
                            let _ =
                                SetTimer(hwnd_054, TASKBAR_TIMER_ID, 300, Some(taskbar_timer_proc));
                        }
                    }
                }
                let w = window.clone();
                let app_handle = handle.clone();
                // Restore saved position/size on startup (window is hidden, so no
                // visible flash). Maximize is deferred until page load finishes so
                // the user never sees a blank webview surface.
                {
                    let state = app_handle.state::<AppState>();
                    let s = state.settings.lock().unwrap();
                    let pos_set = s.main_window_position_set;
                    if pos_set {
                        use tauri::PhysicalPosition;
                        use tauri::PhysicalSize;
                        let saved_x = s.main_window_x as i32;
                        let saved_y = s.main_window_y as i32;
                        let saved_w = s.main_window_width as u32;
                        let saved_h = s.main_window_height as u32;

                        // Clamp position so at least 100px of the title bar
                        // stays on the nearest monitor. Prevents the window
                        // from getting lost after a monitor config change
                        // (e.g. unplugging a secondary display). Also rescale
                        // geometry when the target monitor's DPI differs from
                        // the one saved — otherwise a 150%→100% (or docked/
                        // undocked) change restores a wrong-sized window.
                        let saved_scale = s.main_window_scale;
                        let (mut final_x, mut final_y) = (saved_x, saved_y);
                        let (mut final_w, mut final_h) = (saved_w, saved_h);

                        // ── 损坏自愈 ──
                        // 历史缺陷：虚拟显示器（缩放比与物理屏不同，如
                        // GameViewer 1.0 vs 物理 1.5）被“最近显示器”匹配选中
                        // 时，DPI 重缩放会把几何每次缩小 33%，多次启动后
                        // 复合收缩到最小尺寸（实测 550×688 卡死）。
                        // 检测：保存的物理尺寸换算成逻辑尺寸后小于窗口
                        // 最小值（550×688）→ 几何已损坏 → 本次弃用恢复，
                        // 回到默认 1100×750 居中；随后防抖保存会写入
                        // 正确的新几何（自愈）。
                        let logical_w = saved_w as f64 / saved_scale.max(0.1);
                        let logical_h = saved_h as f64 / saved_scale.max(0.1);
                        let geometry_corrupt =
                            saved_scale > 0.0 && (logical_w < 550.0 || logical_h < 688.0);

                        if let Ok(monitors) = w.available_monitors() {
                            if !monitors.is_empty() && !geometry_corrupt {
                                // Find which monitor the saved position is closest to
                                let saved_center_x = saved_x + saved_w as i32 / 2;
                                let saved_center_y = saved_y + saved_h as i32 / 2;
                                let mut best_monitor = None;
                                let mut best_dist = i64::MAX;
                                for m in &monitors {
                                    let pos = m.position();
                                    let size = m.size();
                                    let mon_cx = pos.x + size.width as i32 / 2;
                                    let mon_cy = pos.y + size.height as i32 / 2;
                                    let dx = saved_center_x - mon_cx;
                                    let dy = saved_center_y - mon_cy;
                                    let dist = (dx as i64) * (dx as i64) + (dy as i64) * (dy as i64);
                                    if dist < best_dist {
                                        best_dist = dist;
                                        best_monitor = Some(m);
                                    }
                                }
                                if let Some(m) = best_monitor {
                                    let mpos = m.position();
                                    let msize = m.size();
                                    let cur_scale = m.scale_factor();
                                    // 重缩放仅在该显示器的缩放比与保存值不同、
                                    // 且保存的中心确实落在该显示器范围内时执行
                                    // ——虚拟显示器的缩放比误判正是收缩元凶。
                                    let center_on_monitor = saved_center_x >= mpos.x - 200
                                        && saved_center_x <= mpos.x + msize.width as i32 + 200
                                        && saved_center_y >= mpos.y - 200
                                        && saved_center_y <= mpos.y + msize.height as i32 + 200;
                                    if saved_scale > 0.0
                                        && (cur_scale - saved_scale).abs() > 0.01
                                        && center_on_monitor
                                    {
                                        let ratio = cur_scale / saved_scale;
                                        final_x = mpos.x
                                            + ((saved_x - mpos.x) as f64 * ratio).round() as i32;
                                        final_y = mpos.y
                                            + ((saved_y - mpos.y) as f64 * ratio).round() as i32;
                                        final_w =
                                            ((saved_w as f64) * ratio).round() as u32;
                                        final_h =
                                            ((saved_h as f64) * ratio).round() as u32;
                                    }
                                    let min_x = mpos.x;
                                    let min_y = mpos.y;
                                    // Leave at least 100px of title bar visible
                                    let max_x = mpos.x + msize.width as i32 - 100;
                                    let max_y = mpos.y + msize.height as i32 - 100;
                                    final_x = final_x.clamp(min_x, max_x);
                                    final_y = final_y.clamp(min_y, max_y);
                                }
                            }
                        }

                        // 尺寸下限：不得低于最小逻辑尺寸 × 当前缩放
                        // （防止重缩放/损坏几何低于最小值被 tao 钳住）
                        let cur_scale_f = w
                            .current_monitor()
                            .ok()
                            .flatten()
                            .map(|m| m.scale_factor())
                            .unwrap_or(1.5);
                        final_w = final_w.max((550.0 * cur_scale_f) as u32);
                        final_h = final_h.max((688.0 * cur_scale_f) as u32);

                        let _ = w.set_position(PhysicalPosition::new(final_x, final_y));
                        let _ = w.set_size(PhysicalSize::new(final_w, final_h));
                        // WebView2 命中区域同步：窗口被 set_position/set_size
                        // 移动后，输入命中区域可能仍停留在创建时的位置
                        // （表现为按钮可见但无法点击，最大化后才恢复——最大化
                        //  会强制重新同步）。±1px 抖动强制控制器重做边界。
                        let _ = w.set_size(PhysicalSize::new(final_w + 1, final_h));
                        let _ = w.set_size(PhysicalSize::new(final_w, final_h));
                    }
                }
                // ── 启动序列说明：窗口在配置中即可见（backgroundColor 深色
                // 兜底），内容的淡入由前端 CSS（#root 动画）完成。
                // 曾经试验过“隐藏创建 + 前端触发显示”，但 WebView2 隐藏创建
                // 后再次显示存在输入不恢复的问题（标题栏按钮无法点击），
                // 亚克力背景亦会加重输入失效与 WebGL 闪烁——均已回退。

                // ── 几何防抖保存：移动/缩放停止 1 秒后自动落盘。
                // 此前的保存点只有“点 X 关闭”，托盘退出（quit_app）之外的
                // 一切退出方式——终端 Ctrl+C、进程被杀、崩溃——都会让下次
                // 启动恢复到过期的几何，这就是“打开时大小/位置偶尔错误”。
                let geo_save_gen = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
                let geo_app = app_handle.clone();
                let geo_gen_for_events = geo_save_gen.clone();
                window.on_window_event(move |event| {
                    // Debounced save on move/resize settle
                    if matches!(event, WindowEvent::Moved(_) | WindowEvent::Resized(_)) {
                        let gen = geo_gen_for_events
                            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                            + 1;
                        let geo_app = geo_app.clone();
                        let geo_gen = geo_save_gen.clone();
                        tauri::async_runtime::spawn(async move {
                            tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
                            // Only the latest pending event performs the save.
                            if geo_gen.load(std::sync::atomic::Ordering::SeqCst) == gen {
                                save_main_window_geometry(&geo_app);
                            }
                        });
                    }
                    if let WindowEvent::CloseRequested { api, .. } = event {
                        // Save geometry (position/size + monitor scale) before
                        // closing; maximized/minimized keep the stored restore
                        // bounds and only update the maximized flag. The helper
                        // already persists settings.json.
                        let s_clone = save_main_window_geometry(&app_handle)
                            .unwrap_or_else(|| {
                                app_handle.state::<AppState>().settings.lock().unwrap().clone()
                            });
                        let close_to_tray = s_clone.close_to_tray;
                        let state = app_handle.state::<AppState>();
                        if close_to_tray {
                            api.prevent_close();
                            let _ = w.hide();
                        } else {
                            // Exiting for real — save session and close auxiliary windows.
                            // Then explicitly exit so the tray icon is removed (Tauri keeps
                            // the process alive while the tray exists, which previously left
                            // a lingering tray icon after the main window closed).
                            // Release the audio device cleanly FIRST. If exclusive
                            // mode was active, this hands the device back to
                            // the system immediately instead of relying on
                            // process teardown.
                            state.engine.stop();
                            let _ = commands::save_session(state);
                            if let Some(dl) = app_handle.get_webview_window("desktop-lyrics") {
                                let _ = dl.close();
                            }
                            if let Some(popup) = app_handle.get_webview_window("tray-popup") {
                                let _ = popup.close();
                            }
                            app_handle.exit(0);
                        }
                    }
                });
            }

            // ── Tray Popup Window ────────────────────────────────
            let mut tray_builder = tauri::WebviewWindowBuilder::new(
                app,
                "tray-popup",
                tauri::WebviewUrl::App("tray-popup.html".into()),
            )
            .title("Phonon Tray")
            .inner_size(180.0, 180.0)
            .decorations(false)
            .always_on_top(true)
            .visible(false)
            .resizable(false);

            #[allow(unused_mut)]
            #[cfg(not(target_os = "macos"))]
            {
                tray_builder = tray_builder
                    .transparent(true)
                    .skip_taskbar(true)
                    .shadow(false);
            }
            #[cfg(target_os = "macos")]
            {
                // macOS: no transparent() API in Tauri 2.x
            }

            let tray_popup_win = tray_builder.build()?;

            // Hide popup when it loses focus (clicking elsewhere closes it).
            // This is more reliable than the JS-level onFocusChanged listener
            // for skip_taskbar + always_on_top windows on Windows.
            {
                let popup_for_event = tray_popup_win.clone();
                tray_popup_win.on_window_event(move |event| {
                    if let WindowEvent::Focused(false) = event {
                        let _ = popup_for_event.hide();
                    }
                });
            }

            // ── System Tray ──────────────────────────────────────
            let _tray = TrayIconBuilder::new()
                .icon(app_icon.unwrap_or_else(|| app.default_window_icon().unwrap().clone()))
                .show_menu_on_left_click(false)
                .on_tray_icon_event(move |tray, event| {
                    let app = tray.app_handle();
                    match event {
                        TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        } => {
                            // Left click: show/hide main window.
                            // Also hide tray popup if it's open (clicking elsewhere closes it).
                            if let Some(popup) = app.get_webview_window("tray-popup") {
                                if popup.is_visible().unwrap_or(false) {
                                    let _ = popup.hide();
                                }
                            }
                            if let Some(window) = app.get_webview_window("main") {
                                if window.is_visible().unwrap_or(false) {
                                    let _ = window.hide();
                                } else {
                                    let _ = window.show();
                                    let _ = window.set_focus();
                                }
                            }
                        }
                        TrayIconEvent::Click {
                            button: MouseButton::Right,
                            button_state: MouseButtonState::Up,
                            rect,
                            ..
                        } => {
                            // Right click: show/hide the popup control panel
                            if let Some(popup) = app.get_webview_window("tray-popup") {
                                if popup.is_visible().unwrap_or(false) {
                                    let _ = popup.hide();
                                } else {
                                    // Position popup above and to the right of the tray
                                    // icon, offsetting it so it doesn't cover the system
                                    // tray area or the taskbar.
                                    let (rx, ry) = match rect.position {
                                        tauri::Position::Physical(p) => (p.x, p.y),
                                        tauri::Position::Logical(p) => (p.x as i32, p.y as i32),
                                    };
                                    let rw = match rect.size {
                                        tauri::Size::Physical(s) => s.width as i32,
                                        tauri::Size::Logical(s) => s.width as i32,
                                    };
                                    // Popup dimensions (must match inner_size in builder).
                                    let pw = 180;
                                    let ph = 180;
                                    // Horizontal: align the right edge of the popup with the
                                    // right edge of the tray icon, plus a small rightward
                                    // offset so it doesn't sit directly on top of the icon.
                                    let x = rx + rw - pw + 20;
                                    // Vertical: place popup above the icon (bottom edge
                                    // sits just above the tray area) with a small gap so
                                    // it doesn't overlap the taskbar.
                                    let y = ry - ph - 8;
                                    let _ = popup.set_position(tauri::PhysicalPosition::new(
                                        x.max(0),
                                        y.max(0),
                                    ));
                                    let _ = popup.show();
                                    // Toggle always_on_top off→on to force window activation on Windows,
                                    // which makes subsequent focus-loss events fire reliably when the
                                    // user clicks elsewhere (desktop, taskbar, other apps).
                                    let _ = popup.set_always_on_top(false);
                                    let _ = popup.set_always_on_top(true);
                                    let _ = popup.set_focus();
                                }
                            }
                        }
                        _ => {}
                    }
                })
                .build(app)?;

            // Listen for show-main-window event from tray popup
            let show_handle = handle.clone();
            app.listen("show-main-window", move |_event| {
                if let Some(window) = show_handle.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
                // Hide tray popup when main window is shown
                if let Some(popup) = show_handle.get_webview_window("tray-popup") {
                    let _ = popup.hide();
                }
            });

            // ── Global Shortcuts ─────────────────────────────────
            let gs_handle = handle.clone();
            app.listen("global-shortcut://play_pause", move |_event| {
                let state = gs_handle.state::<AppState>();
                let s = state.engine.state();
                match s {
                    phonon_core::PlaybackState::Playing => state.engine.pause(),
                    _ => {
                        let _ = state.engine.start_playback();
                    }
                }
            });

            let gs_handle = handle.clone();
            app.listen("global-shortcut://next_track", move |_event| {
                let state = gs_handle.state::<AppState>();
                state.engine.next();
            });

            let gs_handle = handle.clone();
            app.listen("global-shortcut://prev_track", move |_event| {
                let state = gs_handle.state::<AppState>();
                state.engine.previous();
            });

            let gs_handle = handle.clone();
            app.listen("global-shortcut://stop", move |_event| {
                let state = gs_handle.state::<AppState>();
                state.engine.stop();
            });

            // Initialize the global event channel (buffer: 256 messages).
            let (tx, _) = tokio::sync::broadcast::channel(256);
            phonon_core::EVENT_TX
                .set(tx)
                .expect("EVENT_TX already initialized");

            // Forward all app events to the frontend from a dedicated thread.
            // Checks `frontend_reloading` — when the frontend is mid-refresh,
            // events are silently discarded instead of being emitted into a
            // dead webview. This is the primary defense against
            // "Couldn't find callback id" warnings: without it, SpectrumData
            // (40 Hz) and PlaybackProgress (20 Hz) events keep arriving at
            // the JS listeners during the narrow unload window, and any
            // listener that calls `invoke()` in response would schedule a
            // callback that can never be resolved.
            let fwd_handle = handle.clone();
            let fwd_state = handle.state::<AppState>();
            let reloading_flag = fwd_state.frontend_reloading.clone();
            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new()
                    .expect("Failed to create event forwarding runtime");
                rt.block_on(async move {
                    let mut rx = phonon_core::EVENT_TX.get().unwrap().subscribe();
                    loop {
                        match rx.recv().await {
                            Ok(event) => {
                                if reloading_flag.load(std::sync::atomic::Ordering::SeqCst) {
                                    continue;
                                }
                                let _ = fwd_handle.emit("app-event", &event);
                            }
                            // Receiver fell behind the broadcast buffer: skip the
                            // missed events and keep forwarding. Exiting here would
                            // permanently kill ALL frontend events (progress,
                            // spectrum, state, hotplug) until app restart.
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                                log::warn!("Event forwarder lagged, dropped {} events", n);
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                        }
                    }
                });
            });

            // Register HW volume callback
            let state = app.state::<AppState>();
            // Wire up AppHandle onto AppState so hotplug_event_handler can emit
            // frontend events (phonon:toast / phonon:audioDeviceSwitch). MUST be
            // set before start_hotplug_monitor() runs.
            let _ = state.app_handle.set(handle.clone());
            let hw_handle = handle.clone();
            state.engine.set_hw_volume_callback(move |level| {
                let _ = hw_handle.emit("hw-volume", level);
            });

            // Create WASAPI HW volume controller on startup
            #[cfg(windows)]
            {
                use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
                let _com_init = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
                let device_name = state.device_manager.lock().unwrap().current_device_name();
                if let Some(device_name) = device_name {
                    match phonon_core::wasapi::WasapiEndpointVolume::new(&device_name) {
                        Ok(controller) => {
                            state.volume.lock().unwrap().set_hw_controller(controller);
                            log::info!("HW volume controller initialized");
                        }
                        Err(e) => log::warn!("Failed to create HW volume controller: {e}"),
                    }
                }
            }

            // Start hotplug monitoring
            state.start_hotplug_monitor();

            // ── CLI Args ─────────────────────────────────────────
            let cli_files = take_cli_args();
            if !cli_files.is_empty() {
                for file in &cli_files {
                    let _ = state.engine.enqueue(file);
                }
                log::info!("Queued {} file(s) from CLI args", cli_files.len());
                // Auto-start playback if files were passed
                if let Err(e) = state.engine.start_playback() {
                    log::warn!("Failed to auto-play CLI files: {}", e);
                }
            }

            // Start plugin file watcher for hot reload.
            // Recursively watches the plugin directory for new/modified/deleted .wasm files.
            let pr = state.plugin_runtime.clone();
            let pr_handle = handle.clone();
            std::thread::spawn(move || {
                let mut mod_times: std::collections::HashMap<String, std::time::SystemTime> =
                    std::collections::HashMap::new();
                loop {
                    std::thread::sleep(Duration::from_secs(2));
                    let plugin_dir = pr.plugin_dir();
                    if !plugin_dir.exists() {
                        continue;
                    }

                    // Collect all .wasm files recursively
                    let mut current_files: Vec<std::path::PathBuf> = Vec::new();
                    let mut found_any = false;
                    fn collect_wasm(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
                        if let Ok(entries) = std::fs::read_dir(dir) {
                            for entry in entries.flatten() {
                                let path = entry.path();
                                if path.is_dir() {
                                    if path.file_name().is_some_and(|n| n == "target") {
                                        continue;
                                    }
                                    collect_wasm(&path, out);
                                } else if path.extension().is_some_and(|ext| ext == "wasm") {
                                    out.push(path);
                                }
                            }
                        }
                    }
                    collect_wasm(&plugin_dir, &mut current_files);

                    let mut current_keys: std::collections::HashSet<String> =
                        std::collections::HashSet::new();

                    for path in &current_files {
                        let file_path = path.to_string_lossy().to_string();
                        current_keys.insert(file_path.clone());
                        let mod_time = match path.metadata().ok().and_then(|m| m.modified().ok()) {
                            Some(t) => t,
                            None => continue,
                        };

                        if let Some(prev_time) = mod_times.get(&file_path).copied() {
                            if mod_time > prev_time {
                                // File was modified — reload it
                                found_any = true;
                                mod_times.insert(file_path.clone(), mod_time);
                                log::info!("[plugin-watcher] Modified: {}", file_path);
                                match pr.reload(&file_path) {
                                    Ok(()) => {
                                        let _ = pr_handle.emit("plugin-reloaded", &file_path);
                                    }
                                    Err(e) => {
                                        log::error!(
                                            "[plugin-watcher] Failed to reload {}: {}",
                                            file_path,
                                            e
                                        );
                                    }
                                }
                            }
                        } else {
                            // New file detected
                            found_any = true;
                            mod_times.insert(file_path.clone(), mod_time);
                            log::info!("[plugin-watcher] New plugin: {}", file_path);
                        }
                    }

                    // Check for deleted files
                    let removed: Vec<String> = mod_times
                        .keys()
                        .filter(|k| !current_keys.contains(*k))
                        .cloned()
                        .collect();
                    for key in &removed {
                        found_any = true;
                        mod_times.remove(key);
                        log::info!("[plugin-watcher] Removed: {}", key);
                    }

                    // If any change detected, trigger a full scan
                    if found_any {
                        if let Err(e) = pr.scan() {
                            log::error!("[plugin-watcher] Scan failed: {}", e);
                        } else {
                            let _ = pr_handle.emit("plugins-changed", "scan");
                        }
                    }
                }
            });
            let volume = state.volume.clone();
            #[allow(unused_variables)]
            let _device_manager = state.device_manager.clone();
            let engine_handle = handle.clone();
            tauri::async_runtime::spawn(async move {
                // Initialize COM for WASAPI HW volume on this thread
                #[cfg(windows)]
                unsafe {
                    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
                    let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                }
                // Dynamic HW volume controller — created when device becomes available
                #[cfg(windows)]
                let mut hw_ctrl: Option<phonon_core::wasapi::WasapiEndpointVolume> = None;
                #[cfg(windows)]
                let mut last_device: Option<String> = None;
                loop {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    #[cfg(windows)]
                    {
                        // Check if device changed or became available; re-create
                        // hw_ctrl if needed. This task OWNS the canonical HW
                        // controller: after every device switch it re-attaches
                        // the fresh controller to VolumeControl so the audio
                        // callback's apply_hw_volume targets the CURRENT device
                        // (previously only the startup device was ever attached).
                        let dm = _device_manager.lock().unwrap();
                        let current_device = dm
                            .current_device_name()
                            .or_else(|| dm.default_device_name());
                        drop(dm);
                        if current_device != last_device {
                            last_device = current_device.clone();
                            hw_ctrl = current_device.as_ref().and_then(|name| {
                                phonon_core::wasapi::WasapiEndpointVolume::new(name).ok()
                            });
                            if let Some(ref ctrl) = hw_ctrl {
                                log::info!(
                                    "[polling] HW volume controller created for device: {:?}",
                                    current_device
                                );
                                if let Ok(mut vol) = volume.try_lock() {
                                    vol.set_hw_controller(ctrl.clone());
                                }
                            }
                        }
                        use phonon_core::volume::VolumeMode;
                        // Read the device volume WITHOUT holding the volume
                        // lock — a slow COM round-trip must never block the
                        // real-time audio callback waiting on this mutex.
                        let sys_vol = match hw_ctrl.as_ref() {
                            Some(hw) => hw.get_master_volume().ok(),
                            None => None,
                        };
                        if let Some(sys_vol) = sys_vol {
                            let Ok(mut vol) = volume.try_lock() else {
                                continue; // audio callback holds it — skip this tick
                            };
                            if vol.mode() == VolumeMode::Hardware {
                                let clamped = sys_vol.clamp(0.0, 1.0);
                                let internal = vol.level();
                                if (clamped - internal).abs() > 0.01 {
                                    vol.set_volume(clamped);
                                    drop(vol);
                                    let _ = engine_handle.emit("hw-volume", clamped);
                                }
                            }
                        }
                    }
                    #[cfg(not(windows))]
                    {
                        // Prevent dead_code warning for engine_handle on
                        // non-windows builds where hw-volume path is skipped.
                        let _ = &engine_handle;
                    }
                }
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
