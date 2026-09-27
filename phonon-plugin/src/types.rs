//! Plugin type definitions and interfaces.

use serde::{Deserialize, Serialize};

/// Types of plugins supported by the framework.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginType {
    /// Provides audio sources (local files, streaming, etc.).
    SourceProvider,
    /// DSP audio processor (EQ, convolution, etc.).
    DspProcessor,
    /// Audio format decoder.
    AudioDecoder,
    /// Time-stretch / pitch-preserving speed change.
    TimeStretch,
    /// Audio visualizer / analyzer (beat detection, pitch tracking, feature extraction).
    Visualizer,
}

/// Plugin manifest / metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub plugin_type: PluginType,
    /// Required permissions (filesystem, network).
    pub permissions: Vec<PluginPermission>,
    /// Minimum framework version required.
    pub min_framework_version: String,
}

/// Permissions that a WASM plugin can request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginPermission {
    FileSystem,
    Network,
    AudioOutput,
    /// Access to raw PCM audio data for visualization/analysis.
    AudioAnalysis,
}

/// State of a loaded plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginState {
    /// Discovered but not loaded.
    Discovered,
    /// Loaded and ready to use.
    Loaded,
    /// Currently running/active.
    Running,
    /// Failed to load.
    Error,
    /// Disabled by user.
    Disabled,
}

/// 插件来源分类——三层插件系统的核心枚举。
///
/// 与 docs/superpowers/specs/2026-08-16-plugin-three-tiers-design.md §2 一一对应。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PluginOrigin {
    /// Phonon 内置模块（Rust 原生 DSP 等）；静态不可 reload/remove。
    Builtin,
    /// 扩展示例插件：随 workspace/plugins/ 子包，不随安装包发布。
    WasmExample,
    /// 独立扩展插件：外部 Git/第三方分发，用户导入 .wasm。
    WasmExternal,
}

impl Default for PluginOrigin {
    /// 默认视为 WasmExternal（所有旧 PluginInfo 在未补 origin 时按外部插件处理）。
    fn default() -> Self {
        PluginOrigin::WasmExternal
    }
}

/// Information about a plugin in the system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginInfo {
    pub manifest: PluginManifest,
    pub state: PluginState,
    pub file_path: String,
    pub error_message: Option<String>,
    /// 来源分类（见 `PluginOrigin` 文档）。
    #[serde(default)]
    pub origin: PluginOrigin,
}

/// Resource limits for WASM plugin execution.
#[derive(Debug, Clone)]
pub struct PluginResourceLimits {
    /// Maximum memory in bytes (default: 128MB).
    pub max_memory: usize,
    /// Maximum execution time for a single call in milliseconds.
    pub max_call_timeout_ms: u64,
}

impl Default for PluginResourceLimits {
    fn default() -> Self {
        Self {
            max_memory: 128 * 1024 * 1024, // 128 MB
            max_call_timeout_ms: 5000,     // 5 seconds
        }
    }
}

/// Errors that can occur in the plugin system.
#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    #[error("Plugin not found: {0}")]
    NotFound(String),
    #[error("Plugin is in use: {0}")]
    InUse(String),
    #[error("Engine error: {0}")]
    Engine(String),
    #[error("Module error: {0}")]
    Module(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Permission denied: {0}")]
    PermissionDenied(String),
    #[error("Resource limit exceeded: {0}")]
    ResourceLimit(String),
    #[error("Timeout: {0}")]
    Timeout(String),
    /// §14 合规：静态规则扫描发现可疑内容（第三方 URL / eval / 规避关键词等）
    #[error("Suspicious content denied by static scan: {0}")]
    SuspiciousContent(String),
    /// Builtin 插件不允许的动态操作（load/reload/remove）。
    /// 参数 0 = 插件名；参数 1 = 动作描述（例如 "be loaded dynamically"）。
    #[error("[E5001] 内置插件 '{0}' 无法 {1}。Built-in plugins are static and managed by Phonon runtime.")]
    Static(String, String),
}

/// Host functions exposed to WASM plugins.
pub trait PluginHost {
    /// Log a message from the plugin.
    fn log(&self, level: &str, message: &str);

    /// Request file system access (returns true if granted).
    fn request_fs_access(&self, path: &str) -> bool;

    /// Request network access (returns true if granted).
    fn request_network_access(&self, url: &str) -> bool;
}

/// Interface that Source Provider plugins must implement.
pub trait SourceProviderPlugin {
    /// Search for audio files in the given directory.
    fn search(&self, directory: &str) -> Result<Vec<String>, String>;

    /// Get metadata for a specific file.
    fn get_metadata(&self, path: &str) -> Result<String, String>;

    /// Get the plugin name.
    fn name(&self) -> &str;
}

/// Interface that DSP plugins must implement.
pub trait DspProcessorPlugin {
    /// Process a buffer of PCM samples.
    fn process(&self, samples: &[f32], channels: u16, sample_rate: u32)
        -> Result<Vec<f32>, String>;

    /// Get the plugin name.
    fn name(&self) -> &str;

    /// Get additional latency in seconds.
    fn latency(&self) -> f64;
}

/// Interface that Decoder plugins must implement.
pub trait DecoderPlugin {
    /// Decode a file and return PCM samples.
    fn decode(&self, path: &str) -> Result<Vec<f32>, String>;

    /// Get metadata for the decoded file.
    fn metadata(&self) -> Result<String, String>;

    /// Get the plugin name.
    fn name(&self) -> &str;
}

/// Interface that Time-Stretch plugins must implement.
/// These plugins replace the built-in phase vocoder for pitch-preserving
/// speed change. When a TimeStretch plugin is loaded, the engine
/// automatically uses it instead of the built-in stretcher.
pub trait TimeStretchPlugin {
    /// Initialize the stretcher for a given speed, channel count, and sample rate.
    fn init(&mut self, speed: f32, channels: u16, sample_rate: u32) -> Result<(), String>;

    /// Push input samples and get stretched output samples.
    /// Returns the number of output samples produced.
    fn push(&mut self, input: &[f32], output: &mut Vec<f32>) -> Result<(), String>;

    /// Flush remaining samples at end of stream.
    fn flush(&mut self, output: &mut Vec<f32>) -> Result<(), String>;

    /// Reset internal state.
    fn reset(&mut self) -> Result<(), String>;

    /// Get the plugin name.
    fn name(&self) -> &str;
}

// ── Visualizer Plugin Types ───────────────────────────────────

/// High-level audio features extracted from the signal, shared by all
/// visualizers so JS scripts don't have to re-compute them every frame.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioFeatures {
    /// FFT magnitude bands (typically 32–128 bins, log-spaced, 0–1 normalized).
    pub spectrum: Vec<f32>,
    /// Down-mixed time-domain PCM samples for waveform display (typically 512–2048 samples, -1..1).
    pub waveform: Vec<f32>,
    /// Root-mean-square volume (0..1, perceptual-ish).
    pub rms: f32,
    /// Peak sample value in the last block (0..1).
    pub peak: f32,
    /// Spectral centroid in Hz (brightness indicator).
    pub spectral_centroid_hz: f32,
    /// Onset / beat confidence (0..1, spikes on transients).
    pub onset: f32,
    /// True if a beat was detected in this analysis window.
    pub beat: bool,
    /// BPM estimate (best-guess, smoothed over time). 0 if unknown.
    pub bpm: f32,
    /// Chroma / pitch-class profile (12 bins: C, C#, D, …, B). Sum-normalized.
    pub chroma: [f32; 12],
    /// Sample rate of the source audio (info only).
    pub sample_rate: u32,
    /// Channel count of the source audio (info only).
    pub channels: u16,
}

impl Default for AudioFeatures {
    fn default() -> Self {
        Self {
            spectrum: vec![0.0; 32],
            waveform: vec![0.0; 512],
            rms: 0.0,
            peak: 0.0,
            spectral_centroid_hz: 0.0,
            onset: 0.0,
            beat: false,
            bpm: 0.0,
            chroma: [0.0; 12],
            sample_rate: 44100,
            channels: 2,
        }
    }
}

/// Interface that Visualizer / analyzer plugins must implement.
///
/// A Visualizer plugin ingests raw PCM and produces structured
/// `AudioFeatures` (beat detection, pitch tracking, custom
/// descriptors, etc.). The result is pushed to the frontend via
/// events so JS/WebGL scripts can render arbitrary visuals from
/// the same feature set.
///
/// Multiple visualizer plugins may run concurrently; each named
/// slot can override specific fields (e.g. a dedicated beat-tracker
/// plugin supplies `beat` and `bpm`, while another handles chroma).
pub trait VisualizerPlugin {
    /// Initialize or reset internal state for a new audio stream.
    fn init(&mut self, channels: u16, sample_rate: u32) -> Result<(), String>;

    /// Analyze a block of interleaved PCM samples and return
    /// which fields of `AudioFeatures` were updated.
    /// The host merges updates from all active visualizer plugins.
    fn analyze(&mut self, samples: &[f32]) -> Result<AudioFeatures, String>;

    /// Optional: render a pre-baked visual to a raw RGBA pixel buffer.
    /// Most visualizers leave this to the JS side; only WASM-side
    /// renderers (e.g. a raymarched scene) need to implement it.
    fn render_pixels(
        &self,
        _width: u32,
        _height: u32,
        _features: &AudioFeatures,
    ) -> Result<Option<Vec<u8>>, String> {
        Ok(None)
    }

    /// Get the plugin name.
    fn name(&self) -> &str;
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── PluginType ──────────────────────────────────────────────

    #[test]
    fn test_plugin_type_serialize() {
        let json = serde_json::to_string(&PluginType::SourceProvider).unwrap();
        assert_eq!(json, "\"SourceProvider\"");
        let json = serde_json::to_string(&PluginType::DspProcessor).unwrap();
        assert_eq!(json, "\"DspProcessor\"");
        let json = serde_json::to_string(&PluginType::AudioDecoder).unwrap();
        assert_eq!(json, "\"AudioDecoder\"");
    }

    #[test]
    fn test_plugin_type_deserialize() {
        let t: PluginType = serde_json::from_str("\"SourceProvider\"").unwrap();
        assert_eq!(t, PluginType::SourceProvider);
        let t: PluginType = serde_json::from_str("\"DspProcessor\"").unwrap();
        assert_eq!(t, PluginType::DspProcessor);
        let t: PluginType = serde_json::from_str("\"AudioDecoder\"").unwrap();
        assert_eq!(t, PluginType::AudioDecoder);
    }

    #[test]
    fn test_plugin_type_deserialize_unknown() {
        let result: Result<PluginType, _> = serde_json::from_str("\"UnknownType\"");
        assert!(result.is_err());
    }

    #[test]
    fn test_plugin_type_eq() {
        assert_eq!(PluginType::SourceProvider, PluginType::SourceProvider);
        assert_ne!(PluginType::SourceProvider, PluginType::DspProcessor);
        assert_ne!(PluginType::DspProcessor, PluginType::AudioDecoder);
    }

    // ── PluginPermission ────────────────────────────────────────

    #[test]
    fn test_plugin_permission_serialize() {
        assert_eq!(
            serde_json::to_string(&PluginPermission::FileSystem).unwrap(),
            "\"FileSystem\""
        );
        assert_eq!(
            serde_json::to_string(&PluginPermission::Network).unwrap(),
            "\"Network\""
        );
        assert_eq!(
            serde_json::to_string(&PluginPermission::AudioOutput).unwrap(),
            "\"AudioOutput\""
        );
    }

    #[test]
    fn test_plugin_permission_deserialize() {
        let p: PluginPermission = serde_json::from_str("\"FileSystem\"").unwrap();
        assert_eq!(p, PluginPermission::FileSystem);
        let p: PluginPermission = serde_json::from_str("\"Network\"").unwrap();
        assert_eq!(p, PluginPermission::Network);
        let p: PluginPermission = serde_json::from_str("\"AudioOutput\"").unwrap();
        assert_eq!(p, PluginPermission::AudioOutput);
    }

    // ── PluginState ─────────────────────────────────────────────

    #[test]
    fn test_plugin_state_serialize() {
        assert_eq!(
            serde_json::to_string(&PluginState::Discovered).unwrap(),
            "\"Discovered\""
        );
        assert_eq!(
            serde_json::to_string(&PluginState::Loaded).unwrap(),
            "\"Loaded\""
        );
        assert_eq!(
            serde_json::to_string(&PluginState::Running).unwrap(),
            "\"Running\""
        );
        assert_eq!(
            serde_json::to_string(&PluginState::Error).unwrap(),
            "\"Error\""
        );
        assert_eq!(
            serde_json::to_string(&PluginState::Disabled).unwrap(),
            "\"Disabled\""
        );
    }

    #[test]
    fn test_plugin_state_deserialize() {
        let s: PluginState = serde_json::from_str("\"Discovered\"").unwrap();
        assert_eq!(s, PluginState::Discovered);
        let s: PluginState = serde_json::from_str("\"Loaded\"").unwrap();
        assert_eq!(s, PluginState::Loaded);
        let s: PluginState = serde_json::from_str("\"Running\"").unwrap();
        assert_eq!(s, PluginState::Running);
        let s: PluginState = serde_json::from_str("\"Error\"").unwrap();
        assert_eq!(s, PluginState::Error);
        let s: PluginState = serde_json::from_str("\"Disabled\"").unwrap();
        assert_eq!(s, PluginState::Disabled);
    }

    #[test]
    fn test_plugin_state_copy_clone() {
        let s = PluginState::Running;
        let copied = s;
        assert_eq!(s, copied);
    }

    // ── PluginManifest ──────────────────────────────────────────

    #[test]
    fn test_plugin_manifest_serialize_roundtrip() {
        let manifest = PluginManifest {
            name: "test-plugin".into(),
            version: "1.0.0".into(),
            author: "Tester".into(),
            description: "A test plugin".into(),
            plugin_type: PluginType::DspProcessor,
            permissions: vec![PluginPermission::AudioOutput],
            min_framework_version: "0.1.0".into(),
        };
        let json = serde_json::to_string(&manifest).unwrap();
        let parsed: PluginManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, "test-plugin");
        assert_eq!(parsed.version, "1.0.0");
        assert_eq!(parsed.author, "Tester");
        assert_eq!(parsed.description, "A test plugin");
        assert_eq!(parsed.plugin_type, PluginType::DspProcessor);
        assert_eq!(parsed.permissions.len(), 1);
        assert_eq!(parsed.permissions[0], PluginPermission::AudioOutput);
        assert_eq!(parsed.min_framework_version, "0.1.0");
    }

    #[test]
    fn test_plugin_manifest_empty_permissions() {
        let manifest = PluginManifest {
            name: "bare".into(),
            version: "0.0.1".into(),
            author: "".into(),
            description: "".into(),
            plugin_type: PluginType::SourceProvider,
            permissions: vec![],
            min_framework_version: "0.1.0".into(),
        };
        let json = serde_json::to_string(&manifest).unwrap();
        let parsed: PluginManifest = serde_json::from_str(&json).unwrap();
        assert!(parsed.permissions.is_empty());
    }

    #[test]
    fn test_plugin_manifest_multiple_permissions() {
        let manifest = PluginManifest {
            name: "full".into(),
            version: "2.0.0".into(),
            author: "Dev".into(),
            description: "Full access".into(),
            plugin_type: PluginType::SourceProvider,
            permissions: vec![
                PluginPermission::FileSystem,
                PluginPermission::Network,
                PluginPermission::AudioOutput,
            ],
            min_framework_version: "0.1.0".into(),
        };
        assert_eq!(manifest.permissions.len(), 3);
    }

    // ── PluginInfo ──────────────────────────────────────────────

    #[test]
    fn test_plugin_info_serialize() {
        let info = PluginInfo {
            manifest: PluginManifest {
                name: "test".into(),
                version: "1.0".into(),
                author: "me".into(),
                description: "desc".into(),
                plugin_type: PluginType::AudioDecoder,
                permissions: vec![],
                min_framework_version: "0.1.0".into(),
            },
            state: PluginState::Discovered,
            file_path: "/plugins/test.wasm".into(),
            error_message: None,
            origin: PluginOrigin::WasmExternal,
        };
        let json = serde_json::to_string(&info).unwrap();
        let parsed: PluginInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.manifest.name, "test");
        assert_eq!(parsed.state, PluginState::Discovered);
        assert_eq!(parsed.file_path, "/plugins/test.wasm");
        assert!(parsed.error_message.is_none());
        assert_eq!(parsed.origin, PluginOrigin::WasmExternal);
    }

    #[test]
    fn test_plugin_info_origin_default_external_when_missing() {
        // 老 JSON（没有 origin 字段）→ 反序列化后应该是 WasmExternal，不破坏历史 session。
        let json_no_origin = r#"{
            "manifest":{"name":"old","version":"1","author":"","description":"","plugin_type":"SourceProvider","permissions":[],"min_framework_version":"0"},
            "state":"Discovered",
            "file_path":"/old.wasm",
            "error_message":null
        }"#;
        let info: PluginInfo = serde_json::from_str(json_no_origin).unwrap();
        assert_eq!(info.origin, PluginOrigin::WasmExternal);
        assert_eq!(info.manifest.name, "old");
    }

    #[test]
    fn test_plugin_info_with_error() {
        let info = PluginInfo {
            manifest: PluginManifest {
                name: "fail".into(),
                version: "1.0".into(),
                author: "me".into(),
                description: "".into(),
                plugin_type: PluginType::SourceProvider,
                permissions: vec![],
                min_framework_version: "0.1.0".into(),
            },
            state: PluginState::Error,
            file_path: "/plugins/fail.wasm".into(),
            error_message: Some("Compilation failed".into()),
            origin: PluginOrigin::WasmExample,
        };
        assert_eq!(info.state, PluginState::Error);
        assert_eq!(info.error_message, Some("Compilation failed".into()));
        assert_eq!(info.origin, PluginOrigin::WasmExample);
    }

    // ── PluginResourceLimits ────────────────────────────────────

    #[test]
    fn test_resource_limits_default() {
        let limits = PluginResourceLimits::default();
        assert_eq!(limits.max_memory, 128 * 1024 * 1024);
        assert_eq!(limits.max_call_timeout_ms, 5000);
    }

    #[test]
    fn test_resource_limits_custom() {
        let limits = PluginResourceLimits {
            max_memory: 64 * 1024 * 1024,
            max_call_timeout_ms: 1000,
        };
        assert_eq!(limits.max_memory, 64 * 1024 * 1024);
        assert_eq!(limits.max_call_timeout_ms, 1000);
    }

    #[test]
    fn test_resource_limits_clone() {
        let limits = PluginResourceLimits::default();
        let cloned = limits.clone();
        assert_eq!(cloned.max_memory, limits.max_memory);
        assert_eq!(cloned.max_call_timeout_ms, limits.max_call_timeout_ms);
    }

    // ── PluginHost trait ────────────────────────────────────────

    struct MockHost;
    impl PluginHost for MockHost {
        fn log(&self, _level: &str, _message: &str) {}
        fn request_fs_access(&self, _path: &str) -> bool {
            true
        }
        fn request_network_access(&self, _url: &str) -> bool {
            false
        }
    }

    #[test]
    fn test_plugin_host_trait() {
        let host = MockHost;
        host.log("info", "hello");
        assert!(host.request_fs_access("/any/path"));
        assert!(!host.request_network_access("https://example.com"));
    }

    // ── SourceProviderPlugin trait ──────────────────────────────

    struct MockSourceProvider;
    impl SourceProviderPlugin for MockSourceProvider {
        fn search(&self, _directory: &str) -> Result<Vec<String>, String> {
            Ok(vec!["file1.flac".into(), "file2.mp3".into()])
        }
        fn get_metadata(&self, _path: &str) -> Result<String, String> {
            Ok(r#"{"artist":"Test"}"#.into())
        }
        fn name(&self) -> &str {
            "mock-source"
        }
    }

    #[test]
    fn test_source_provider_trait() {
        let plugin = MockSourceProvider;
        assert_eq!(plugin.name(), "mock-source");
        let files = plugin.search("/music").unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0], "file1.flac");
        let meta = plugin.get_metadata("/music/file1.flac").unwrap();
        assert!(meta.contains("artist"));
    }

    // ── DspProcessorPlugin trait ────────────────────────────────

    struct MockDsp;
    impl DspProcessorPlugin for MockDsp {
        fn process(
            &self,
            samples: &[f32],
            _channels: u16,
            _sample_rate: u32,
        ) -> Result<Vec<f32>, String> {
            // Pass-through with gain
            Ok(samples.iter().map(|s| s * 0.5).collect())
        }
        fn name(&self) -> &str {
            "mock-dsp"
        }
        fn latency(&self) -> f64 {
            0.01
        }
    }

    #[test]
    fn test_dsp_processor_trait() {
        let plugin = MockDsp;
        assert_eq!(plugin.name(), "mock-dsp");
        assert!((plugin.latency() - 0.01).abs() < 1e-9);
        let input = vec![0.5, -0.5, 1.0];
        let output = plugin.process(&input, 1, 44100).unwrap();
        assert_eq!(output.len(), 3);
        assert!((output[0] - 0.25).abs() < 1e-6);
        assert!((output[1] - (-0.25)).abs() < 1e-6);
        assert!((output[2] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn test_dsp_processor_empty_input() {
        let plugin = MockDsp;
        let output = plugin.process(&[], 2, 48000).unwrap();
        assert!(output.is_empty());
    }

    // ── DecoderPlugin trait ─────────────────────────────────────

    struct MockDecoder {
        decoded: Vec<f32>,
    }
    impl DecoderPlugin for MockDecoder {
        fn decode(&self, _path: &str) -> Result<Vec<f32>, String> {
            Ok(self.decoded.clone())
        }
        fn metadata(&self) -> Result<String, String> {
            Ok(r#"{"format":"flac","sample_rate":44100}"#.into())
        }
        fn name(&self) -> &str {
            "mock-decoder"
        }
    }

    #[test]
    fn test_decoder_plugin_trait() {
        let plugin = MockDecoder {
            decoded: vec![0.1, 0.2, 0.3],
        };
        assert_eq!(plugin.name(), "mock-decoder");
        let samples = plugin.decode("/test.flac").unwrap();
        assert_eq!(samples, vec![0.1, 0.2, 0.3]);
        let meta = plugin.metadata().unwrap();
        assert!(meta.contains("flac"));
    }

    #[test]
    fn test_decoder_plugin_empty() {
        let plugin = MockDecoder { decoded: vec![] };
        let samples = plugin.decode("/empty.flac").unwrap();
        assert!(samples.is_empty());
    }

    // ── PluginError ─────────────────────────────────────────────

    #[test]
    fn test_plugin_error_display() {
        let err = PluginError::NotFound("test.wasm".into());
        assert_eq!(err.to_string(), "Plugin not found: test.wasm");

        let err = PluginError::InUse("my-plugin".into());
        assert_eq!(err.to_string(), "Plugin is in use: my-plugin");

        let err = PluginError::Engine("bad config".into());
        assert_eq!(err.to_string(), "Engine error: bad config");

        let err = PluginError::Module("compile failed".into());
        assert_eq!(err.to_string(), "Module error: compile failed");

        let err = PluginError::PermissionDenied("network".into());
        assert_eq!(err.to_string(), "Permission denied: network");

        let err = PluginError::ResourceLimit("memory exceeded".into());
        assert_eq!(err.to_string(), "Resource limit exceeded: memory exceeded");

        let err = PluginError::Timeout("call took too long".into());
        assert_eq!(err.to_string(), "Timeout: call took too long");

        // PluginError::Static — Builtin 操作拒绝（错误码 E5001）
        let err = PluginError::Static("phonon_equalizer".into(), "be loaded dynamically".into());
        let msg = err.to_string();
        assert!(
            msg.contains("[E5001]"),
            "E5001 错误码未出现在 Static: {}",
            msg
        );
        assert!(msg.contains("phonon_equalizer"), "插件名未显示: {}", msg);
        assert!(
            msg.contains("be loaded dynamically"),
            "动作描述未显示: {}",
            msg
        );
        assert!(
            msg.contains("Built-in plugins are static"),
            "英文兜底说明未显示: {}",
            msg
        );
    }

    #[test]
    fn test_plugin_error_from_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file missing");
        let plugin_err: PluginError = io_err.into();
        assert!(plugin_err.to_string().contains("IO error"));
        assert!(plugin_err.to_string().contains("file missing"));
    }

    #[test]
    fn test_plugin_error_debug() {
        let err = PluginError::NotFound("test.wasm".into());
        let debug = format!("{:?}", err);
        assert!(debug.contains("NotFound"));
        assert!(debug.contains("test.wasm"));
    }
}
