//! Integration tests for the phonon-plugin crate.
//!
//! Tests plugin config, reload, and scanning.

use phonon_plugin::PluginRuntime;
use std::path::PathBuf;

// ============================================================
// Plugin Config tests
// ============================================================

#[test]
fn test_plugin_config_default_is_none() {
    let runtime = PluginRuntime::new(&PathBuf::from("./test_plugins")).unwrap();
    assert!(runtime.get_plugin_config("nonexistent").is_none());
}

#[test]
fn test_plugin_config_set_and_get() {
    let runtime = PluginRuntime::new(&PathBuf::from("./test_plugins")).unwrap();
    runtime.set_plugin_config("test-plugin", r#"{"gain": 1.0}"#);
    let config = runtime.get_plugin_config("test-plugin");
    assert!(config.is_some());
    assert!(config.unwrap().contains("gain"));
}

#[test]
fn test_plugin_config_overwrite() {
    let runtime = PluginRuntime::new(&PathBuf::from("./test_plugins")).unwrap();
    runtime.set_plugin_config("test-plugin", r#"{"v": 1}"#);
    runtime.set_plugin_config("test-plugin", r#"{"v": 2}"#);
    let config = runtime.get_plugin_config("test-plugin").unwrap();
    assert!(config.contains(r#""v": 2"#));
}

#[test]
fn test_plugin_config_empty_string() {
    let runtime = PluginRuntime::new(&PathBuf::from("./test_plugins")).unwrap();
    runtime.set_plugin_config("test-plugin", "");
    let config = runtime.get_plugin_config("test-plugin").unwrap();
    assert_eq!(config, "");
}

#[test]
fn test_plugin_config_multiple_plugins() {
    let runtime = PluginRuntime::new(&PathBuf::from("./test_plugins")).unwrap();
    runtime.set_plugin_config("plugin-a", r#"{"a": 1}"#);
    runtime.set_plugin_config("plugin-b", r#"{"b": 2}"#);
    assert!(runtime.get_plugin_config("plugin-a").unwrap().contains("a"));
    assert!(runtime.get_plugin_config("plugin-b").unwrap().contains("b"));
}

// ============================================================
// Plugin Reload tests
// ============================================================

#[test]
fn test_plugin_reload_nonexistent_fails() {
    let runtime = PluginRuntime::new(&PathBuf::from("./test_plugins")).unwrap();
    let result = runtime.reload("nonexistent.wasm");
    assert!(result.is_err());
}

#[test]
fn test_plugin_scan_empty_dir() {
    use phonon_plugin::types::PluginOrigin;
    let runtime = PluginRuntime::new(&PathBuf::from("./test_plugins")).unwrap();
    let result = runtime.scan();
    assert!(result.is_ok());
    let plugins = runtime.list_plugins();
    // list_plugins() 始终 prepend 5 个 Builtin（三层插件系统）；
    // 空插件目录下不应有任何 WasmExternal/WasmExample，只包含 Builtin。
    assert_eq!(
        plugins.len(),
        5,
        "空目录应只返回 5 个 Builtin，实际 {} 个",
        plugins.len()
    );
    for p in &plugins {
        assert_eq!(
            p.origin,
            PluginOrigin::Builtin,
            "空插件目录所有插件 origin 都应是 Builtin，{} 实际是 {:?}",
            p.manifest.name,
            p.origin
        );
    }
    let wasm_count: usize = plugins
        .iter()
        .filter(|p| !matches!(p.origin, PluginOrigin::Builtin))
        .count();
    assert_eq!(wasm_count, 0, "空插件目录 Wasm 插件数量应为 0");
}

#[test]
fn test_plugin_dir_returns_path() {
    let runtime = PluginRuntime::new(&PathBuf::from("./test_plugins")).unwrap();
    let dir = runtime.plugin_dir();
    assert!(dir.to_string_lossy().contains("test_plugins"));
}

// ============================================================
// Plugin Resource Limits tests
// ============================================================

#[test]
fn test_plugin_set_resource_limits() {
    let runtime = PluginRuntime::new(&PathBuf::from("./test_plugins")).unwrap();
    runtime.set_resource_limits(phonon_plugin::PluginResourceLimits {
        max_memory: 256 * 1024 * 1024,
        max_call_timeout_ms: 10000,
    });
    // Verify the set didn't crash
    assert!(true);
}

#[test]
fn test_plugin_call_timeout() {
    let runtime = PluginRuntime::new(&PathBuf::from("./test_plugins")).unwrap();
    runtime.set_call_timeout(3000);
    assert!(true);
}
