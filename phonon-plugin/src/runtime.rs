//! WASM plugin runtime based on wasmtime.
//!
//! Handles plugin discovery, loading, sandboxing, and resource limits.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex, RwLock, Weak,
};
use std::time::Duration;
use wasmtime::*;

use crate::types::*;
use phonon_core::{PluginTimeStretcher, TimeStretch, TimeStretchFactory};

// ── 三层插件：辅助常量与纯函数 ────────────────────────────────

/// 扩展示例插件的 manifest.name 白名单（示例插件已删除，当前仅保留测试占位）。
///
/// 如果未来重新引入示例子包，需在此登记其 manifest.name，
/// 以便 scan 阶段在 `debug_assertions` 模式下将它们标记为 `PluginOrigin::WasmExample`
/// （优先级低于用户手动导入的 `WasmExternal`，冲突时 External 覆盖 Example）。
///
/// `_test_example_plugin_1/2` 为单元测试占位符，不对应任何真实 wasm 文件。
const EXAMPLE_NAMES: &[&str] = &["_test_example_plugin_1", "_test_example_plugin_2"];

/// 判断一个 wasm name 是否在示例子包白名单内。
fn is_example_name(name: &str) -> bool {
    EXAMPLE_NAMES.contains(&name)
}

/// 「扫描阶段打 origin」辅助函数。
///
/// 判定规则（与 Spec §2 扫描/冲突处理表格对齐）：
///   1. file_path 位于用户 plugins_dir（self.plugin_dir）下 → WasmExternal（用户目录永远最高优先级）
///   2. 否则 cfg(debug_assertions) + manifest.name ∈ EXAMPLE_NAMES → WasmExample
///   3. 其他 → WasmExternal（Default 回退）
fn tag_origin_for_scan(
    file_path: &str,
    _plugin_name: &str,
    user_plugins_dir: &Path,
) -> PluginOrigin {
    let file_path = Path::new(file_path);
    // 用户目录下发现的一律是 WasmExternal（用户目录优先于示例目录）
    if let Ok(rel) = file_path.strip_prefix(user_plugins_dir) {
        let _ = rel;
        return PluginOrigin::WasmExternal;
    }
    // 不是用户目录 → 名字命中示例白名单 + 开发模式 → WasmExample
    // 注：发布模式（release / not debug_assertions）下 WasmExample 不应该出现在
    // extra_scan_dirs 里；但即使由于某些配置（例如用户手动塞 target 目录）
    // 发现了同名 wasm，也按 WasmExternal 标记，避免混淆。
    #[cfg(debug_assertions)]
    {
        if is_example_name(_plugin_name) {
            return PluginOrigin::WasmExample;
        }
    }
    // 其他：默认外部
    PluginOrigin::WasmExternal
}

/// 三层合并 + 冲突去重的纯函数（提取出来便于直接单元测试，不依赖 self）。
///
/// 输入：已被 scan 阶段打好 origin 的 wasm_plugins 集合（WasmExample + WasmExternal 混合）。
/// 输出：先 External（按用户目录优先且不去重，因为 scan 阶段 seen_names 已去重）、
///       后 Example（仅保留 External 集合中没有同名覆盖的 Example）。
pub fn merge_plugin_layers(mut wasm_plugins: Vec<PluginInfo>) -> Vec<PluginInfo> {
    use std::collections::HashSet;

    let mut result = Vec::with_capacity(wasm_plugins.len());
    let mut external_names = HashSet::new();

    // Pass 1：全部 WasmExternal 先入列，登记名字集合
    for info in wasm_plugins.iter() {
        if matches!(info.origin, PluginOrigin::WasmExternal) {
            external_names.insert(info.manifest.name.clone());
        }
    }
    wasm_plugins.retain(|i| {
        if matches!(i.origin, PluginOrigin::WasmExternal) {
            result.push(i.clone());
            false
        } else {
            true
        }
    });

    // Pass 2：WasmExample 中未被 External 覆盖的才加入；EXAMPLE_NAMES 命中校验多一道防线
    for info in wasm_plugins {
        if matches!(info.origin, PluginOrigin::WasmExample)
            && is_example_name(&info.manifest.name)
            && !external_names.contains(&info.manifest.name)
        {
            result.push(info);
        }
    }
    result
}

/// Default timeout for plugin function calls (5 seconds).
#[allow(dead_code)]
const DEFAULT_CALL_TIMEOUT_MS: u64 = 5000;

/// Interval between engine epoch bumps by the ticker thread (milliseconds).
/// All plugin-call timeouts are measured in these ticks.
const EPOCH_TICK_MS: u64 = 5;

/// Start the engine's epoch ticker thread, once per engine.
///
/// wasmtime's recommended timeout pattern: a single background thread bumps
/// the engine epoch at a fixed interval, and every call arms its own store
/// with a relative deadline before invoking. This replaces the previous
/// watchdog-thread-per-call scheme (a thread spawn + join on every audio
/// block) and confines a timeout to the offending store instead of
/// interrupting every plugin sharing the engine.
fn ensure_epoch_ticker_started(engine: &Arc<Engine>, started: &std::sync::Once) {
    started.call_once(|| {
        let engine = engine.clone();
        let _ = std::thread::Builder::new()
            .name("wasm-epoch-ticker".into())
            .spawn(move || loop {
                std::thread::sleep(Duration::from_millis(EPOCH_TICK_MS));
                engine.increment_epoch();
            });
    });
}

/// User data carried inside each wasmtime `Store`. The `limits` field is
/// used by wasmtime's `store.limiter()` to enforce memory + table
/// ceilings; the remaining weak refs let host imports look up per-plugin
/// config blobs / version counters without ever holding onto the
/// `PluginRuntime` by a strong `Arc` (avoids cycles).
pub struct PluginStoreData {
    pub limits: StoreLimits,
    pub config_versions: std::sync::Weak<RwLock<HashMap<String, u64>>>,
    pub plugin_configs: std::sync::Weak<Mutex<HashMap<String, String>>>,
}

/// A loaded WASM plugin instance ready for processing.
struct LoadedWasm {
    info: PluginInfo,
    store: Arc<Mutex<Store<PluginStoreData>>>,
    instance: Instance,
}

/// Read a UTF-8 string out of a wasmtime `Memory`. Shared helper used by
/// every host import that accepts a (ptr, len) string parameter.
fn read_utf8<T>(memory: &Memory, caller: &wasmtime::StoreContext<'_, T>, ptr: i32, len: i32) -> Option<String> {
    if ptr < 0 || len < 0 {
        return None;
    }
    let mut buf = vec![0u8; len as usize];
    memory.read(caller, ptr as usize, &mut buf).ok()?;
    String::from_utf8(buf).ok()
}

/// Stub WASI preview1 imports that the wasm32-wasip1 target links against
/// but plugins don't actually use. File I/O stubs return EBADF (8).
fn add_wasi_stubs(linker: &mut Linker<PluginStoreData>) -> Result<(), PluginError> {
    let m = |e| PluginError::Module(format!("WASI stub: {}", e));

    // environ_sizes_get
    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "environ_sizes_get",
            |mut caller: Caller<'_, PluginStoreData>, count: i32, buf_size: i32| -> i32 {
                let mem = match caller.get_export("memory") {
                    Some(Extern::Memory(m)) => m,
                    _ => return 1,
                };
                let data = mem.data_mut(&mut caller);
                let c = count as usize;
                let s = buf_size as usize;
                if c + 4 <= data.len() && s + 4 <= data.len() {
                    data[c..c + 4].copy_from_slice(&0u32.to_le_bytes());
                    data[s..s + 4].copy_from_slice(&0u32.to_le_bytes());
                }
                0
            },
        )
        .map_err(m)?;

    // environ_get
    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "environ_get",
            |_: Caller<'_, PluginStoreData>, _a: i32, _b: i32| -> i32 { 0 },
        )
        .map_err(m)?;

    // random_get
    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "random_get",
            |mut caller: Caller<'_, PluginStoreData>, buf: i32, buf_len: i32| -> i32 {
                let mem = match caller.get_export("memory") {
                    Some(Extern::Memory(m)) => m,
                    _ => return 1,
                };
                let data = mem.data_mut(&mut caller);
                let o = buf as usize;
                let l = buf_len as usize;
                if o + l > data.len() {
                    return 1;
                }
                for (i, slot) in data[o..o + l].iter_mut().enumerate() {
                    *slot = (i as u8).wrapping_mul(17);
                }
                0
            },
        )
        .map_err(m)?;

    // proc_exit
    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "proc_exit",
            |_: Caller<'_, PluginStoreData>, _code: i32| {},
        )
        .map_err(m)?;

    // File I/O stubs (return EBADF)
    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "fd_write",
            |_: Caller<'_, PluginStoreData>,
             _fd: i32,
             _iovs: i32,
             _iovs_len: i32,
             _nwritten: i32|
             -> i32 { 8 },
        )
        .map_err(m)?;
    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "fd_read",
            |_: Caller<'_, PluginStoreData>,
             _fd: i32,
             _iovs: i32,
             _iovs_len: i32,
             _nread: i32|
             -> i32 { 8 },
        )
        .map_err(m)?;
    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "fd_close",
            |_: Caller<'_, PluginStoreData>, _fd: i32| -> i32 { 8 },
        )
        .map_err(m)?;
    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "fd_seek",
            |_: Caller<'_, PluginStoreData>,
             _fd: i32,
             _offset: i64,
             _whence: i32,
             _newoffset: i32|
             -> i32 { 8 },
        )
        .map_err(m)?;
    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "fd_fdstat_get",
            |_: Caller<'_, PluginStoreData>, _fd: i32, _stat: i32| -> i32 { 8 },
        )
        .map_err(m)?;
    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "fd_prestat_get",
            |_: Caller<'_, PluginStoreData>, _fd: i32, _buf: i32| -> i32 { 8 },
        )
        .map_err(m)?;
    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "fd_prestat_dir_name",
            |_: Caller<'_, PluginStoreData>, _fd: i32, _path: i32, _path_len: i32| -> i32 { 8 },
        )
        .map_err(m)?;

    // clock_time_get — required by std::time::SystemTime::now() on wasm32-wasip1.
    // Writes a dummy i64 nanosecond timestamp (0 = UNIX_EPOCH) to [time_ptr].
    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "clock_time_get",
            |mut caller: Caller<'_, PluginStoreData>, _clock_id: i32, _precision: i64, time_ptr: i32| -> i32 {
                let mem = match caller.get_export("memory") {
                    Some(Extern::Memory(m)) => m,
                    _ => return 1,
                };
                let o = time_ptr as usize;
                if o + 8 <= mem.data_size(&caller) {
                    let _ = mem.write(&mut caller, o, &0i64.to_le_bytes());
                }
                0
            },
        )
        .map_err(m)?;

    // clock_res_get — paired with clock_time_get.
    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "clock_res_get",
            |mut caller: Caller<'_, PluginStoreData>, _clock_id: i32, res_ptr: i32| -> i32 {
                let mem = match caller.get_export("memory") {
                    Some(Extern::Memory(m)) => m,
                    _ => return 1,
                };
                let o = res_ptr as usize;
                if o + 8 <= mem.data_size(&caller) {
                    let _ = mem.write(&mut caller, o, &1i64.to_le_bytes());
                }
                0
            },
        )
        .map_err(m)?;

    // args_sizes_get — required by std::env::args on wasm32-wasip1.
    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "args_sizes_get",
            |mut caller: Caller<'_, PluginStoreData>, count_ptr: i32, buf_size_ptr: i32| -> i32 {
                let mem = match caller.get_export("memory") {
                    Some(Extern::Memory(m)) => m,
                    _ => return 1,
                };
                let c = count_ptr as usize;
                let s = buf_size_ptr as usize;
                if c + 4 <= mem.data_size(&caller) {
                    let _ = mem.write(&mut caller, c, &0u32.to_le_bytes());
                }
                if s + 4 <= mem.data_size(&caller) {
                    let _ = mem.write(&mut caller, s, &0u32.to_le_bytes());
                }
                0
            },
        )
        .map_err(m)?;

    // args_get
    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "args_get",
            |_: Caller<'_, PluginStoreData>, _argv_ptr: i32, _argv_buf_ptr: i32| -> i32 { 0 },
        )
        .map_err(m)?;

    Ok(())
}

/// 注册"真正的"插件-宿主通道 imports（module 名 = "host"）。
///
/// 这是 Spec §3.4 的 `host::emit_event` 实现；所有 Wasm 插件想向 UI
/// 发自定义 JSON 事件必须走这条通道（Builtin 可直接 `EVENT_TX.send`）。
///
/// 宿主侧保持「最小假设」原则：
///   · payload 不做 schema 校验，原样作为 String 存入 AppEvent::PluginUserEvent
///   · 事件名也不做限制（允许插件任意命名，如 "gain-changed"、"my-custom-event"）
///   · 返回值：0 = ok；1 = EVENT_TX 未初始化；2 = 内存读取/UTF-8 失败；3 = 发送通道已满/断开
fn add_host_imports(linker: &mut Linker<PluginStoreData>) -> Result<(), PluginError> {
    use phonon_core::AppEvent;
    use wasmtime::Extern;

    let to_mod_err =
        |e| -> PluginError { PluginError::Module(format!("host import link error: {}", e)) };

    // host.emit_event(name_ptr:i32, name_len:i32, payload_ptr:i32, payload_len:i32) -> i32
    linker
        .func_wrap(
            "host",
            "emit_event",
            |mut caller: Caller<'_, PluginStoreData>,
             name_ptr: i32,
             name_len: i32,
             payload_ptr: i32,
             payload_len: i32|
             -> i32 {
                let memory = match caller.get_export("memory") {
                    Some(Extern::Memory(m)) => m,
                    None => return 2,
                    Some(_) => return 2, // "memory" 导出存在但不是 Memory 类型（Func/Global/Table 等）
                };
                // 读取 Wasm 内存中的字符串（返回 None → 失败）
                let read_str = |ptr: i32, len: i32| -> Option<String> {
                    if ptr < 0 || len < 0 {
                        return None;
                    }
                    let mut buf = vec![0u8; len as usize];
                    memory.read(&caller, ptr as usize, &mut buf).ok()?;
                    String::from_utf8(buf).ok()
                };
                let name = match read_str(name_ptr, name_len) {
                    Some(s) => s,
                    None => return 2,
                };
                let payload = match read_str(payload_ptr, payload_len) {
                    Some(s) => s,
                    None => return 2,
                };

                // 经宿主回调转发至 EVENT_TX → Tauri 的 app-event 通道 → 前端 listen("app-event")
                match phonon_core::EVENT_TX.get() {
                    Some(tx) => match tx.send(AppEvent::PluginUserEvent { name, payload }) {
                        Ok(_) => 0,
                        Err(_send_err) => 3, // receiver 全部 drop / 通道断开
                    },
                    None => 1, // EVENT_TX 尚未初始化（通常出现在启动前极短时间）
                }
            },
        )
        .map_err(to_mod_err)?;

    // ── calibration-fir pull model (Task 5.2) ────────────────────────
    // host.get_plugin_config_version(name_ptr:i32, name_len:i32) -> i64
    //   Returns the current monotonic version of `name`'s JSON blob; 0
    //   means no config exists. Version is written by `set_plugin_config`
    //   in PluginRuntime, and the plugin polls on every frame so a new
    //   FIR tap set takes effect within a single 5 ms callback.
    //
    // host.read_plugin_config(name_ptr:i32, name_len:i32, dst_ptr:i32, dst_len:i32) -> i32
    //   If dst_len == 0: returns size hint (bytes needed to hold the JSON
    //   text). Otherwise: copies JSON bytes into the wasm linear memory at
    //   [dst_ptr, dst_ptr+dst_len). Returns bytes copied on success, 0 on
    //   name-not-found, negative on buffer-too-small (abs(ret) is needed
    //   size), -2 on memory read/write errors from wasm.
    linker
        .func_wrap(
            "host",
            "get_plugin_config_version",
            |mut caller: Caller<'_, PluginStoreData>, name_ptr: i32, name_len: i32| -> i64 {
                let memory = match caller.get_export("memory") {
                    Some(Extern::Memory(m)) => m,
                    None => return 0,
                    Some(_) => return 0,
                };
                let name = match read_utf8(&memory, &caller.as_context(), name_ptr, name_len) {
                    Some(s) => s,
                    None => return 0,
                };
                let data = caller.data();
                match Weak::upgrade(&data.config_versions) {
                    Some(arc) => match arc.read() {
                        Ok(guard) => guard.get(&name).copied().unwrap_or(0) as i64,
                        Err(_) => 0,
                    },
                    None => 0, // PluginRuntime dropped, give up
                }
            },
        )
        .map_err(to_mod_err)?;

    linker
        .func_wrap(
            "host",
            "read_plugin_config",
            |mut caller: Caller<'_, PluginStoreData>,
             name_ptr: i32,
             name_len: i32,
             dst_ptr: i32,
             dst_len: i32|
             -> i32 {
                let memory = match caller.get_export("memory") {
                    Some(Extern::Memory(m)) => m,
                    None => return -2,
                    Some(_) => return -2,
                };
                let name = match read_utf8(&memory, &caller.as_context(), name_ptr, name_len) {
                    Some(s) => s,
                    None => return -2,
                };
                let config_json = match Weak::upgrade(&caller.data().plugin_configs) {
                    Some(arc) => match arc.lock() {
                        Ok(guard) => guard.get(&name).cloned().unwrap_or_default(),
                        Err(_) => return 0,
                    },
                    None => return 0,
                };
                if dst_len == 0 {
                    // Size query (see calibration-fir/src/lib.rs::pull_config).
                    return config_json.len() as i32;
                }
                let dst_len_us = dst_len as usize;
                if dst_ptr < 0 || (dst_ptr as usize).checked_add(dst_len_us).is_none() {
                    return -2;
                }
                if (dst_len_us) < config_json.len() {
                    // Buffer too small: caller allocates again.
                    let neg_size = -(config_json.len() as i32);
                    return neg_size;
                }
                match memory.write(&mut caller, dst_ptr as usize, config_json.as_bytes()) {
                    Ok(()) => config_json.len() as i32,
                    Err(_) => -2,
                }
            },
        )
        .map_err(to_mod_err)?;

    Ok(())
}

/// The WASM plugin runtime.
pub struct PluginRuntime {
    engine: Arc<Engine>,
    plugins: Arc<Mutex<Vec<PluginInfo>>>,
    plugin_dir: Arc<RwLock<PathBuf>>,
    /// Additional directories to scan for .wasm files (e.g., project-root plugins/).
    extra_scan_dirs: Arc<Mutex<Vec<PathBuf>>>,
    resource_limits: Arc<Mutex<PluginResourceLimits>>,
    /// Maximum time a single plugin function call may take (milliseconds).
    call_timeout_ms: AtomicU64,
    /// Loaded plugin instances keyed by plugin name.
    loaded: Arc<Mutex<HashMap<String, LoadedWasm>>>,
    /// Per-plugin JSON configuration strings.
    plugin_configs: Arc<Mutex<HashMap<String, String>>>,
    /// Monotonic versions of each plugin's config blob. Written by
    /// `set_plugin_config`, read by both AppState helpers and (through
    /// a weak Arc clone held in StoreLimits) the Wasm host import
    /// `host.get_plugin_config_version`. Using RwLock keeps the hot
    /// `process_frame` path read-only shared with other plugins.
    config_versions: Arc<RwLock<HashMap<String, u64>>>,
    /// Ensures the engine's epoch-ticker thread starts exactly once.
    /// `Arc` so per-call closures can capture a handle to it.
    epoch_ticker: Arc<std::sync::Once>,
}

impl PluginRuntime {
    /// Create a new plugin runtime.
    pub fn new(plugin_dir: impl Into<PathBuf>) -> Result<Self, PluginError> {
        let mut config = Config::default();
        config.wasm_backtrace_details(wasmtime::WasmBacktraceDetails::Enable);
        config.wasm_multi_memory(true);
        config.epoch_interruption(true);

        let engine = Engine::new(&config).map_err(|e| PluginError::Engine(e.to_string()))?;
        let default_limits = PluginResourceLimits::default();
        let timeout_ms = default_limits.max_call_timeout_ms;

        Ok(Self {
            engine: Arc::new(engine),
            plugins: Arc::new(Mutex::new(Vec::new())),
            plugin_dir: Arc::new(RwLock::new(plugin_dir.into())),
            extra_scan_dirs: Arc::new(Mutex::new(Vec::new())),
            resource_limits: Arc::new(Mutex::new(default_limits)),
            call_timeout_ms: AtomicU64::new(timeout_ms),
            loaded: Arc::new(Mutex::new(HashMap::new())),
            plugin_configs: Arc::new(Mutex::new(HashMap::new())),
            config_versions: Arc::new(RwLock::new(HashMap::new())),
            epoch_ticker: Arc::new(std::sync::Once::new()),
        })
    }

    /// Timeout for a plugin call, expressed in epoch ticks (the ticker
    /// thread bumps the engine epoch every `EPOCH_TICK_MS` milliseconds).
    fn deadline_ticks(timeout: Duration) -> u64 {
        (timeout.as_millis() as u64)
            .div_ceil(EPOCH_TICK_MS)
            .max(1)
    }

    /// Ensure this runtime's engine ticker is running (see the free function).
    fn ensure_epoch_ticker(&self) {
        ensure_epoch_ticker_started(&self.engine, &self.epoch_ticker);
    }

    /// Recursively scan for .wasm files in a directory and its subdirectories.
    fn scan_wasm_files(
        &self,
        dir: &std::path::Path,
        results: &mut Vec<std::path::PathBuf>,
    ) -> Result<(), PluginError> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                // Skip build output directories to avoid scanning
                // target/ artifacts (which contain duplicate .wasm files)
                if path.file_name().is_some_and(|n| n == "target") {
                    continue;
                }
                self.scan_wasm_files(&path, results)?;
            } else if path.extension().is_some_and(|ext| ext == "wasm") {
                results.push(path);
            }
        }
        Ok(())
    }

    /// Scan the plugin directory for .wasm files and detect plugin types.
    /// Preserves loaded plugins even if their files are not in the directory,
    /// so that DSP chain processors remain visible in the plugin list.
    /// Also scans `extra_scan_dirs` so that plugins placed in other directories
    /// (e.g. project-root plugins/) are discovered.
    pub fn scan(&self) -> Result<Vec<PluginInfo>, PluginError> {
        let mut plugins = self.plugins.lock().unwrap();
        plugins.clear();

        // Collect all scan directories: main + extras
        let mut all_dirs = vec![self.plugin_dir.read().unwrap().clone()];
        all_dirs.extend(self.extra_scan_dirs.lock().unwrap().clone());

        // Track seen file stems to avoid duplicates
        let mut seen_names = std::collections::HashSet::new();

        for dir in &all_dirs {
            if !dir.exists() {
                continue;
            }
            let mut wasm_files = Vec::new();
            self.scan_wasm_files(dir, &mut wasm_files)?;

            for path in wasm_files {
                let name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown")
                    .to_string();

                // Skip duplicates across directories
                if !seen_names.insert(name.clone()) {
                    log::debug!("Skipping duplicate plugin: {} ({})", name, path.display());
                    continue;
                }

                let plugin_type = self.detect_plugin_type(&path);
                let file_path_str = path.to_string_lossy().to_string();
                let origin = tag_origin_for_scan(&file_path_str, &name, &self.plugin_dir.read().unwrap());

                let info = PluginInfo {
                    manifest: PluginManifest {
                        name: name.clone(),
                        version: "0.1.0".into(),
                        author: "unknown".into(),
                        description: String::new(),
                        plugin_type,
                        permissions: Vec::new(),
                        min_framework_version: "0.1.0".into(),
                    },
                    state: PluginState::Discovered,
                    file_path: file_path_str,
                    error_message: None,
                    origin,
                };

                log::info!(
                    "Discovered plugin: {} ({:?})",
                    info.manifest.name,
                    plugin_type
                );
                plugins.push(info);
            }
        }

        // Preserve loaded state: plugins in self.loaded are already active
        let loaded = self.loaded.lock().unwrap();
        for info in plugins.iter_mut() {
            if loaded.contains_key(&info.manifest.name) {
                info.state = PluginState::Loaded;
            }
        }

        // Re-add loaded plugins that weren't re-discovered from disk
        // (e.g., files were moved/deleted or directory is wrong — DSP chain still needs them)
        for (name, wasm) in loaded.iter() {
            if !plugins.iter().any(|p| p.manifest.name == *name) {
                let mut info = wasm.info.clone();
                info.state = PluginState::Loaded;
                log::info!("Preserving loaded plugin {} (not found on disk)", name);
                plugins.push(info);
            }
        }
        drop(loaded);

        Ok(plugins.clone())
    }

    /// Detect plugin type by checking the wasm module's exports.
    fn detect_plugin_type(&self, path: &std::path::Path) -> PluginType {
        let wasm_bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                log::warn!(
                    "Failed to read {} for type detection: {}",
                    path.display(),
                    e
                );
                return PluginType::SourceProvider;
            }
        };

        let module = match Module::from_binary(&self.engine, &wasm_bytes) {
            Ok(m) => m,
            Err(e) => {
                log::warn!(
                    "Failed to compile {} for type detection: {}",
                    path.display(),
                    e
                );
                return PluginType::SourceProvider;
            }
        };

        let exports: Vec<String> = module.exports().map(|e| e.name().to_string()).collect();
        log::debug!("Plugin {} exports: {:?}", path.display(), exports);

        // Detect by characteristic exports
        if exports.contains(&"plugin_process".to_string()) {
            PluginType::DspProcessor
        } else if exports.contains(&"plugin_decode".to_string()) {
            PluginType::AudioDecoder
        } else if exports.contains(&"plugin_time_stretch_push".to_string()) {
            PluginType::TimeStretch
        } else {
            PluginType::SourceProvider
        }
    }

    /// Perform a static pre-check on plugin WASM bytes.
    ///
    /// Scans for suspicious byte patterns that indicate non-compliant content
    /// (third-party URLs, eval-like keywords, evasion markers) before the
    /// module is compiled. Returns `Ok(())` if the bytes are clean, or
    /// `Err(PluginError::SuspiciousContent)` with a human-readable reason.
    ///
    /// The check is intentionally conservative — it is only a belt-and-braces
    /// complement to the hard WASM sandbox (no FS/network imports, epoch
    /// interruption, memory limits). It catches plain-text metadata and
    /// accidental leakage of protected keywords in the binary.
    fn static_scan(wasm_bytes: &[u8]) -> Result<(), PluginError> {
        // Helper: case-insensitive ASCII substring search.
        let contains_ci = |haystack: &[u8], needle: &[u8]| -> bool {
            if needle.is_empty() || haystack.len() < needle.len() {
                return false;
            }
            let mut window = haystack.windows(needle.len());
            while let Some(w) = window.next() {
                if w.iter()
                    .zip(needle.iter())
                    .all(|(a, b)| a.eq_ignore_ascii_case(b))
                {
                    return true;
                }
            }
            false
        };

        // 1. Third-party network endpoints — framework is "纯净" (no third-party
        //    commercial sources). We block hard-coded URLs that point to known
        //    streaming / lyrics / music metadata services.
        let blocked_urls: &[&[u8]] = &[
            b"music.163.com",
            b"y.qq.com",
            b"tencentmusic.com",
            b"kugou.com",
            b"kuwo.cn",
            b"ximalaya.com",
            b"douban.fm",
            b"deezer.com",
            b"spotify.com",
            b"itunes.apple.com",
            b"api.douyin.com",
            b"v.douyin.com",
            b"kuaishou.com",
        ];
        for url in blocked_urls {
            if contains_ci(wasm_bytes, url) {
                let s = String::from_utf8_lossy(url);
                return Err(PluginError::SuspiciousContent(format!(
                    "Blocked third-party URL '{}' found in WASM binary (合规：框架不得携带第三方商业音源)",
                    s
                )));
            }
        }

        // 2. Evasion / obfuscation markers — strong signal of hostile intent
        //    in an audio plugin.
        let evasion_markers: &[&[u8]] = &[
            b"eval(",
            b"unescape(",
            b"atob(",
            b"fromCharCode",
            b"Function(",
            b"obfuscated",
            b"base64",
            b"powershell",
            b"cmd.exe",
            b"\\x00\\x00\\x00\\x00", // placeholder for raw shellcode signature
        ];
        for marker in evasion_markers {
            if contains_ci(wasm_bytes, marker) {
                let s = String::from_utf8_lossy(marker);
                return Err(PluginError::SuspiciousContent(format!(
                    "Blocked evasion marker '{}' found in WASM binary",
                    s
                )));
            }
        }

        // 3. Excessive embedded URL count — a legitimate DSP/decoder plugin
        //    should not ship with many URLs. Reject more than a small threshold
        //    to flag data-harvesting / phishing patterns.
        let mut url_matches = 0usize;
        let mut idx = 0usize;
        while idx + 8 <= wasm_bytes.len() {
            let rest = &wasm_bytes[idx..];
            if rest.len() >= 8 && rest[..8].eq_ignore_ascii_case(b"https://") {
                url_matches += 1;
                idx += 8;
            } else if rest.len() >= 7 && rest[..7].eq_ignore_ascii_case(b"http://") {
                url_matches += 1;
                idx += 7;
            } else {
                idx += 1;
            }
        }
        if url_matches > 4 {
            return Err(PluginError::SuspiciousContent(format!(
                "WASM binary contains {} embedded URL markers (threshold: 4) — possible data-harvesting",
                url_matches
            )));
        }

        // 4. Block common web-origin HTTP request markers — plugins run sandboxed
        //    without network by default; plain-text fetch/xhr bodies indicate
        //    attempts to exfiltrate data via later host-bridge manipulation.
        let exfil_markers: &[&[u8]] = &[
            b"XMLHttpRequest",
            b"fetch(",
            b"sendBeacon",
            b"navigator.send",
            b"WebSocket",
        ];
        for marker in exfil_markers {
            if contains_ci(wasm_bytes, marker) {
                let s = String::from_utf8_lossy(marker);
                return Err(PluginError::SuspiciousContent(format!(
                    "Blocked network-exfiltration marker '{}' found in WASM binary \
                     (plugins have no network access by default)",
                    s
                )));
            }
        }

        Ok(())
    }

    /// Load a plugin by its file path.
    pub fn load(&self, file_path: &str) -> Result<(), PluginError> {
        // Builtin 插件由 Phonon 内核管理，不能动态 load/reload/remove（错误码 E5001）。
        if crate::builtin::all()
            .iter()
            .any(|b| b.manifest.name == file_path)
        {
            return Err(PluginError::Static(
                file_path.to_string(),
                "be loaded dynamically (call load_plugin / reload_plugin / remove_plugin)".into(),
            ));
        }
        let path = Path::new(file_path);
        if !path.exists() {
            return Err(PluginError::NotFound(file_path.to_string()));
        }

        let wasm_bytes = std::fs::read(path)?;

        // 合规：静态规则扫描，拒绝可疑内容
        Self::static_scan(&wasm_bytes)?;

        let module = Module::from_binary(&self.engine, &wasm_bytes)
            .map_err(|e| PluginError::Module(format!("Failed to compile module: {}", e)))?;

        let mut store = self.create_store();
        let mut linker = Linker::new(&self.engine);
        add_wasi_stubs(&mut linker)?;
        add_host_imports(&mut linker)?;
        let instance = linker
            .instantiate(&mut store, &module)
            .map_err(|e| PluginError::Module(format!("Failed to instantiate: {}", e)))?;

        // Call plugin_init if the plugin exports it
        if let Ok(init_fn) = instance.get_typed_func::<(), ()>(&mut store, "plugin_init") {
            let _ = init_fn.call(&mut store, ());
            log::info!("Called plugin_init");
        }

        // Get the plugin info from the plugin list
        let plugins = self.plugins.lock().unwrap();
        let plugin_info = plugins
            .iter()
            .find(|p| p.file_path == file_path)
            .cloned()
            .ok_or_else(|| PluginError::NotFound(file_path.to_string()))?;
        let plugin_name = plugin_info.manifest.name.clone();
        drop(plugins);

        // Store the loaded plugin instance with its info
        let mut loaded = self.loaded.lock().unwrap();
        loaded.insert(
            plugin_name.clone(),
            LoadedWasm {
                info: plugin_info.clone(),
                store: Arc::new(Mutex::new(store)),
                instance,
            },
        );

        // Update plugin state
        let mut plugins = self.plugins.lock().unwrap();
        if let Some(info) = plugins.iter_mut().find(|p| p.file_path == file_path) {
            info.state = PluginState::Loaded;
            info.error_message = None;
        }

        log::info!("Loaded plugin: {} ({})", plugin_name, file_path);
        Ok(())
    }

    /// Unload a plugin by its file path.
    pub fn unload(&self, file_path: &str) -> Result<(), PluginError> {
        let mut plugins = self.plugins.lock().unwrap();
        let plugin_name = plugins
            .iter()
            .find(|p| p.file_path == file_path)
            .map(|p| p.manifest.name.clone())
            .ok_or_else(|| PluginError::NotFound(file_path.to_string()))?;

        if let Some(info) = plugins.iter_mut().find(|p| p.file_path == file_path) {
            if info.state == PluginState::Running {
                return Err(PluginError::InUse(info.manifest.name.clone()));
            }
            info.state = PluginState::Disabled;
        }
        drop(plugins);

        // Remove the loaded instance
        let mut loaded = self.loaded.lock().unwrap();
        loaded.remove(&plugin_name);

        log::info!("Unloaded plugin: {}", plugin_name);
        Ok(())
    }

    /// Remove a plugin file from disk.
    pub fn remove(&self, file_path: &str) -> Result<(), PluginError> {
        // Builtin 插件由 Phonon 内核管理，不能删除
        if crate::builtin::all()
            .iter()
            .any(|b| b.manifest.name == file_path)
        {
            return Err(PluginError::Static(
                file_path.to_string(),
                "be removed from disk. Built-in plugins ship with Phonon and cannot be deleted."
                    .into(),
            ));
        }
        // Unload first if loaded
        let _ = self.unload(file_path);
        // Remove from plugins list
        let mut plugins = self.plugins.lock().unwrap();
        plugins.retain(|p| p.file_path != file_path);
        drop(plugins);
        // Delete the file
        let path = std::path::Path::new(file_path);
        if path.exists() {
            std::fs::remove_file(path)
                .map_err(|e| PluginError::Module(format!("Failed to remove: {}", e)))?;
        }
        log::info!("Removed plugin: {}", file_path);
        Ok(())
    }

    /// Get all discovered plugins.
    ///
    /// 返回顺序按三层插件架构（Spec §1.3）：
    ///   1. 前 5 位：Builtin（来自 `builtin::all()` 静态注册表）
    ///   2. 中间：WasmExternal（用户 plugins 目录；用户目录优先于示例目录，冲突时 External 保留 Example 丢弃）
    ///   3. 末尾：WasmExample（仅开发模式可见；名字命中 `EXAMPLE_NAMES`）
    pub fn list_plugins(&self) -> Vec<PluginInfo> {
        use crate::builtin;

        // 先合并 Wasm 两层（External + Example，冲突解决）
        let wasm = self.plugins.lock().unwrap().clone();
        let mut merged_wasm = merge_plugin_layers(wasm);

        // prepend Builtin（永远在最前，保持 `builtin::all()` 原始顺序）
        // Builtin 状态统一为 Loaded：这个语义代表"模块已被内核加载并注册"，而不是 DSP 链上开关状态
        // （enable_dsp 开关状态由 enable_dsp() 命令单独反映）。
        let mut builtins: Vec<PluginInfo> = builtin::all()
            .iter()
            .map(|b| PluginInfo {
                manifest: b.manifest.clone(),
                state: PluginState::Loaded,
                file_path: format!("builtin:{id}", id = b.id),
                error_message: None,
                origin: PluginOrigin::Builtin,
            })
            .collect();

        builtins.append(&mut merged_wasm);
        builtins
    }

    /// Set resource limits for plugins.
    pub fn set_resource_limits(&self, limits: PluginResourceLimits) {
        *self.resource_limits.lock().unwrap() = limits;
    }

    /// Set the maximum time a single plugin function call may take.
    pub fn set_call_timeout(&self, timeout_ms: u64) {
        self.call_timeout_ms.store(timeout_ms, Ordering::Relaxed);
    }

    /// Get the current call timeout in milliseconds.
    pub fn call_timeout(&self) -> Duration {
        Duration::from_millis(self.call_timeout_ms.load(Ordering::Relaxed))
    }

    /// Get the plugin directory path.
    pub fn plugin_dir(&self) -> PathBuf {
        self.plugin_dir.read().unwrap().clone()
    }

    /// Replace the main plugin directory and re-scan.
    ///
    /// Used when the app discovers the "real" user data directory after
    /// initial construction (e.g. in Tauri's `.setup()` callback where
    /// `app_data_dir()` becomes available).
    ///
    /// Returns an error if the scan fails; the old directory is preserved.
    pub fn set_plugin_dir(&self, dir: impl Into<PathBuf>) -> Result<(), PluginError> {
        let d = dir.into();
        let _ = std::fs::create_dir_all(&d);
        let mut current = self.plugin_dir.write().unwrap();
        *current = d.clone();
        drop(current);
        self.scan().map(|_| ())
    }

    /// Add an additional directory to scan for .wasm plugins.
    /// Plugins placed here will be discovered by `scan()` even if
    /// the main plugin directory is elsewhere.
    pub fn add_scan_dir(&self, dir: impl Into<PathBuf>) {
        let d = dir.into();
        if d.exists() {
            self.extra_scan_dirs.lock().unwrap().push(d);
        }
    }

    /// Get the JSON configuration for a plugin by name.
    pub fn get_plugin_config(&self, name: &str) -> Option<String> {
        self.plugin_configs.lock().unwrap().get(name).cloned()
    }

    /// Set the JSON configuration for a plugin by name.
    pub fn set_plugin_config(&self, name: &str, config: &str) {
        self.plugin_configs
            .lock()
            .unwrap()
            .insert(name.to_string(), config.to_string());
        // Bump the monotonic config version so the Wasm plugin (which
        // polls via `host.get_plugin_config_version`) sees the update.
        self.config_versions
            .write()
            .map(|mut guard| {
                let next = guard.get(name).copied().unwrap_or(0).saturating_add(1);
                guard.insert(name.to_string(), next);
            })
            .ok();
        log::info!("Plugin config updated for: {}", name);
    }

    /// Return the current monotonic version for a plugin's config blob.
    /// `0` = no config ever set for this name. Exposed for AppState.
    pub fn get_plugin_config_version(&self, name: &str) -> u64 {
        self.config_versions
            .read()
            .ok()
            .and_then(|g| g.get(name).copied())
            .unwrap_or(0)
    }

    /// Call a custom command on a loaded WASM plugin.
    ///
    /// The plugin must export `plugin_command(cmd_ptr, cmd_len, input_ptr, input_len) -> i64`.
    /// Return value packs (output_ptr: u32) << 32 | (output_len: u32); ptr=0 means error.
    /// The output is a JSON string allocated in WASM linear memory via `allocate_string`;
    /// the host calls `plugin_free` after reading it.
    ///
    /// This is intended for non-real-time operations (e.g. computation, analysis).
    /// It runs with the same timeout protection as other plugin calls.
    pub fn call_plugin_command(&self, plugin_name: &str, command: &str, input_json: &str) -> Result<String, PluginError> {
        let loaded = self.loaded.lock().unwrap();
        let wasm = loaded
            .get(plugin_name)
            .ok_or_else(|| PluginError::NotFound(plugin_name.to_string()))?;

        let store = wasm.store.clone();
        let instance = wasm.instance.clone();
        let timeout = Duration::from_millis(self.call_timeout_ms.load(Ordering::Relaxed));
        drop(loaded);

        let mut store_guard = store.lock().unwrap();

        // Check that plugin_command export exists
        let cmd_fn = instance
            .get_typed_func::<(i32, i32, i32, i32), i64>(&mut *store_guard, "plugin_command")
            .map_err(|e| PluginError::Module(format!("plugin_command: {}", e)))?;

        let memory = instance
            .get_memory(&mut *store_guard, "memory")
            .ok_or_else(|| PluginError::Module("No memory export".to_string()))?;

        // Calculate total bytes needed: command + input, with some padding
        let cmd_bytes = command.as_bytes();
        let input_bytes = input_json.as_bytes();
        let total_needed = cmd_bytes.len() + input_bytes.len() + 16; // extra padding

        // Grow memory if needed
        let current = memory.data_size(&*store_guard);
        if current < total_needed {
            let needed_pages = ((total_needed - current) / 65536) + 1;
            memory
                .grow(&mut *store_guard, needed_pages as u64)
                .map_err(|e| PluginError::Module(format!("memory grow failed: {}", e)))?;
        }

        // Write command string at offset 0
        let cmd_ptr = 0i32;
        memory
            .write(&mut *store_guard, cmd_ptr as usize, cmd_bytes)
            .map_err(|e| PluginError::Module(format!("write cmd failed: {}", e)))?;

        // Write input JSON right after command
        let input_ptr = cmd_bytes.len() as i32;
        memory
            .write(&mut *store_guard, input_ptr as usize, input_bytes)
            .map_err(|e| PluginError::Module(format!("write input failed: {}", e)))?;

        // Arm this store's timeout via the engine epoch ticker.
        self.ensure_epoch_ticker();
        store_guard.set_epoch_deadline(Self::deadline_ticks(timeout));

        // Call plugin_command(cmd_ptr, cmd_len, input_ptr, input_len)
        let result = cmd_fn.call(
            &mut *store_guard,
            (cmd_ptr, cmd_bytes.len() as i32, input_ptr, input_bytes.len() as i32),
        );

        let packed = match result {
            Ok(v) => v,
            Err(e) => {
                return Err(PluginError::Module(format!(
                    "plugin_command failed: {}",
                    e
                )));
            }
        };

        // Unpack return value: upper 32 bits = ptr, lower 32 bits = len
        let output_ptr = ((packed >> 32) & 0xFFFFFFFF) as i32;
        let output_len = (packed & 0xFFFFFFFF) as i32;

        if output_ptr == 0 || output_len <= 0 {
            return Err(PluginError::Module(
                "plugin_command returned null or empty result".to_string(),
            ));
        }

        // Read output JSON from WASM memory
        let mut output_buf = vec![0u8; output_len as usize];
        memory
            .read(&*store_guard, output_ptr as usize, &mut output_buf)
            .map_err(|e| PluginError::Module(format!("read output failed: {}", e)))?;

        let output = String::from_utf8(output_buf)
            .map_err(|e| PluginError::Module(format!("output not valid UTF-8: {}", e)))?;

        // Call plugin_free to free the output buffer
        if let Ok(free_fn) = instance
            .get_typed_func::<(i32, i32), ()>(&mut *store_guard, "plugin_free")
        {
            let _ = free_fn.call(&mut *store_guard, (output_ptr, output_len));
        }

        drop(store_guard);
        Ok(output)
    }

    /// Reload a plugin by its file path. Unloads the old instance and loads the new one.
    /// Existing DSP processors will continue using the old instance until recreated.
    pub fn reload(&self, file_path: &str) -> Result<(), PluginError> {
        // Builtin 插件由 Phonon 内核管理，不能动态 reload
        if crate::builtin::all()
            .iter()
            .any(|b| b.manifest.name == file_path)
        {
            return Err(PluginError::Static(
                file_path.to_string(),
                "be hot-reloaded. Built-in plugins are statically compiled into the Phonon binary."
                    .into(),
            ));
        }
        let path = Path::new(file_path);
        if !path.exists() {
            return Err(PluginError::NotFound(file_path.to_string()));
        }

        let wasm_bytes = std::fs::read(path)?;

        // 合规：静态规则扫描，拒绝可疑内容
        Self::static_scan(&wasm_bytes)?;

        // Get the plugin info
        let plugins = self.plugins.lock().unwrap();
        let plugin_info = plugins
            .iter()
            .find(|p| p.file_path == file_path)
            .cloned()
            .ok_or_else(|| PluginError::NotFound(file_path.to_string()))?;
        let plugin_name = plugin_info.manifest.name.clone();
        drop(plugins);

        // Remove old loaded instance (existing Arc refs in DSP processors stay valid)
        {
            let mut loaded = self.loaded.lock().unwrap();
            loaded.remove(&plugin_name);
        }

        // Compile the new module
        let module = Module::from_binary(&self.engine, &wasm_bytes)
            .map_err(|e| PluginError::Module(format!("Failed to compile module: {}", e)))?;

        // Create new store and instance
        let mut store = self.create_store();
        let mut linker = Linker::new(&self.engine);
        add_wasi_stubs(&mut linker)?;
        add_host_imports(&mut linker)?;
        let instance = linker
            .instantiate(&mut store, &module)
            .map_err(|e| PluginError::Module(format!("Failed to instantiate: {}", e)))?;

        // Call plugin_init if the plugin exports it
        if let Ok(init_fn) = instance.get_typed_func::<(), ()>(&mut store, "plugin_init") {
            let _ = init_fn.call(&mut store, ());
            log::info!("Called plugin_init on reload");
        }

        // Store the new loaded instance with its info
        let mut loaded = self.loaded.lock().unwrap();
        loaded.insert(
            plugin_name.clone(),
            LoadedWasm {
                info: plugin_info.clone(),
                store: Arc::new(Mutex::new(store)),
                instance,
            },
        );

        // Update plugin state
        let mut plugins = self.plugins.lock().unwrap();
        if let Some(info) = plugins.iter_mut().find(|p| p.file_path == file_path) {
            info.state = PluginState::Loaded;
            info.error_message = None;
        }

        log::info!("Reloaded plugin: {} ({})", plugin_name, file_path);
        Ok(())
    }

    /// Create a DSP process callback for a loaded plugin.
    ///
    /// Returns a closure that writes PCM samples to WASM linear memory,
    /// calls the plugin's `plugin_process` export, and reads back the
    /// processed samples. The closure is `Send` and can be stored in
    /// `WasmDspProcessor`.
    pub fn create_processor(
        &self,
        name: &str,
    ) -> Result<Box<dyn FnMut(&mut [f32], u16, u32) + Send>, PluginError> {
        let loaded = self.loaded.lock().unwrap();
        let wasm = loaded
            .get(name)
            .ok_or_else(|| PluginError::NotFound(name.to_string()))?;

        let store = wasm.store.clone();
        let instance = wasm.instance.clone();
        let engine = self.engine.clone();
        let epoch_ticker = self.epoch_ticker.clone();
        let timeout = Duration::from_millis(self.call_timeout_ms.load(Ordering::Relaxed));

        // Pre-validate: check that plugin_process exists with correct signature
        {
            let mut store_guard = store.lock().unwrap();
            instance
                .get_typed_func::<(i32, i32, i32, i32), ()>(&mut *store_guard, "plugin_process")
                .map_err(|e| PluginError::Module(format!("plugin_process: {}", e)))?;
        }
        drop(loaded);

        Ok(Box::new(
            move |samples: &mut [f32], channels: u16, sample_rate: u32| {
                let mut store_guard = match store.lock() {
                    Ok(g) => g,
                    Err(_) => return,
                };

                let memory = match instance.get_memory(&mut *store_guard, "memory") {
                    Some(m) => m,
                    None => {
                        log::error!("[WASM DSP] No memory export");
                        return;
                    }
                };

                let byte_len = samples.len() * 4;
                // Grow memory if needed (1 page = 64KB)
                let current = memory.data_size(&*store_guard);
                if current < byte_len {
                    let needed_pages = ((byte_len - current) / 65536) + 1;
                    if memory.grow(&mut *store_guard, needed_pages as u64).is_err() {
                        log::error!("[WASM DSP] Failed to grow memory");
                        return;
                    }
                }

                // Write samples to WASM memory at offset 0
                let bytes: &[u8] =
                    unsafe { std::slice::from_raw_parts(samples.as_ptr() as *const u8, byte_len) };
                if memory.write(&mut *store_guard, 0, bytes).is_err() {
                    log::error!("[WASM DSP] Failed to write samples to WASM memory");
                    return;
                }

                // Call plugin_process(data_ptr, len, channels, sample_rate) with timeout
                let process_fn = match instance
                    .get_typed_func::<(i32, i32, i32, i32), ()>(&mut *store_guard, "plugin_process")
                {
                    Ok(f) => f,
                    Err(_) => return,
                };

                // Arm this store's timeout via the engine epoch ticker.
                ensure_epoch_ticker_started(&engine, &epoch_ticker);
                store_guard.set_epoch_deadline(PluginRuntime::deadline_ticks(timeout));

                let result = process_fn.call(
                    &mut *store_guard,
                    (0, samples.len() as i32, channels as i32, sample_rate as i32),
                );

                match result {
                    Ok(()) => {
                        // Read back processed samples from offset 0
                        let data = memory.data(&*store_guard);
                        if data.len() >= byte_len {
                            let processed: &[f32] = unsafe {
                                std::slice::from_raw_parts(
                                    data.as_ptr() as *const f32,
                                    samples.len(),
                                )
                            };
                            samples.copy_from_slice(processed);
                        }
                    }
                    Err(e) => {
                        let msg = e.to_string();
                        if msg.contains("epoch") || msg.contains("interrupt") {
                            log::warn!(
                                "[WASM DSP] Plugin call timed out after {:?} — skipping chunk",
                                timeout
                            );
                        } else {
                            log::error!("[WASM DSP] Plugin call failed: {}", e);
                        }
                    }
                }
            },
        ))
    }

    /// Create a wasmtime Store with resource limits + weak references to
    /// the runtime's per-plugin config + version maps.
    ///
    /// The host imports inside wasm need access to both maps; we carry
    /// them in `StoreLimits::data()` via our own wrapper
    /// (`PluginStoreData` — wasmtime's `Store<T>` allows any user data).
    /// Using `Weak` ensures no accidental Arc-cycle when stores outlive
    /// the `PluginRuntime` they were spawned from.
    fn create_store(&self) -> Store<PluginStoreData> {
        let max_mem = self.resource_limits.lock().unwrap().max_memory;
        let limits = StoreLimitsBuilder::new().memory_size(max_mem).build();
        let data = PluginStoreData {
            limits,
            config_versions: Arc::downgrade(&self.config_versions),
            plugin_configs: Arc::downgrade(&self.plugin_configs),
        };
        let mut store = Store::new(&self.engine, data);
        store.limiter(|state| &mut state.limits);
        store
    }

    /// Create a TimeStretch callback for a loaded plugin.
    ///
    /// Returns a closure that handles:
    /// - `init(speed, channels, sample_rate)` → initialize for new speed
    /// - `push(input_samples)` → push input, get stretched output
    /// - `flush()` → flush remaining output
    ///
    /// The closure is `Send` and can be stored in the engine.
    pub fn create_time_stretcher(
        &self,
        name: &str,
    ) -> Result<
        Box<dyn FnMut(f32, u16, u32, &[f32], &mut Vec<f32>) -> Result<(), String> + Send>,
        PluginError,
    > {
        let loaded = self.loaded.lock().unwrap();
        let wasm = loaded
            .get(name)
            .ok_or_else(|| PluginError::NotFound(name.to_string()))?;

        let store = wasm.store.clone();
        let instance = wasm.instance.clone();
        let engine = self.engine.clone();
        let epoch_ticker = self.epoch_ticker.clone();
        let timeout = Duration::from_millis(self.call_timeout_ms.load(Ordering::Relaxed));

        // Pre-validate required exports
        {
            let mut store_guard = store.lock().unwrap();
            // plugin_time_stretch_init(speed: f32, channels: i32, sample_rate: i32) -> i32
            let _ = instance
                .get_typed_func::<(f32, i32, i32), i32>(
                    &mut *store_guard,
                    "plugin_time_stretch_init",
                )
                .map_err(|e| PluginError::Module(format!("plugin_time_stretch_init: {}", e)))?;
            // plugin_time_stretch_push(input_ptr: i32, input_len: i32, output_ptr: i32, output_cap: i32) -> i32
            let _ = instance
                .get_typed_func::<(i32, i32, i32, i32), i32>(
                    &mut *store_guard,
                    "plugin_time_stretch_push",
                )
                .map_err(|e| PluginError::Module(format!("plugin_time_stretch_push: {}", e)))?;
            // plugin_time_stretch_flush(output_ptr: i32, output_cap: i32) -> i32
            let _ = instance
                .get_typed_func::<(i32, i32), i32>(&mut *store_guard, "plugin_time_stretch_flush")
                .map_err(|e| PluginError::Module(format!("plugin_time_stretch_flush: {}", e)))?;
            // plugin_time_stretch_reset() -> ()
            let _ = instance
                .get_typed_func::<(), ()>(&mut *store_guard, "plugin_time_stretch_reset")
                .map_err(|e| PluginError::Module(format!("plugin_time_stretch_reset: {}", e)))?;
        }
        drop(loaded);

        Ok(Box::new(
            move |speed: f32,
                  channels: u16,
                  sample_rate: u32,
                  input: &[f32],
                  output: &mut Vec<f32>|
                  -> Result<(), String> {
                let mut store_guard = match store.lock() {
                    Ok(g) => g,
                    Err(e) => return Err(e.to_string()),
                };

                let memory = match instance.get_memory(&mut *store_guard, "memory") {
                    Some(m) => m,
                    None => return Err("No memory export".to_string()),
                };

                // 1. Call init
                let init_fn = instance
                    .get_typed_func::<(f32, i32, i32), i32>(
                        &mut *store_guard,
                        "plugin_time_stretch_init",
                    )
                    .map_err(|e| e.to_string())?;

                // Arm this store's timeout via the engine epoch ticker.
                ensure_epoch_ticker_started(&engine, &epoch_ticker);
                store_guard.set_epoch_deadline(PluginRuntime::deadline_ticks(timeout));

                let init_result = init_fn.call(
                    &mut *store_guard,
                    (speed, channels as i32, sample_rate as i32),
                );

                if let Err(e) = init_result {
                    return Err(e.to_string());
                }

                if input.is_empty() {
                    return Ok(());
                }

                // 2. Write input to WASM memory at offset 0
                let input_byte_len = input.len() * 4;
                let current = memory.data_size(&*store_guard);
                if current < input_byte_len {
                    let needed_pages = ((input_byte_len - current) / 65536) + 1;
                    if memory.grow(&mut *store_guard, needed_pages as u64).is_err() {
                        return Err("Failed to grow memory for input".to_string());
                    }
                }

                let input_bytes: &[u8] = unsafe {
                    std::slice::from_raw_parts(input.as_ptr() as *const u8, input_byte_len)
                };
                if memory.write(&mut *store_guard, 0, input_bytes).is_err() {
                    return Err("Failed to write input to WASM memory".to_string());
                }

                // Reserve output space: input.len() * (1.0 / speed) + some headroom
                let output_cap = (input.len() as f32 / speed).ceil() as usize + 1024;
                let output_byte_cap = output_cap * 4;
                let current_out = memory.data_size(&*store_guard);
                if current_out < input_byte_len + output_byte_cap {
                    let needed_pages =
                        ((input_byte_len + output_byte_cap - current_out) / 65536) + 1;
                    if memory.grow(&mut *store_guard, needed_pages as u64).is_err() {
                        return Err("Failed to grow memory for output".to_string());
                    }
                }

                // 3. Call push
                let push_fn = instance
                    .get_typed_func::<(i32, i32, i32, i32), i32>(
                        &mut *store_guard,
                        "plugin_time_stretch_push",
                    )
                    .map_err(|e| e.to_string())?;

                ensure_epoch_ticker_started(&engine, &epoch_ticker);
                store_guard.set_epoch_deadline(PluginRuntime::deadline_ticks(timeout));

                let output_len = push_fn.call(
                    &mut *store_guard,
                    (
                        0,
                        input.len() as i32,
                        (input_byte_len) as i32,
                        output_cap as i32,
                    ),
                );

                let output_len = match output_len {
                    Ok(n) => n as usize,
                    Err(e) => return Err(e.to_string()),
                };

                if output_len > 0 {
                    // Read back output from offset input_byte_len
                    let data = memory.data(&*store_guard);
                    let output_start = input_byte_len;
                    if data.len() >= output_start + output_len * 4 {
                        let output_ptr = data.as_ptr();
                        let output_f32: &[f32] = unsafe {
                            std::slice::from_raw_parts(
                                (output_ptr.wrapping_add(output_start)) as *const f32,
                                output_len,
                            )
                        };
                        output.extend_from_slice(output_f32);
                    }
                }

                Ok(())
            },
        ))
    }

    /// Create a TimeStretchFactory that can be stored in the engine.
    /// When the factory is called with (speed, channels, sample_rate),
    /// it creates a PluginTimeStretcher that wraps the WASM plugin.
    pub fn create_time_stretch_factory(
        &self,
        name: &str,
    ) -> Result<TimeStretchFactory, PluginError> {
        let loaded = self.loaded.lock().unwrap();
        let wasm = loaded
            .get(name)
            .ok_or_else(|| PluginError::NotFound(name.to_string()))?;

        let store = wasm.store.clone();
        let instance = wasm.instance.clone();
        let engine = self.engine.clone();
        let epoch_ticker = self.epoch_ticker.clone();
        let timeout = Duration::from_millis(self.call_timeout_ms.load(Ordering::Relaxed));

        // Pre-validate required exports
        {
            let mut store_guard = store.lock().unwrap();
            let _ = instance
                .get_typed_func::<(f32, i32, i32), i32>(
                    &mut *store_guard,
                    "plugin_time_stretch_init",
                )
                .map_err(|e| PluginError::Module(format!("plugin_time_stretch_init: {}", e)))?;
            let _ = instance
                .get_typed_func::<(i32, i32, i32, i32), i32>(
                    &mut *store_guard,
                    "plugin_time_stretch_push",
                )
                .map_err(|e| PluginError::Module(format!("plugin_time_stretch_push: {}", e)))?;
            let _ = instance
                .get_typed_func::<(i32, i32), i32>(&mut *store_guard, "plugin_time_stretch_flush")
                .map_err(|e| PluginError::Module(format!("plugin_time_stretch_flush: {}", e)))?;
            let _ = instance
                .get_typed_func::<(), ()>(&mut *store_guard, "plugin_time_stretch_reset")
                .map_err(|e| PluginError::Module(format!("plugin_time_stretch_reset: {}", e)))?;
        }
        drop(loaded);

        let name = name.to_string();
        Ok(Arc::new(move |speed, channels, sample_rate| {
            let store2 = store.clone();
            let instance2 = instance;
            let engine2 = engine.clone();
            let epoch_ticker_ts = epoch_ticker.clone();
            let timeout2 = timeout;
            let name_clone = name.clone();

            // Run init once at instance creation time
            {
                let mut store_guard = match store2.lock() {
                    Ok(g) => g,
                    Err(e) => {
                        log::error!("[TimeStretch] Failed to lock store for init: {}", e);
                        return Box::new(PluginTimeStretcher::new(
                            Box::new(|_, _| Err("Store poisoned".to_string())),
                            Box::new(|_| Err("Store poisoned".to_string())),
                            Box::new(|| Err("Store poisoned".to_string())),
                            Box::new(|_| Err("Store poisoned".to_string())),
                            speed,
                            channels,
                        )) as Box<dyn TimeStretch>;
                    }
                };

                let init_fn = match instance2.get_typed_func::<(f32, i32, i32), i32>(
                    &mut *store_guard,
                    "plugin_time_stretch_init",
                ) {
                    Ok(f) => f,
                    Err(e) => {
                        log::error!("[TimeStretch] init fn not found: {}", e);
                        return Box::new(PluginTimeStretcher::new(
                            Box::new(|_, _| Err("Init fn not found".to_string())),
                            Box::new(|_| Err("Init fn not found".to_string())),
                            Box::new(|| Err("Init fn not found".to_string())),
                            Box::new(|_| Err("Init fn not found".to_string())),
                            speed,
                            channels,
                        )) as Box<dyn TimeStretch>;
                    }
                };

                ensure_epoch_ticker_started(&engine2, &epoch_ticker_ts);
                store_guard.set_epoch_deadline(Self::deadline_ticks(timeout2));

                let init_result = init_fn.call(
                    &mut *store_guard,
                    (speed, channels as i32, sample_rate as i32),
                );

                if let Err(e) = init_result {
                    let err_msg = e.to_string();
                    log::error!(
                        "[TimeStretch] init call failed for '{}': {}",
                        name_clone,
                        err_msg
                    );
                    let err1 = err_msg.clone();
                    let err2 = err_msg.clone();
                    let err3 = err_msg.clone();
                    let err4 = err_msg;
                    return Box::new(PluginTimeStretcher::new(
                        Box::new(move |_, _| Err(format!("Init failed: {}", err1))),
                        Box::new(move |_| Err(format!("Init failed: {}", err2))),
                        Box::new(move || Err(format!("Init failed: {}", err3))),
                        Box::new(move |_| Err(format!("Init failed: {}", err4))),
                        speed,
                        channels,
                    )) as Box<dyn TimeStretch>;
                }

                log::info!(
                    "[TimeStretch] Initialized plugin '{}' (speed={}, ch={}, sr={})",
                    name_clone,
                    speed,
                    channels,
                    sample_rate
                );
            }

            // Push closure
            let store_push = store2.clone();
            let engine_push = engine2.clone();
            let epoch_ticker_push = epoch_ticker_ts.clone();
            let timeout_push = timeout2;
            let name_push = name_clone.clone();
            let push_fn: Box<dyn FnMut(&[f32], &mut Vec<f32>) -> Result<(), String> + Send> =
                Box::new(
                    move |input: &[f32], output: &mut Vec<f32>| -> Result<(), String> {
                        let mut store_guard = store_push.lock().map_err(|e| e.to_string())?;
                        let memory = instance2
                            .get_memory(&mut *store_guard, "memory")
                            .ok_or_else(|| "No memory export".to_string())?;

                        let input_byte_len = input.len() * 4;
                        let current = memory.data_size(&*store_guard);
                        if current < input_byte_len {
                            let needed_pages = ((input_byte_len - current) / 65536) + 1;
                            memory
                                .grow(&mut *store_guard, needed_pages as u64)
                                .map_err(|_| "Failed to grow memory for input".to_string())?;
                        }

                        let input_bytes: &[u8] = unsafe {
                            std::slice::from_raw_parts(input.as_ptr() as *const u8, input_byte_len)
                        };
                        memory
                            .write(&mut *store_guard, 0, input_bytes)
                            .map_err(|_| "Failed to write input to WASM memory".to_string())?;

                        let output_cap = (input.len() as f32 / speed).ceil() as usize + 1024;
                        let output_byte_cap = output_cap * 4;
                        let current_out = memory.data_size(&*store_guard);
                        if current_out < input_byte_len + output_byte_cap {
                            let needed_pages =
                                ((input_byte_len + output_byte_cap - current_out) / 65536) + 1;
                            memory
                                .grow(&mut *store_guard, needed_pages as u64)
                                .map_err(|_| "Failed to grow memory for output".to_string())?;
                        }

                        let push_fn = instance2
                            .get_typed_func::<(i32, i32, i32, i32), i32>(
                                &mut *store_guard,
                                "plugin_time_stretch_push",
                            )
                            .map_err(|e| e.to_string())?;

                        ensure_epoch_ticker_started(&engine_push, &epoch_ticker_push);
                        store_guard
                            .set_epoch_deadline(Self::deadline_ticks(timeout_push));

                        let output_len = push_fn.call(
                            &mut *store_guard,
                            (
                                0,
                                input.len() as i32,
                                input_byte_len as i32,
                                output_cap as i32,
                            ),
                        );

                        let output_len = output_len.map_err(|e| {
                            log::error!("[TimeStretch] push error in '{}': {}", name_push, e);
                            e.to_string()
                        })? as usize;

                        if output_len > 0 {
                            let data = memory.data(&*store_guard);
                            let output_start = input_byte_len;
                            if data.len() >= output_start + output_len * 4 {
                                let output_ptr = data.as_ptr();
                                let output_f32: &[f32] = unsafe {
                                    std::slice::from_raw_parts(
                                        output_ptr.wrapping_add(output_start) as *const f32,
                                        output_len,
                                    )
                                };
                                output.extend_from_slice(output_f32);
                            }
                        }

                        Ok(())
                    },
                );

            // Flush closure
            let store_flush = store2.clone();
            let engine_flush = engine2.clone();
            let epoch_ticker_flush = epoch_ticker_ts.clone();
            let timeout_flush = timeout2;
            let name_flush = name_clone.clone();
            let flush_fn: Box<dyn FnMut(&mut Vec<f32>) -> Result<(), String> + Send> =
                Box::new(move |output: &mut Vec<f32>| -> Result<(), String> {
                    let mut store_guard = store_flush.lock().map_err(|e| e.to_string())?;
                    let memory = instance2
                        .get_memory(&mut *store_guard, "memory")
                        .ok_or_else(|| "No memory export".to_string())?;

                    let flush_fn = instance2
                        .get_typed_func::<(i32, i32), i32>(
                            &mut *store_guard,
                            "plugin_time_stretch_flush",
                        )
                        .map_err(|e| e.to_string())?;

                    let output_cap = 4096;
                    let output_byte_cap = output_cap * 4;
                    let current_out = memory.data_size(&*store_guard);
                    if current_out < output_byte_cap {
                        let needed_pages = ((output_byte_cap - current_out) / 65536) + 1;
                        memory
                            .grow(&mut *store_guard, needed_pages as u64)
                            .map_err(|_| "Failed to grow memory for flush".to_string())?;
                    }

                    ensure_epoch_ticker_started(&engine_flush, &epoch_ticker_flush);
                    store_guard.set_epoch_deadline(Self::deadline_ticks(timeout_flush));

                    let output_len = flush_fn.call(&mut *store_guard, (0, output_cap as i32));

                    let output_len = output_len.map_err(|e| {
                        log::error!("[TimeStretch] flush error in '{}': {}", name_flush, e);
                        e.to_string()
                    })? as usize;

                    if output_len > 0 {
                        let data = memory.data(&*store_guard);
                        if data.len() >= output_len * 4 {
                            let output_ptr = data.as_ptr();
                            let output_f32: &[f32] = unsafe {
                                std::slice::from_raw_parts(output_ptr as *const f32, output_len)
                            };
                            output.extend_from_slice(output_f32);
                        }
                    }

                    Ok(())
                });

            // Reset closure
            let store_reset = store2.clone();
            let engine_reset = engine2.clone();
            let epoch_ticker_reset = epoch_ticker_ts.clone();
            let timeout_reset = timeout2;
            let name_reset = name_clone.clone();
            let reset_fn: Box<dyn FnMut() -> Result<(), String> + Send> =
                Box::new(move || -> Result<(), String> {
                    let mut store_guard = store_reset.lock().map_err(|e| e.to_string())?;

                    let reset_fn = instance2
                        .get_typed_func::<(), ()>(&mut *store_guard, "plugin_time_stretch_reset")
                        .map_err(|e| e.to_string())?;

                    ensure_epoch_ticker_started(&engine_reset, &epoch_ticker_reset);
                    store_guard.set_epoch_deadline(Self::deadline_ticks(timeout_reset));

                    let result = reset_fn.call(&mut *store_guard, ());

                    result.map_err(|e| {
                        log::error!("[TimeStretch] reset error in '{}': {}", name_reset, e);
                        e.to_string()
                    })?;

                    log::debug!("[TimeStretch] Reset plugin '{}'", name_reset);
                    Ok(())
                });

            // Set speed closure — first try dedicated plugin_time_stretch_set_speed
            // (phase-preserving), fall back to re-init if not available.
            let store_set = store2.clone();
            let engine_set = engine2.clone();
            let epoch_ticker_set = epoch_ticker_ts.clone();
            let timeout_set = timeout2;
            let name_set = name_clone.clone();
            let channels_capture = channels;
            let sample_rate_capture = sample_rate;

            // Check if dedicated set_speed export exists
            let has_set_speed = {
                let sg = store2.lock().ok();
                match sg {
                    Some(mut guard) => instance2
                        .get_typed_func::<f32, i32>(&mut *guard, "plugin_time_stretch_set_speed")
                        .is_ok(),
                    None => false,
                }
            };

            let set_speed_fn: Box<dyn FnMut(f32) -> Result<(), String> + Send> = if has_set_speed {
                // Phase-preserving dedicated set_speed
                Box::new(move |new_speed: f32| -> Result<(), String> {
                    let mut store_guard = store_set.lock().map_err(|e| e.to_string())?;
                    let set_fn = instance2
                        .get_typed_func::<f32, i32>(
                            &mut *store_guard,
                            "plugin_time_stretch_set_speed",
                        )
                        .map_err(|e| e.to_string())?;

                    ensure_epoch_ticker_started(&engine_set, &epoch_ticker_set);
                    store_guard.set_epoch_deadline(Self::deadline_ticks(timeout_set));

                    let result = set_fn.call(&mut *store_guard, new_speed);

                    result.map_err(|e| {
                        log::error!("[TimeStretch] set_speed error in '{}': {}", name_set, e);
                        e.to_string()
                    })?;

                    log::debug!(
                        "[TimeStretch] Set speed of '{}' to {:.2}x (phase-preserving)",
                        name_set,
                        new_speed
                    );
                    Ok(())
                })
            } else {
                // Fallback: re-initialize (loses phase state)
                Box::new(move |new_speed: f32| -> Result<(), String> {
                    let mut store_guard = store_set.lock().map_err(|e| e.to_string())?;
                    let init_fn = instance2
                        .get_typed_func::<(f32, i32, i32), i32>(
                            &mut *store_guard,
                            "plugin_time_stretch_init",
                        )
                        .map_err(|e| e.to_string())?;

                    ensure_epoch_ticker_started(&engine_set, &epoch_ticker_set);
                    store_guard.set_epoch_deadline(Self::deadline_ticks(timeout_set));

                    let result = init_fn.call(
                        &mut *store_guard,
                        (
                            new_speed,
                            channels_capture as i32,
                            sample_rate_capture as i32,
                        ),
                    );

                    result.map_err(|e| {
                        log::error!("[TimeStretch] set_speed error in '{}': {}", name_set, e);
                        e.to_string()
                    })?;

                    log::debug!(
                        "[TimeStretch] Set speed of '{}' to {:.2}x (re-init)",
                        name_set,
                        new_speed
                    );
                    Ok(())
                })
            };

            Box::new(PluginTimeStretcher::new(
                push_fn,
                flush_fn,
                reset_fn,
                set_speed_fn,
                speed,
                channels,
            )) as Box<dyn TimeStretch>
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_static_scan_clean_bytes_pass() {
        // A minimal valid wasm magic + empty content should pass.
        let clean = b"\x00asm\x01\x00\x00\x00";
        assert!(PluginRuntime::static_scan(clean).is_ok());
    }

    #[test]
    fn test_static_scan_rejects_blocked_url() {
        let bad = b"plugin metadata music.163.com/song?id=12345";
        let err = PluginRuntime::static_scan(bad).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("music.163.com"), "msg={}", msg);
        assert!(msg.contains("Suspicious content"));
    }

    #[test]
    fn test_static_scan_rejects_case_insensitive_url() {
        let bad = b"endpoint HTTPS://Y.QQ.COM/openapi";
        assert!(PluginRuntime::static_scan(bad).is_err());
    }

    #[test]
    fn test_static_scan_rejects_eval() {
        let bad = b"runtime hook: eval(\"danger\")";
        let err = PluginRuntime::static_scan(bad).unwrap_err();
        assert!(err.to_string().contains("eval("));
    }

    #[test]
    fn test_static_scan_rejects_obfuscation_markers() {
        let bad = b"obfuscated payload atob(\"aGVsbG8=\")";
        let err = PluginRuntime::static_scan(bad).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("obfuscated") || msg.contains("atob("));
    }

    #[test]
    fn test_static_scan_rejects_exfil_markers() {
        let bad = b"XMLHttpRequest to sendBeacon exfil";
        let err = PluginRuntime::static_scan(bad).unwrap_err();
        assert!(err.to_string().contains("XMLHttpRequest"));
    }

    #[test]
    fn test_static_scan_rejects_too_many_urls() {
        let mut payload = Vec::new();
        for _ in 0..10 {
            payload.extend_from_slice(b"http://example.com/ ");
        }
        let err = PluginRuntime::static_scan(&payload).unwrap_err();
        assert!(err.to_string().contains("URL markers"));
    }

    #[test]
    fn test_static_scan_allows_few_urls() {
        // 4 URLs is the threshold — must pass.
        let mut payload = Vec::new();
        for _ in 0..4 {
            payload.extend_from_slice(b"http://example.com/ ");
        }
        assert!(PluginRuntime::static_scan(&payload).is_ok());
    }

    #[test]
    fn test_static_scan_allows_normal_content_near_keywords() {
        // Legitimate DSP plugin metadata that happens to contain similar ASCII
        // fragments should NOT be rejected unless it actually triggers a rule.
        let ok = b"Phonon DSP plugin v1.0: processes PCM samples with eval() name";
        // "eval(" IS a blocked marker, so this should fail — verify behavior.
        assert!(PluginRuntime::static_scan(ok).is_err());
    }

    #[test]
    fn test_static_scan_allows_normal_plugin_metadata() {
        let ok = b"Phonon plugin: DSP audio processor with f32 sample buffer";
        assert!(PluginRuntime::static_scan(ok).is_ok());
    }

    // ── 三层插件：Origin 打标 + merge 纯函数 + Builtin 操作拒绝单元测试 ──

    fn fake_info(name: &str, origin: PluginOrigin) -> PluginInfo {
        PluginInfo {
            manifest: PluginManifest {
                name: name.into(),
                version: "0.1.0".into(),
                author: "test".into(),
                description: String::new(),
                plugin_type: PluginType::DspProcessor,
                permissions: vec![],
                min_framework_version: "0.1.0".into(),
            },
            state: PluginState::Discovered,
            file_path: format!("/tmp/{name}.wasm"),
            error_message: None,
            origin,
        }
    }

    #[test]
    fn test_tag_origin_user_dir_is_external() {
        // 用户目录下找到的任何 wasm 都标记为 WasmExternal（用户目录优先于示例目录）
        let user_dir = Path::new(r"C:\Users\alice\AppData\Roaming\Phonon\plugins");
        let path = r"C:\Users\alice\AppData\Roaming\Phonon\plugins\gain_processor.wasm";
        let origin = tag_origin_for_scan(path, "gain_processor", user_dir);
        assert_eq!(
            origin,
            PluginOrigin::WasmExternal,
            "用户目录下的示例名被错误标记为 Example"
        );
    }

    #[test]
    #[cfg(debug_assertions)]
    fn test_tag_origin_non_user_dir_example_name_dev() {
        // 非用户目录 + debug + 命中示例白名单 → WasmExample
        // 注意：示例插件目录已删除，此处使用白名单占位名 `_test_example_plugin_1`
        let user_dir = Path::new(r"C:\Users\alice\AppData\Roaming\Phonon\plugins");
        let path = r"D:\Phonon\target\wasm32-wasi\release\_test_example_plugin_1.wasm";
        let origin = tag_origin_for_scan(path, "_test_example_plugin_1", user_dir);
        assert_eq!(origin, PluginOrigin::WasmExample);
    }

    #[test]
    fn test_tag_origin_non_user_dir_unknown_name_external() {
        let user_dir = Path::new(r"C:\Users\alice\AppData\Roaming\Phonon\plugins");
        let path = r"D:\stuff\my-own-thing.wasm";
        let origin = tag_origin_for_scan(path, "my_own_thing", user_dir);
        assert_eq!(origin, PluginOrigin::WasmExternal);
    }

    #[test]
    fn test_merge_layers_external_comes_before_example_when_no_conflict() {
        let plugins = vec![
            fake_info("_test_example_plugin_1", PluginOrigin::WasmExample),
            fake_info("my_dsp", PluginOrigin::WasmExternal),
        ];
        let merged = merge_plugin_layers(plugins);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].manifest.name, "my_dsp");
        assert_eq!(merged[0].origin, PluginOrigin::WasmExternal);
        assert_eq!(merged[1].manifest.name, "_test_example_plugin_1");
        assert_eq!(merged[1].origin, PluginOrigin::WasmExample);
    }

    #[test]
    fn test_merge_layers_name_conflict_user_dir_external_wins() {
        // 用户目录下装了同名 Example（External），应该覆盖 Example 版本
        let plugins = vec![
            fake_info("_test_example_plugin_1", PluginOrigin::WasmExample),
            fake_info("_test_example_plugin_1", PluginOrigin::WasmExternal),
        ];
        let merged = merge_plugin_layers(plugins);
        assert_eq!(
            merged.len(),
            1,
            "冲突时应保留 External 1 条，实际 {} 条",
            merged.len()
        );
        assert_eq!(merged[0].origin, PluginOrigin::WasmExternal);
        assert_eq!(merged[0].manifest.name, "_test_example_plugin_1");
    }

    #[test]
    fn test_merge_layers_drops_example_with_non_whitelist_name() {
        // Example 白名单外的名字就算 origin 打成 WasmExample （如果 scan 阶段出错手动标错了）
        // 也会在 merge 这一步被丢掉，多一道防线
        let plugins = vec![fake_info(
            "not_on_whitelist_at_all",
            PluginOrigin::WasmExample,
        )];
        let merged = merge_plugin_layers(plugins);
        assert!(
            merged.is_empty(),
            "非白名单 Example 应该被过滤，但剩 {} 条",
            merged.len()
        );
    }

    // Builtin 动态操作拒绝（E5001）—— 不需要真实 wasm，先检查守卫分支是否命中
    // 注意：load 函数的 Builtin 守卫放在 `!path.exists()` 之前，因此传 Builtin manifest.name
    // 作为 file_path 会命中守卫，而不是 NotFound。
    #[test]
    fn test_load_builtin_refused_static_e5001() {
        let dir = std::env::temp_dir().join("phonon_rt_load");
        let _ = std::fs::create_dir_all(&dir);
        let rt = PluginRuntime::new(&dir).unwrap();
        let name = "phonon_equalizer";
        let err = rt.load(name).unwrap_err();
        match &err {
            PluginError::Static(n, _) => assert_eq!(n, name),
            other => panic!("期望 PluginError::Static，实际是 {:?}", other),
        }
        let msg = err.to_string();
        assert!(msg.contains("[E5001]"), "错误码缺失: {}", msg);
    }

    #[test]
    fn test_reload_builtin_refused_static_e5001() {
        let dir = std::env::temp_dir().join("phonon_rt_reload");
        let _ = std::fs::create_dir_all(&dir);
        let rt = PluginRuntime::new(&dir).unwrap();
        let err = rt.reload("phonon_surround_sound").unwrap_err();
        match &err {
            PluginError::Static(n, _) => assert_eq!(n, "phonon_surround_sound"),
            other => panic!("期望 PluginError::Static，实际是 {:?}", other),
        }
        assert!(err.to_string().contains("[E5001]"));
    }

    #[test]
    fn test_remove_builtin_refused_static_e5001() {
        let dir = std::env::temp_dir().join("phonon_rt_remove");
        let _ = std::fs::create_dir_all(&dir);
        let rt = PluginRuntime::new(&dir).unwrap();
        let err = rt.remove("phonon_downmix").unwrap_err();
        match &err {
            PluginError::Static(n, _) => assert_eq!(n, "phonon_downmix"),
            other => panic!("期望 PluginError::Static，实际是 {:?}", other),
        }
        assert!(err.to_string().contains("[E5001]"));
    }

    #[test]
    fn test_list_plugins_starts_with_5_builtins_fixed_order() {
        let dir = std::env::temp_dir().join("phonon_rt_list");
        let _ = std::fs::create_dir_all(&dir);
        let rt = PluginRuntime::new(&dir).unwrap();
        let list = rt.list_plugins();
        assert!(
            list.len() >= 5,
            "至少有 5 个 Builtin 插件，实际 {}",
            list.len()
        );
        let expected_ids = [
            "equalizer",
            "smart_effect",
            "surround_sound",
            "downmix",
            "replaygain",
        ];
        for (i, expected) in expected_ids.iter().enumerate() {
            assert_eq!(
                list[i].origin,
                PluginOrigin::Builtin,
                "第 {i} 个不是 Builtin"
            );
            let expected_name = format!("phonon_{expected}");
            assert_eq!(
                list[i].manifest.name, expected_name,
                "第 {i} 个 name 不匹配"
            );
            assert_eq!(
                list[i].state,
                PluginState::Loaded,
                "Builtin 状态应为 Loaded（不反映 DSP enable_dsp 开关）"
            );
            let expected_file_path = format!("builtin:{expected}");
            assert_eq!(
                list[i].file_path, expected_file_path,
                "第 {i} 个 file_path 不匹配"
            );
        }
    }
}
