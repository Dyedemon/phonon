//! Phonon Plugin SDK
//!
//! Helper macros and utilities for building WASM plugins for Phonon.
//!
//! # Quick Start
//!
//! 1. Add `phonon-plugin-sdk` to your `Cargo.toml`
//! 2. Build with `cargo build --target wasm32-wasip1 --release`
//! 3. Copy the `.wasm` file to Phonon's plugins directory
//!
//! # Plugin Types
//!
//! | Type | Required Exports | Description |
//! |------|-----------------|-------------|
//! | DSP Processor | `plugin_process` | Audio DSP effects |
//! | Time Stretch | `plugin_time_stretch_*` | Pitch-preserving speed change |
//! | Audio Decoder | `plugin_decode` | Audio format decoder |
//! | Source Provider | `plugin_search` | Audio source provider |

use serde::Serialize;

// ── Memory helpers ───────────────────────────────────────────────

/// Allocate a string in WASM linear memory and return a pointer to it.
/// The caller must free it with `plugin_free`.
pub fn allocate_string(s: &str) -> *mut u8 {
    let bytes = s.as_bytes().to_vec();
    let ptr = bytes.as_ptr() as *mut u8;
    std::mem::forget(bytes);
    ptr
}

/// Allocate a JSON value in WASM linear memory and return a pointer to it.
pub fn allocate_json<T: Serialize>(value: &T) -> *mut u8 {
    let json = serde_json::to_string(value).unwrap_or_else(|_| "{}".into());
    allocate_string(&json)
}

/// Free memory allocated by `allocate_string` or `allocate_json`.
/// # Safety
/// `ptr` must have been allocated by this crate.
#[no_mangle]
pub unsafe extern "C" fn plugin_free(ptr: *mut u8, len: usize) {
    if !ptr.is_null() && len > 0 {
        let _ = Vec::from_raw_parts(ptr, len, len);
    }
}

/// Read a string from WASM linear memory at the given pointer.
/// # Safety
/// `ptr` must point to valid UTF-8 data of length `len`.
pub unsafe fn read_string(ptr: *const u8, len: usize) -> &'static str {
    if ptr.is_null() || len == 0 {
        return "";
    }
    let bytes = std::slice::from_raw_parts(ptr, len);
    std::str::from_utf8(bytes).unwrap_or("")
}

/// Read an f32 slice from WASM linear memory.
/// # Safety
/// `ptr` must point to valid f32 data.
pub unsafe fn read_f32_slice(ptr: *const u8, len: usize) -> &'static [f32] {
    let Some(byte_len) = len.checked_mul(4) else {
        return &[];
    };
    if ptr.is_null() || byte_len == 0 {
        return &[];
    }
    let data = std::slice::from_raw_parts(ptr, byte_len);
    std::slice::from_raw_parts(data.as_ptr() as *const f32, len)
}

/// Read an f32 slice from WASM linear memory (mutable).
/// # Safety
/// `ptr` must point to valid f32 data.
pub unsafe fn read_f32_slice_mut(ptr: *mut u8, len: usize) -> &'static mut [f32] {
    let Some(byte_len) = len.checked_mul(4) else {
        return &mut [];
    };
    if ptr.is_null() || byte_len == 0 {
        return &mut [];
    }
    let data = std::slice::from_raw_parts_mut(ptr, byte_len);
    std::slice::from_raw_parts_mut(data.as_mut_ptr() as *mut f32, len)
}

// ── DSP Plugin helpers ──────────────────────────────────────────

/// Macro to declare the standard DSP plugin exports.
///
/// # Example
/// ```
/// use phonon_plugin_sdk::declare_dsp_plugin;
///
/// static mut GAIN: f32 = 1.0;
///
/// declare_dsp_plugin! {
///     name: "my-gain-plugin",
///     version: "1.0.0",
///     init: || {
///         // Optional: called once on load
///     },
///     process: |samples: &mut [f32], _channels: u16, _sample_rate: u32| {
///         for s in samples.iter_mut() {
///             *s *= unsafe { GAIN };
///         }
///         Ok(())
///     },
/// }
/// ```
#[macro_export]
macro_rules! declare_dsp_plugin {
    (
        name: $name:expr,
        version: $version:expr,
        init: $init_fn:expr,
        process: $process_fn:expr $(,)?
    ) => {
        static mut _INITIALIZED: bool = false;

        #[no_mangle]
        pub extern "C" fn plugin_init() {
            unsafe {
                if _INITIALIZED {
                    return;
                }
                _INITIALIZED = true;
            }
            $init_fn();
        }

        #[no_mangle]
        pub extern "C" fn plugin_process(
            data_ptr: i32,
            data_len: i32,
            channels: i32,
            sample_rate: i32,
        ) {
            // Reject garbage lengths up front: a negative or oversized count
            // must never reach the raw-slice helpers.
            if data_ptr == 0 || data_len <= 0 {
                return;
            }
            let samples =
                unsafe { $crate::read_f32_slice_mut(data_ptr as *mut u8, data_len as usize) };
            let result: Result<(), String> =
                $process_fn(samples, channels as u16, sample_rate as u32);
            if let Err(e) = result {
                // Log error via host (if available)
                let _ = e;
            }
        }

        #[no_mangle]
        pub extern "C" fn plugin_name() -> *const u8 {
            concat!($name, "\0").as_ptr()
        }

        #[no_mangle]
        pub extern "C" fn plugin_version() -> *const u8 {
            concat!($version, "\0").as_ptr()
        }
    };
}

// ── TimeStretch Plugin helpers ──────────────────────────────────

/// Macro to declare the standard TimeStretch plugin exports.
#[macro_export]
macro_rules! declare_timestretch_plugin {
    (
        name: $name:expr,
        version: $version:expr,
        init: $init_fn:expr,
        stretch_init: $stretch_init_fn:expr,
        stretch_push: $stretch_push_fn:expr $(,)?
    ) => {
        static mut _INITIALIZED: bool = false;

        #[no_mangle]
        pub extern "C" fn plugin_init() {
            unsafe {
                if _INITIALIZED {
                    return;
                }
                _INITIALIZED = true;
            }
            $init_fn();
        }

        #[no_mangle]
        pub extern "C" fn plugin_time_stretch_init(
            speed: f32,
            channels: i32,
            sample_rate: i32,
        ) -> i32 {
            match $stretch_init_fn(speed, channels as u16, sample_rate as u32) {
                Ok(()) => 0,
                Err(_) => -1,
            }
        }

        #[no_mangle]
        pub extern "C" fn plugin_time_stretch_push(
            input_ptr: i32,
            input_len: i32,
            output_ptr: i32,
            output_cap: i32,
        ) -> i32 {
            let input = unsafe {
                if input_ptr == 0 || input_len <= 0 {
                    &[][..]
                } else {
                    $crate::read_f32_slice(input_ptr as *const u8, input_len as usize)
                }
            };
            let output =
                unsafe { $crate::read_f32_slice_mut(output_ptr as *mut u8, output_cap as usize) };
            let mut out_vec = Vec::new();
            match $stretch_push_fn(input, &mut out_vec) {
                Ok(()) => {
                    let n = out_vec.len().min(output.len());
                    output[..n].copy_from_slice(&out_vec[..n]);
                    n as i32
                }
                Err(_) => -1,
            }
        }

        #[no_mangle]
        pub extern "C" fn plugin_time_stretch_flush(_output_ptr: i32, _output_cap: i32) -> i32 {
            0
        }

        #[no_mangle]
        pub extern "C" fn plugin_time_stretch_reset() {}

        #[no_mangle]
        pub extern "C" fn plugin_name() -> *const u8 {
            concat!($name, "\0").as_ptr()
        }

        #[no_mangle]
        pub extern "C" fn plugin_version() -> *const u8 {
            concat!($version, "\0").as_ptr()
        }
    };
}

// ── Decoder Plugin helpers ──────────────────────────────────────

/// Macro to declare the standard Decoder plugin exports.
#[macro_export]
macro_rules! declare_decoder_plugin {
    (
        name: $name:expr,
        version: $version:expr,
        init: $init_fn:expr,
        decode: $decode_fn:expr $(,)?
    ) => {
        static mut _INITIALIZED: bool = false;

        #[no_mangle]
        pub extern "C" fn plugin_init() {
            unsafe {
                if _INITIALIZED {
                    return;
                }
                _INITIALIZED = true;
            }
            $init_fn();
        }

        #[no_mangle]
        pub extern "C" fn plugin_decode(path_ptr: *const u8, path_len: usize) -> *mut u8 {
            let path = unsafe { $crate::read_string(path_ptr, path_len) };
            match $decode_fn(path) {
                Ok(samples) => $crate::allocate_json(&samples),
                Err(_) => $crate::allocate_string("[]"),
            }
        }

        #[no_mangle]
        pub extern "C" fn plugin_name() -> *const u8 {
            concat!($name, "\0").as_ptr()
        }

        #[no_mangle]
        pub extern "C" fn plugin_version() -> *const u8 {
            concat!($version, "\0").as_ptr()
        }
    };
}

// ── Config helpers ──────────────────────────────────────────────

/// Macro to declare plugin config exports.
/// Adds `plugin_get_config` and `plugin_set_config` to the plugin.
#[macro_export]
macro_rules! declare_plugin_config {
    ($default_config:expr) => {
        use std::sync::Mutex;
        static CONFIG: Mutex<String> = Mutex::new(String::new());
        static CONFIG_INIT: std::sync::Once = std::sync::Once::new();

        #[no_mangle]
        pub extern "C" fn plugin_get_config() -> *mut u8 {
            CONFIG_INIT.call_once(|| {
                *CONFIG.lock().unwrap() = $default_config.to_string();
            });
            let config = CONFIG.lock().unwrap();
            $crate::allocate_string(&config)
        }

        #[no_mangle]
        pub extern "C" fn plugin_set_config(config_ptr: *const u8, config_len: usize) {
            let config = unsafe { $crate::read_string(config_ptr, config_len) };
            *CONFIG.lock().unwrap() = config.to_string();
        }
    };
}

/// Declare a custom command handler for the plugin.
///
/// The host calls `plugin_command` via `PluginRuntime::call_plugin_command`.
/// The plugin receives a command name and a JSON input string, and returns
/// a JSON output string allocated in WASM linear memory.
///
/// The closure receives `(command: &str, input_json: &str)` and must return
/// a `Result<String, String>` — `Ok(json)` is returned to the host,
/// `Err(msg)` results in a null return (host sees error).
///
/// # ABI
///
/// ```text
/// plugin_command(cmd_ptr, cmd_len, input_ptr, input_len) -> i64
/// ```
/// Return value packs (output_ptr as u32) << 32 | (output_len as u32).
/// output_ptr = 0 means error.
///
/// The host calls `plugin_free` on the returned pointer after reading.
#[macro_export]
macro_rules! declare_plugin_command {
    ($handler:expr) => {
        #[no_mangle]
        pub extern "C" fn plugin_command(
            cmd_ptr: *const u8,
            cmd_len: i32,
            input_ptr: *const u8,
            input_len: i32,
        ) -> i64 {
            if cmd_ptr.is_null() || input_ptr.is_null() || cmd_len <= 0 || input_len < 0 {
                return 0;
            }
            let command = unsafe { $crate::read_string(cmd_ptr, cmd_len as usize) };
            let input_json = unsafe { $crate::read_string(input_ptr, input_len as usize) };

            let handler_fn: fn(&str, &str) -> Result<String, String> = $handler;
            match handler_fn(command, input_json) {
                Ok(output) => {
                    let ptr = $crate::allocate_string(&output);
                    let len = output.len() as u32;
                    let ptr_u32 = ptr as u32;
                    ((ptr_u32 as i64) << 32) | (len as i64)
                }
                Err(_) => 0,
            }
        }
    };
}

// ── Host imports & event emission ───────────────────────────────
//
// Phonon 在加载 Wasm 插件时会以 `(module = "host", name = "emit_event")` 的
// 形式注入 Host Import。调用后，Host 通过 Tauri `app-event`（type = "PluginUserEvent"）
// 将事件推送到前端。前端可通过：
//   window.addEventListener('plugin-user-event', (e: CustomEvent) => {
//       console.log(e.detail.name, e.detail.payload)
//   })
// 订阅自定义事件。
//
// 注意：`extern "C"` 仅在 wasm32 目标下声明（真正由宿主通过 wasmtime 注入）。
// 在非 wasm 目标（本地测试/构建）下提供 no-op stub，避免 LNK2019 链接错误。

/// Host 侧原始导入函数。通常无需直接调用，使用 [`emit_event`] / [`emit_json`]。
pub mod host {
    #[cfg(target_arch = "wasm32")]
    extern "C" {
        /// 注入的 host import。name 与 payload 必须为宿主线性内存中的有效 UTF-8 字节范围。
        pub fn emit_event(
            name_ptr: *const u8,
            name_len: i32,
            payload_ptr: *const u8,
            payload_len: i32,
        );
    }

    /// 非 wasm 目标下的 no-op stub：宿主 import 在本地不注入，测试时静默丢弃。
    #[cfg(not(target_arch = "wasm32"))]
    #[no_mangle]
    pub extern "C" fn emit_event(
        _name_ptr: *const u8,
        _name_len: i32,
        _payload_ptr: *const u8,
        _payload_len: i32,
    ) {
        // no-op: host import 仅在 wasm runtime 内被 wasmtime 注入
    }
}

/// 向宿主（进而向 Phonon 前端）派发一个自定义事件。
///
/// - `name`：事件名（任意字符串，由插件自行约定语义）
/// - `payload`：事件 payload（任意字符串，通常为 JSON，插件自行在前端 JSON.parse）
///
/// 前端订阅示例：
/// ```ts
/// window.addEventListener('plugin-user-event', (e) => {
///   const { name, payload } = (e as CustomEvent).detail
///   if (name === 'my-plugin/beat') console.log(JSON.parse(payload))
/// })
/// ```
pub fn emit_event(name: &str, payload: &str) {
    // wasm32 目标下 `host::emit_event` 是 extern "C"（需要 unsafe）；
    // 非 wasm32 目标下是普通 Rust 函数（无需 unsafe）。
    // 统一用 cfg 分发，避免 unused_unsafe 警告。
    #[cfg(target_arch = "wasm32")]
    unsafe {
        host::emit_event(
            name.as_ptr(),
            name.len() as i32,
            payload.as_ptr(),
            payload.len() as i32,
        )
    }
    #[cfg(not(target_arch = "wasm32"))]
    host::emit_event(
        name.as_ptr(),
        name.len() as i32,
        payload.as_ptr(),
        payload.len() as i32,
    )
}

/// 派发一个 JSON 事件：payload 为任意可序列化值（内部会序列化为 JSON 字符串）。
pub fn emit_json<T: serde::Serialize>(name: &str, payload: &T) {
    match serde_json::to_string(payload) {
        Ok(s) => emit_event(name, &s),
        Err(e) => emit_event(name, &format!(r#"{{"serialization_error":"{}"}}"#, e)),
    }
}
