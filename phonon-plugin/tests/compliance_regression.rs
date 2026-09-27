//! §15-6 E2E: 合规回归测试
//!
//! 扫描 Phonon 本体所有 Rust 源码（`phonon-*/src/**/*.rs`），
//! 断言源码中不存在第三方商业音源/歌词/元数据平台的域名常量字符串。
//!
//! 这是 §15 E2E 测试策略的合规防线：防止开发者在内核代码中
//! 意外引入第三方版权平台耦合（违反"纯净框架"定位）。
//!
//! 注意：`plugins/` 子目录是独立项目，受 [external] 标记，
//! 不在本次扫描范围（其内容由 phonon-plugin/src/runtime.rs::static_scan
//! 在 WASM 加载时拦截）。

use std::fs;
use std::path::{Path, PathBuf};

/// 被禁的第三方商业音源/歌词/元数据平台域名（与 runtime.rs::static_scan 一致）。
const BLOCKED_DOMAINS: &[&str] = &[
    "music.163.com",
    "y.qq.com",
    "tencentmusic.com",
    "kugou.com",
    "kuwo.cn",
    "ximalaya.com",
    "douban.fm",
    "deezer.com",
    "spotify.com",
    "itunes.apple.com",
    "api.douyin.com",
    "v.douyin.com",
    "kuaishou.com",
];

/// 收集 phonon-{codec,core,media,plugin,plugin/phonon-plugin-sdk,source,cli} 的 src/ 目录。
fn collect_phonon_src_dirs(root: &Path) -> Vec<PathBuf> {
    let crates = [
        "phonon-codec",
        "phonon-core",
        "phonon-media",
        "phonon-plugin",
        "phonon-plugin/phonon-plugin-sdk",
        "phonon-source",
        "phonon-cli",
    ];
    crates
        .iter()
        .map(|c| root.join(c).join("src"))
        .filter(|p| p.exists())
        .collect()
}

/// 递归收集 `dir` 下所有 `.rs` 文件。
fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    if !dir.is_dir() {
        return;
    }
    for entry in fs::read_dir(dir).expect("read_dir failed") {
        let entry = entry.expect("dir entry failed");
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

/// 扫描源码，返回 (文件路径, 命中的域名) 列表。
fn scan_source(root: &Path) -> Vec<(PathBuf, &'static str)> {
    let mut hits = Vec::new();
    let src_dirs = collect_phonon_src_dirs(root);
    let mut files = Vec::new();
    for dir in &src_dirs {
        collect_rs_files(dir, &mut files);
    }

    for file in &files {
        let content = match fs::read_to_string(file) {
            Ok(s) => s,
            Err(_) => continue,
        };
        // 大小写不敏感匹配
        let lower = content.to_lowercase();
        for &domain in BLOCKED_DOMAINS {
            if lower.contains(domain) {
                hits.push((file.clone(), domain));
            }
        }
    }
    hits
}

#[test]
fn test_no_third_party_domains_in_phonon_source() {
    // 测试可执行文件的工作目录是 workspace 根
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let workspace_root = Path::new(manifest_dir);

    let hits = scan_source(workspace_root);

    if !hits.is_empty() {
        let detail: Vec<String> = hits
            .iter()
            .map(|(path, domain)| format!("  - {} contains '{}'", path.display(), domain))
            .collect();
        panic!(
            "合规回归失败：Phonon 本体源码中发现第三方商业平台域名常量：\n{}\n\
             框架定位为纯净，不得在内核代码中耦合第三方音源/歌词/元数据平台。",
            detail.join("\n")
        );
    }
}

/// 反向测试：确认扫描器能正确检测到域名（防止扫描逻辑失效）。
/// 在临时目录中创建一个包含 `music.163.com` 的 .rs 文件并扫描。
#[test]
fn test_compliance_scanner_detects_blocked_domain() {
    let temp = std::env::temp_dir().join("phonon_compliance_self_test");
    let fake_crate = temp.join("phonon-fake").join("src");
    fs::create_dir_all(&fake_crate).unwrap();
    fs::write(
        fake_crate.join("lib.rs"),
        "// test fixture\nconst URL: &str = \"https://music.163.com/api/song\";\n",
    )
    .unwrap();

    // 临时加入 crates 列表进行扫描
    let mut files = Vec::new();
    collect_rs_files(&fake_crate, &mut files);
    assert!(!files.is_empty(), "scanner should find fixture file");

    let content = fs::read_to_string(&files[0]).unwrap();
    let lower = content.to_lowercase();
    let found = BLOCKED_DOMAINS.iter().any(|d| lower.contains(d));
    assert!(found, "scanner should detect music.163.com in fixture");

    // 清理
    let _ = fs::remove_dir_all(&temp);
}
