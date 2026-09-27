//! Three-Tier Plugin System — Integration Tests
//!
//! 对应 docs/superpowers/specs/2026-08-16-plugin-three-tiers-design.md §5.2。
//! 覆盖 7 条核心路径：Builtin 列表/顺序、origin 分类、合并冲突、
//! Builtin 拒绝操作、config 读写。

use phonon_plugin::builtin;
use phonon_plugin::types::{PluginError, PluginOrigin, PluginState, PluginType};
use phonon_plugin::{merge_plugin_layers, PluginInfo, PluginManifest, PluginRuntime};

// ── 辅助函数 ──────────────────────────────────────────────────

fn fake_manifest(name: &str) -> PluginManifest {
    PluginManifest {
        name: name.into(),
        version: "0.1.0".into(),
        author: "test".into(),
        description: String::new(),
        plugin_type: PluginType::DspProcessor,
        permissions: vec![],
        min_framework_version: "0.1.0".into(),
    }
}

fn fake_info(name: &str, origin: PluginOrigin) -> PluginInfo {
    PluginInfo {
        manifest: fake_manifest(name),
        state: PluginState::Discovered,
        file_path: format!("/tmp/{}.wasm", name),
        error_message: None,
        origin,
    }
}

fn make_runtime() -> PluginRuntime {
    let tmp = std::env::temp_dir().join("phonon_test_plugins");
    let _ = std::fs::create_dir_all(&tmp);
    PluginRuntime::new(&tmp).expect("PluginRuntime::new failed")
}

// ── Test 1: list_plugins 返回 Builtin 在前 5 位，origin = Builtin ──

#[test]
fn test_list_plugins_builtins_first_and_sorted() {
    let rt = make_runtime();
    let list = rt.list_plugins();

    // 至少 5 个 Builtin
    assert!(
        list.len() >= 5,
        "list_plugins should have at least 5 builtins"
    );

    // 前 5 位都是 Builtin，顺序固定
    let expected_ids = [
        "equalizer",
        "smart_effect",
        "surround_sound",
        "downmix",
        "replaygain",
    ];
    for (i, expected_id) in expected_ids.iter().enumerate() {
        assert_eq!(
            list[i].origin,
            PluginOrigin::Builtin,
            "index {} should be Builtin",
            i
        );
        let expected_name = format!("phonon_{}", expected_id);
        assert_eq!(
            list[i].manifest.name, expected_name,
            "index {} name mismatch",
            i
        );
        assert_eq!(
            list[i].file_path,
            format!("builtin:{}", expected_id),
            "file_path should be builtin:{{id}}"
        );
    }
}

// ── Test 2: Builtin 元数据不变量 ──────────────────────────────

#[test]
fn test_builtin_metadata_invariants() {
    let builtins = builtin::all();

    // 数量 = 5
    assert_eq!(builtins.len(), 5, "must have exactly 5 builtins");

    // 所有 name 以 phonon_ 前缀
    for b in builtins {
        assert!(
            b.manifest.name.starts_with("phonon_"),
            "builtin '{}' must use phonon_ prefix",
            b.manifest.name
        );
        // permissions 必须为空（Builtin 不走 Wasm 授权流程）
        assert!(
            b.manifest.permissions.is_empty(),
            "builtin '{}' permissions must be empty",
            b.manifest.name
        );
        // 都是 DspProcessor 类型
        assert_eq!(
            b.manifest.plugin_type,
            PluginType::DspProcessor,
            "builtin '{}' must be DspProcessor",
            b.manifest.name
        );
    }
}

// ── Test 3: merge_plugin_layers — External 覆盖 Example 同名 ──

#[test]
fn test_merge_external_wins_over_example_on_name_conflict() {
    // 使用白名单测试占位名 _test_example_plugin_1 / _test_example_plugin_2
    let external = fake_info("_test_example_plugin_1", PluginOrigin::WasmExternal);
    let example = fake_info("_test_example_plugin_1", PluginOrigin::WasmExample);
    let other_example = fake_info("_test_example_plugin_2", PluginOrigin::WasmExample);

    let merged = merge_plugin_layers(vec![external.clone(), example, other_example.clone()]);

    // External 保留，同名 Example 被丢弃
    let names: Vec<&str> = merged.iter().map(|p| p.manifest.name.as_str()).collect();
    assert!(
        names.contains(&"_test_example_plugin_1"),
        "external _test_example_plugin_1 should survive"
    );
    assert!(
        names.contains(&"_test_example_plugin_2"),
        "non-conflicting example should survive"
    );

    // 不应该有两条 _test_example_plugin_1
    let count_1 = merged
        .iter()
        .filter(|p| p.manifest.name == "_test_example_plugin_1")
        .count();
    assert_eq!(count_1, 1, "duplicate _test_example_plugin_1 after merge");

    // External 的 _test_example_plugin_1 排在 Example 之前
    let idx_1 = merged
        .iter()
        .position(|p| p.manifest.name == "_test_example_plugin_1")
        .unwrap();
    let idx_2 = merged
        .iter()
        .position(|p| p.manifest.name == "_test_example_plugin_2")
        .unwrap();
    assert!(idx_1 < idx_2, "External should come before Example");
}

// ── Test 4: merge_plugin_layers — 无冲突时 External + Example 共存 ──

#[test]
fn test_merge_no_conflict_keeps_both() {
    let ext = fake_info("my_external", PluginOrigin::WasmExternal);
    let ex = fake_info("_test_example_plugin_1", PluginOrigin::WasmExample);

    let merged = merge_plugin_layers(vec![ext, ex]);

    assert_eq!(merged.len(), 2, "both should survive when no conflict");
    assert_eq!(merged[0].origin, PluginOrigin::WasmExternal);
    assert_eq!(merged[1].origin, PluginOrigin::WasmExample);
}

// ── Test 5: load/reload/remove 对 Builtin 名均返回 Static (E5001) ──

#[test]
fn test_builtin_load_refused_with_e5001() {
    let rt = make_runtime();
    let result = rt.load("phonon_equalizer");
    assert!(
        matches!(&result, Err(PluginError::Static(_, _))),
        "load Builtin should return Static error, got: {:?}",
        result
    );
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("E5001"),
        "error message should contain E5001: {}",
        msg
    );
}

#[test]
fn test_builtin_reload_refused_with_e5001() {
    let rt = make_runtime();
    let result = rt.reload("phonon_smart_effect");
    assert!(
        matches!(&result, Err(PluginError::Static(_, _))),
        "reload Builtin should return Static error, got: {:?}",
        result
    );
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("E5001"),
        "error message should contain E5001: {}",
        msg
    );
}

#[test]
fn test_builtin_remove_refused_with_e5001() {
    let rt = make_runtime();
    let result = rt.remove("phonon_downmix");
    assert!(
        matches!(&result, Err(PluginError::Static(_, _))),
        "remove Builtin should return Static error, got: {:?}",
        result
    );
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("E5001"),
        "error message should contain E5001: {}",
        msg
    );
}

// ── Test 6: get/set_plugin_config 对 Wasm 插件 JSON 读写 ──────

#[test]
fn test_plugin_config_roundtrip() {
    let rt = make_runtime();

    // 初始状态无配置
    assert!(rt.get_plugin_config("nonexistent").is_none());

    // 写入配置
    let json = r#"{"mode":"pop","intensity":0.7}"#;
    rt.set_plugin_config("test_plugin", json);

    // 读回应一致
    let result = rt.get_plugin_config("test_plugin");
    assert!(result.is_some(), "config should exist after set");
    assert_eq!(result.unwrap(), json, "config should round-trip exactly");
}

// ── Test 7: list_plugins 无 Wasm 时只返回 5 Builtin ──────────

#[test]
fn test_list_plugins_empty_wasm_returns_only_builtins() {
    let rt = make_runtime();
    let list = rt.list_plugins();

    // 没有 wasm 插件时，只有 5 Builtin
    assert_eq!(
        list.len(),
        5,
        "with no wasm plugins, list should contain exactly 5 builtins"
    );
    // 全部是 Builtin
    assert!(
        list.iter().all(|p| p.origin == PluginOrigin::Builtin),
        "all entries should be Builtin"
    );
}
