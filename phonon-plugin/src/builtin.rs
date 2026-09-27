//! Builtin Plugin Registry — Phonon 内置插件（Rust 原生 DSP）静态元数据。
//!
//! 这些"插件"的真实 process 逻辑不经过 wasmtime，而是在 phonon-core 的 DSP 链上
//! 作为原生 DspProcessor 实现。本注册表的职责是为 `list_plugins()` 与前端
//! PluginManager 提供与 Wasm 插件同构的元数据视图（manifest + 配置说明）。
//!
//! 对应 Spec §1.2：当前 5 个 Builtin = Equalizer / SmartEffect / Surround / Downmix / ReplayGain。
//! 命名约定：所有 Builtin 的 `manifest.name` 统一使用 `phonon_*` 前缀，避免与第三方
//! WasmExample / WasmExternal 插件撞名。

use crate::types::{PluginManifest, PluginType};
use std::sync::LazyLock;

/// 单个内置插件记录（用于 list_plugins → PluginInfo）。
#[derive(Debug, Clone)]
pub struct BuiltinPlugin {
    /// DSP 命令层的 `enable_dsp(id, enabled)` 标识。
    pub id: &'static str,
    /// 插件 manifest（name 一律使用 `phonon_*` 保留前缀）。
    pub manifest: PluginManifest,
    /// 在 PluginManager 行尾展示的配置说明文案。
    pub config_remarks: &'static str,
}

/// 返回全部内置插件数组；
/// 顺序固定为 `Equalizer → SmartEffect → Surround → Downmix → ReplayGain`
/// （与 DspPanel 上从主到辅的顺序一致，前端 PluginManager 顶部展示顺序直接取此）。
pub fn all() -> &'static [BuiltinPlugin] {
    &*BUILTINS
}

static BUILTINS: LazyLock<[BuiltinPlugin; 5]> = LazyLock::new(|| {
    [
        BuiltinPlugin {
            id: "equalizer",
            manifest: PluginManifest {
                name: "phonon_equalizer".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                author: "Phonon Contributors".into(),
                description: "GEQ 10/15 段 + PEQ 16~20 段 IIR Biquad 均衡器。".into(),
                plugin_type: PluginType::DspProcessor,
                permissions: vec![],
                min_framework_version: env!("CARGO_PKG_VERSION").into(),
            },
            config_remarks:
                "DspPanel → Equalizer：Parametric / Graphic 模式与段数切换 + 前级 Preamp。",
        },
        BuiltinPlugin {
            id: "smart_effect",
            manifest: PluginManifest {
                name: "phonon_smart_effect".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                author: "Phonon Contributors".into(),
                description: "Smart Effect 九模式动态范围控制（含流派 Auto）。".into(),
                plugin_type: PluginType::DspProcessor,
                permissions: vec![],
                min_framework_version: env!("CARGO_PKG_VERSION").into(),
            },
            config_remarks: "DspPanel → Smart Effect：九模式单选 + Auto 流派自动推断。",
        },
        BuiltinPlugin {
            id: "surround_sound",
            manifest: PluginManifest {
                name: "phonon_surround_sound".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                author: "Phonon Contributors".into(),
                description: "Surround Sound 六模式沉浸空间化（耳机/音箱分别优化）。".into(),
                plugin_type: PluginType::DspProcessor,
                permissions: vec![],
                min_framework_version: env!("CARGO_PKG_VERSION").into(),
            },
            config_remarks: "DspPanel → Surround Sound：六模式图标化切换。",
        },
        BuiltinPlugin {
            id: "downmix",
            manifest: PluginManifest {
                name: "phonon_downmix".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                author: "Phonon Contributors".into(),
                description: "5.1/7.1 多声道→立体声下混（中置/环绕/LFE 权重可调）。".into(),
                plugin_type: PluginType::DspProcessor,
                permissions: vec![],
                min_framework_version: env!("CARGO_PKG_VERSION").into(),
            },
            config_remarks: "DspPanel → Downmix：开关 + Center/Surround/LFE 权重滑条。",
        },
        BuiltinPlugin {
            id: "replaygain",
            manifest: PluginManifest {
                name: "phonon_replaygain".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                author: "Phonon Contributors".into(),
                description: "ReplayGain 响度归一化（Off / Track / Album 三模式）。".into(),
                plugin_type: PluginType::DspProcessor,
                permissions: vec![],
                min_framework_version: env!("CARGO_PKG_VERSION").into(),
            },
            config_remarks: "DspPanel → ReplayGain：三模式单选；无 tag 自动 Fallback 到 Off。",
        },
    ]
});

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::PluginOrigin;

    // helper：检查一个 BuiltinPlugin 是否符合所有命名/字段约束
    fn assert_builtin_invariants(b: &BuiltinPlugin) {
        // 1. manifest.name 必须是 phonon_* 保留前缀（§1.3 表格最后一行要求）
        assert!(
            b.manifest.name.starts_with("phonon_"),
            "builtin id={} 其 manifest.name='{}' 必须使用 'phonon_' 前缀",
            b.id,
            b.manifest.name,
        );
        // 2. id 必须存在且非空；manifest.name == "phonon_{id}"
        assert!(!b.id.is_empty(), "id 不能为空");
        assert_eq!(
            b.manifest.name,
            format!("phonon_{}", b.id),
            "manifest.name 与 id 映射规则不匹配: id={}",
            b.id,
        );
        // 3. Builtin 永远不走 Wasm 权限授予流程；permissions 必须为空数组
        assert!(
            b.manifest.permissions.is_empty(),
            "builtin id={} permissions 必须为空",
            b.id,
        );
        // 4. 当前 5 个都是 DSP Processor（如果以后加 SourceProvider/Visualizer 扩展类型这里也放宽）
        assert_eq!(
            b.manifest.plugin_type,
            PluginType::DspProcessor,
            "builtin id={} 当前必须是 DspProcessor",
            b.id,
        );
        // 5. 描述文案不能空
        assert!(
            !b.manifest.description.is_empty(),
            "builtin id={} description 必须有内容",
            b.id,
        );
        // 6. config_remarks 不能空
        assert!(
            !b.config_remarks.is_empty(),
            "builtin id={} config_remarks 必须有内容",
            b.id,
        );
        // 7. version/min_framework_version 不能空（都取 CARGO_PKG_VERSION，正常不会空）
        assert!(!b.manifest.version.is_empty());
        assert!(!b.manifest.min_framework_version.is_empty());
        // 8. author 默认 Phonon Contributors
        assert_eq!(b.manifest.author, "Phonon Contributors");
        // 9. PluginOrigin（这条其实是 runtime 层 list_plugins 会打上的，这里只检查作为类型可用）
        let _ = PluginOrigin::Builtin;
    }

    #[test]
    fn builtin_count_is_5() {
        assert_eq!(all().len(), 5);
    }

    #[test]
    fn builtin_order_matches_spec() {
        // 顺序按 Spec 固定：Equalizer / SmartEffect / Surround / Downmix / ReplayGain
        let ids: Vec<&str> = all().iter().map(|b| b.id).collect();
        assert_eq!(
            ids,
            vec![
                "equalizer",
                "smart_effect",
                "surround_sound",
                "downmix",
                "replaygain",
            ]
        );
    }

    #[test]
    fn each_builtin_satisfies_invariants() {
        for b in all() {
            assert_builtin_invariants(b);
        }
    }

    #[test]
    fn builtin_names_all_distinct() {
        use std::collections::HashSet;
        let mut names = HashSet::new();
        for b in all() {
            assert!(
                names.insert(b.manifest.name.clone()),
                "重复的 manifest.name: {}",
                b.manifest.name
            );
            assert!(names.insert(format!("id:{}", b.id)), "重复的 id: {}", b.id);
        }
    }

    #[test]
    fn builtin_dsp_ids_match_enable_dsp_command_ids() {
        // 这 5 个 id 必须能对得上 enable_dsp(id, enabled) 命令当前使用的 DSP 标识。
        // 历史代码 enable_dsp 使用的 key 为 equalizer / smart_effect / surround_sound / downmix / replaygain
        let mut ids: Vec<&str> = all().iter().map(|b| b.id).collect();
        ids.sort_unstable();
        let mut expected = vec![
            "downmix",
            "equalizer",
            "replaygain",
            "smart_effect",
            "surround_sound",
        ];
        expected.sort_unstable();
        assert_eq!(ids, expected);
    }
}
