//! Core installation logic.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use windows::core::PCWSTR;
use windows::Win32::System::Registry::*;
#[cfg(not(embedded_bundle))]
use windows::Win32::System::LibraryLoader::{
    FindResourceW, GetModuleHandleW, LoadResource, LockResource, SizeofResource,
};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, HWND_BROADCAST};
use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

// ============ Rollback tracker ============

enum RollbackOp {
    DeleteFile(PathBuf),
    RemoveDir(PathBuf),
    DeleteRegKey { hive: HKEY, path: String },
    DeleteRegValue { hive: HKEY, path: String, name: String },
    RestoreRegValue { hive: HKEY, path: String, name: String, original: Vec<u16> },
}

pub struct RollbackTracker {
    ops: Vec<RollbackOp>,
}

impl RollbackTracker {
    fn new() -> Self {
        Self {
            ops: Vec::new(),
        }
    }

    fn track_file(&mut self, path: impl AsRef<Path>) {
        self.ops.push(RollbackOp::DeleteFile(path.as_ref().to_path_buf()));
    }

    fn track_dir(&mut self, path: impl AsRef<Path>) {
        self.ops.push(RollbackOp::RemoveDir(path.as_ref().to_path_buf()));
    }

    fn track_reg_key(&mut self, hive: HKEY, path: &str) {
        self.ops.push(RollbackOp::DeleteRegKey {
            hive,
            path: path.to_string(),
        });
    }

    fn track_reg_value(&mut self, hive: HKEY, path: &str, name: &str) {
        self.ops.push(RollbackOp::DeleteRegValue {
            hive,
            path: path.to_string(),
            name: name.to_string(),
        });
    }

    fn track_reg_value_restore(&mut self, hive: HKEY, path: &str, name: &str, original: Vec<u16>) {
        self.ops.push(RollbackOp::RestoreRegValue {
            hive,
            path: path.to_string(),
            name: name.to_string(),
            original,
        });
    }

    fn rollback(&self) {
        log::warn!("正在回滚安装操作...");
        for op in self.ops.iter().rev() {
            match op {
                RollbackOp::DeleteFile(p) => {
                    let _ = std::fs::remove_file(p);
                }
                RollbackOp::RemoveDir(p) => {
                    let _ = std::fs::remove_dir_all(p);
                }
                RollbackOp::DeleteRegKey { hive, path } => {
                    let path_w = wide(path);
                    unsafe { let _ = RegDeleteKeyW(*hive, PCWSTR(path_w.as_ptr())); }
                }
                RollbackOp::DeleteRegValue { hive, path, name } => {
                    let path_w = wide(path);
                    unsafe {
                        let mut hkey = HKEY::default();
                        let ret = RegOpenKeyExW(*hive, PCWSTR(path_w.as_ptr()), 0, KEY_WRITE, &mut hkey);
                        if ret.is_ok() {
                            let name_w = wide(name);
                            let _ = RegDeleteValueW(hkey, PCWSTR(name_w.as_ptr()));
                            let _ = RegCloseKey(hkey);
                        }
                    }
                }
                RollbackOp::RestoreRegValue { hive, path, name, original } => {
                    let path_w = wide(path);
                    unsafe {
                        let mut hkey = HKEY::default();
                        let ret = RegOpenKeyExW(*hive, PCWSTR(path_w.as_ptr()), 0, KEY_WRITE, &mut hkey);
                        if ret.is_ok() {
                            let name_w = wide(name);
                            let bytes = std::slice::from_raw_parts(
                                original.as_ptr() as *const u8,
                                original.len() * 2,
                            );
                            let _ = RegSetValueExW(
                                hkey,
                                PCWSTR(name_w.as_ptr()),
                                0,
                                REG_SZ,
                                Some(bytes),
                            );
                            let _ = RegCloseKey(hkey);
                        }
                    }
                }
            }
        }
        log::warn!("回滚完成");
    }
}

/// Installation configuration collected from the UI pages.
#[derive(Clone)]
pub struct InstallConfig {
    pub install_dir: String,
    pub per_machine: bool,
    pub create_desktop_shortcut: bool,
    pub create_start_menu: bool,
    pub add_to_path: bool,
    pub associate_files: bool,
    pub launch_after_install: bool,
}

/// Progress event sent from the install thread to the UI.
#[derive(Clone)]
pub enum InstallProgress {
    Overall(f32),
    Step(&'static str),
    Done,
    #[allow(dead_code)]
    Failed(u32),
}

pub fn run_install<F>(config: &InstallConfig, progress_fn: F) -> Result<()>
where
    F: Fn(InstallProgress) + Send + 'static,
{
    let mut tracker = RollbackTracker::new();

    let result = run_install_inner(config, &progress_fn, &mut tracker);

    if result.is_err() {
        tracker.rollback();
        if let Err(e) = &result {
            log::error!("安装失败: {}", e);
        }
    }

    result
}

fn run_install_inner<F>(
    config: &InstallConfig,
    progress_fn: &F,
    tracker: &mut RollbackTracker,
) -> Result<()>
where
    F: Fn(InstallProgress) + Send + 'static,
{
    progress_fn(InstallProgress::Step("正在准备安装..."));
    progress_fn(InstallProgress::Overall(0.02));

    // Verify disk space before proceeding
    if let Some(free) = check_disk_space(&config.install_dir) {
        if free < REQUIRED_SPACE_BYTES {
            return Err(anyhow!(
                "磁盘空间不足，需要 {} MB，可用 {} MB",
                REQUIRED_SPACE_MB,
                free / (1024 * 1024)
            ));
        }
    }

    if is_phonon_running() {
        progress_fn(InstallProgress::Step("正在关闭运行中的 Phonon..."));
        kill_phonon_process();
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    progress_fn(InstallProgress::Overall(0.05));

    progress_fn(InstallProgress::Step("正在创建安装目录..."));
    std::fs::create_dir_all(&config.install_dir)
        .map_err(|e| anyhow!("无法创建安装目录: {}", e))?;
    tracker.track_dir(&config.install_dir);
    progress_fn(InstallProgress::Overall(0.08));

    progress_fn(InstallProgress::Step("正在解压文件..."));
    extract_app_bundle(&config.install_dir, |p| {
        progress_fn(InstallProgress::Overall(0.08 + p * 0.60));
    }, tracker)?;
    progress_fn(InstallProgress::Overall(0.70));

    progress_fn(InstallProgress::Step("正在注册卸载信息..."));
    register_uninstaller(config, tracker)?;
    progress_fn(InstallProgress::Overall(0.95));

    progress_fn(InstallProgress::Step("安装完成"));
    progress_fn(InstallProgress::Overall(1.0));
    progress_fn(InstallProgress::Done);

    Ok(())
}

/// Apply post-install options (shortcuts, PATH) after the user clicks "Finish".
/// Called from the finish page — no rollback tracking needed.
pub fn apply_post_install_options(config: &InstallConfig) -> Result<()> {
    if config.create_desktop_shortcut {
        create_desktop_shortcut(config, &mut RollbackTracker::new())?;
    }
    if config.create_start_menu {
        create_start_menu_shortcut(config, &mut RollbackTracker::new())?;
    }
    if config.add_to_path {
        let _ = add_to_path(config, &mut RollbackTracker::new());
    }
    Ok(())
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

// ============ App bundle extraction ============

#[cfg(embedded_bundle)]
const APP_BUNDLE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/app_bundle.bin"));

/// RT_RCDATA as MAKEINTRESOURCEW(10)
#[cfg(not(embedded_bundle))]
fn rt_rcdata() -> PCWSTR {
    PCWSTR(10usize as *const u16)
}

fn extract_app_bundle<P: Fn(f32)>(
    install_dir: &str,
    progress: P,
    tracker: &mut RollbackTracker,
) -> Result<usize> {
    #[cfg(embedded_bundle)]
    {
        let cursor = std::io::Cursor::new(APP_BUNDLE);
        let mut archive = zip::ZipArchive::new(cursor)
            .map_err(|e| anyhow!("无法打开嵌入的 zip: {}", e))?;
        return extract_zip(&mut archive, install_dir, &progress, tracker);
    }

    #[cfg(not(embedded_bundle))]
    {
        // Try PE resource (RT_RCDATA) first
        if let Some(data) = find_pe_resource() {
            let cursor = std::io::Cursor::new(data);
            if let Ok(mut archive) = zip::ZipArchive::new(cursor) {
                return extract_zip(&mut archive, install_dir, &progress, tracker);
            }
        }

        // Fall back to copying files alongside the installer exe
        let fallback_dir = Path::new(install_dir);
        return copy_standalone_files(fallback_dir, &progress, tracker);
    }
}

#[cfg(not(embedded_bundle))]
fn find_pe_resource() -> Option<Vec<u8>> {
    let hinst = unsafe { GetModuleHandleW(None).ok()? };
    let res_name = wide("APP_BUNDLE");
    let hres = unsafe { FindResourceW(hinst, PCWSTR(res_name.as_ptr()), rt_rcdata()) };
    if hres.0.is_null() {
        return None;
    }
    let size = unsafe { SizeofResource(hinst, hres) } as usize;
    if size == 0 {
        return None;
    }
    let hmem = unsafe { LoadResource(hinst, hres).ok()? };
    let ptr = unsafe { LockResource(hmem) } as *const u8;
    if ptr.is_null() {
        return None;
    }
    Some(unsafe { std::slice::from_raw_parts(ptr, size).to_vec() })
}

fn extract_zip<R: std::io::Read + std::io::Seek, P: Fn(f32)>(
    archive: &mut zip::ZipArchive<R>,
    install_dir: &str,
    progress: &P,
    tracker: &mut RollbackTracker,
) -> Result<usize> {
    let total = archive.len();
    let dest = Path::new(install_dir);

    for i in 0..total {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| anyhow!("读取 zip 条目失败: {}", e))?;

        let entry_name = entry
            .enclosed_name()
            .ok_or_else(|| anyhow!("zip 条目路径非法"))?
            .to_owned();

        let outpath = dest.join(&entry_name);

        if entry.is_dir() {
            std::fs::create_dir_all(&outpath)
                .map_err(|e| anyhow!("创建目录失败 {}: {}", outpath.display(), e))?;
        } else {
            if let Some(parent) = outpath.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| anyhow!("创建父目录失败: {}", e))?;
            }
            let mut outfile = std::fs::File::create(&outpath)
                .map_err(|e| anyhow!("创建文件失败 {}: {}", outpath.display(), e))?;
            std::io::copy(&mut entry, &mut outfile)
                .map_err(|e| anyhow!("写入文件失败: {}", e))?;
            tracker.track_file(&outpath);
        }

        progress(i as f32 / total as f32);
    }

    Ok(total)
}

#[cfg(not(embedded_bundle))]
fn copy_standalone_files<P: Fn(f32)>(
    dest: &Path,
    progress: &P,
    tracker: &mut RollbackTracker,
) -> Result<usize> {
    let exe_dir = std::env::current_exe()
        .map_err(|e| anyhow!("无法获取安装器路径: {}", e))?
        .parent()
        .ok_or_else(|| anyhow!("无效的安装器路径"))?
        .to_path_buf();

    let mut count = 0;
    let entries: Vec<_> = walkdir::WalkDir::new(&exe_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path() != exe_dir.join("phonon-setup.exe"))
        .collect();
    let total = entries.len().max(1);

    for (i, entry) in entries.iter().enumerate() {
        if entry.file_type().is_file() {
            let rel = entry
                .path()
                .strip_prefix(&exe_dir)
                .map_err(|e| anyhow!("路径解析失败: {}", e))?;
            let target = dest.join(rel);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| anyhow!("创建目录失败: {}", e))?;
            }
            std::fs::copy(entry.path(), &target)
                .map_err(|e| anyhow!("复制文件失败 {}: {}", entry.path().display(), e))?;
            tracker.track_file(&target);
            count += 1;
        }
        progress(i as f32 / total as f32);
    }

    Ok(count)
}

// ============ Uninstaller registration ============

fn register_uninstaller(config: &InstallConfig, tracker: &mut RollbackTracker) -> Result<()> {
    let key_path = "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Phonon";
    let hive = if config.per_machine {
        HKEY_LOCAL_MACHINE
    } else {
        HKEY_CURRENT_USER
    };

    tracker.track_reg_key(hive, key_path);

    let key_path_w = wide(key_path);

    unsafe {
        let mut hkey = HKEY::default();
        let ret = RegCreateKeyExW(
            hive,
            PCWSTR(key_path_w.as_ptr()),
            0,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut hkey,
            None,
        );
        if ret.is_err() {
            return Err(anyhow!("RegCreateKeyExW failed: {}", ret.0));
        }

        let install_dir_w = wide(&config.install_dir);
        let uninstall_exe = format!("{}\\uninstall.exe", config.install_dir);
        let uninstall_w = wide(&uninstall_exe);
        let display_name_w = wide("Phonon");
        let version_w = wide(env!("CARGO_PKG_VERSION"));
        let publisher_w = wide("Phonon Contributors");
        let url_w = wide("https://phonon.app");
        let icon_path = format!("{}\\phonon.exe,0", config.install_dir);
        let icon_w = wide(&icon_path);

        set_reg_sz(hkey, "DisplayName", &display_name_w);
        set_reg_sz(hkey, "DisplayVersion", &version_w);
        set_reg_sz(hkey, "Publisher", &publisher_w);
        set_reg_sz(hkey, "InstallLocation", &install_dir_w);
        set_reg_sz(hkey, "UninstallString", &uninstall_w);
        set_reg_sz(hkey, "QuietUninstallString", &uninstall_w);
        set_reg_sz(hkey, "URLInfoAbout", &url_w);
        set_reg_sz(hkey, "DisplayIcon", &icon_w);
        set_reg_dword(hkey, "NoModify", 1);
        set_reg_dword(hkey, "NoRepair", 1);
        set_reg_dword(hkey, "EstimatedSize", 200_000);

        let _ = RegCloseKey(hkey);
    }

    Ok(())
}

// ============ Shortcut creation ============

fn create_start_menu_shortcut(config: &InstallConfig, tracker: &mut RollbackTracker) -> Result<()> {
    let programs_dir = if config.per_machine {
        "C:\\ProgramData\\Microsoft\\Windows\\Start Menu\\Programs\\Phonon".to_string()
    } else {
        let appdata = std::env::var("APPDATA").unwrap_or_default();
        format!("{}\\Microsoft\\Windows\\Start Menu\\Programs\\Phonon", appdata)
    };

    std::fs::create_dir_all(&programs_dir)
        .map_err(|e| anyhow!("创建开始菜单目录失败: {}", e))?;

    let lnk_path = format!("{}\\Phonon.lnk", programs_dir);
    let exe_path = format!("{}\\phonon.exe", config.install_dir);

    tracker.track_file(&lnk_path);
    tracker.track_dir(&programs_dir);

    create_shortcut(&exe_path, &lnk_path, &config.install_dir, "Phonon")
}

fn create_desktop_shortcut(config: &InstallConfig, tracker: &mut RollbackTracker) -> Result<()> {
    let desktop_dir = std::env::var("USERPROFILE")
        .map(|p| format!("{}\\Desktop", p))
        .unwrap_or_else(|_| "C:\\Users\\Public\\Desktop".to_string());

    let lnk_path = format!("{}\\Phonon.lnk", desktop_dir);
    let exe_path = format!("{}\\phonon.exe", config.install_dir);

    tracker.track_file(&lnk_path);

    create_shortcut(&exe_path, &lnk_path, &config.install_dir, "Phonon")
}

fn create_shortcut(
    exe_path: &str,
    lnk_path: &str,
    working_dir: &str,
    description: &str,
) -> Result<()> {
    use windows::core::Interface;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile,
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Shell::IShellLinkW;
    use windows::Win32::UI::Shell::PropertiesSystem::{
        IPropertyStore, PROPERTYKEY,
    };
    use windows::core::PROPVARIANT;

    let clsid_shelllink = windows::core::GUID::from_u128(0x00021401_0000_0000_C000_000000000046);

    // PKEY_AppUserModel_ID = {9F4C2855-9F79-4B39-A8D0-E1D148DEA65F} pid=5
    let pkey_app_user_model_id = PROPERTYKEY {
        fmtid: windows::core::GUID::from_u128(0x9F4C2855_9F79_4B39_A8D0_E1D148DEA65F),
        pid: 5,
    };

    unsafe {
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let need_uninit = hr.is_ok();

        let result = (|| -> Result<()> {
            let shell_link: IShellLinkW = CoCreateInstance(
                &clsid_shelllink,
                None,
                CLSCTX_INPROC_SERVER,
            )
            .map_err(|e| anyhow!("CoCreateInstance failed: {}", e))?;

            let exe_w = wide(exe_path);
            shell_link
                .SetPath(PCWSTR(exe_w.as_ptr()))
                .map_err(|e| anyhow!("SetPath failed: {}", e))?;

            let work_w = wide(working_dir);
            shell_link
                .SetWorkingDirectory(PCWSTR(work_w.as_ptr()))
                .map_err(|e| anyhow!("SetWorkingDirectory failed: {}", e))?;

            let desc_w = wide(description);
            shell_link
                .SetDescription(PCWSTR(desc_w.as_ptr()))
                .map_err(|e| anyhow!("SetDescription failed: {}", e))?;

            let icon_path = wide(exe_path);
            shell_link
                .SetIconLocation(PCWSTR(icon_path.as_ptr()), 0)
                .ok();

            // Set AppUserModelID for taskbar grouping
            let prop_store: IPropertyStore = shell_link
                .cast()
                .map_err(|e| anyhow!("QueryInterface IPropertyStore failed: {}", e))?;
            let prop = PROPVARIANT::from("com.phonon.app");
            prop_store
                .SetValue(&pkey_app_user_model_id, &prop)
                .map_err(|e| anyhow!("SetValue AppUserModelID failed: {}", e))?;
            let _ = prop_store.Commit();

            let persist: IPersistFile = shell_link
                .cast()
                .map_err(|e| anyhow!("QueryInterface IPersistFile failed: {}", e))?;

            let lnk_w = wide(lnk_path);
            persist
                .Save(PCWSTR(lnk_w.as_ptr()), true)
                .map_err(|e| anyhow!("Save .lnk failed: {}", e))?;

            Ok(())
        })();

        if need_uninit {
            CoUninitialize();
        }

        result?;
    }

    Ok(())
}

// ============ File associations ============

fn register_file_associations(config: &InstallConfig, tracker: &mut RollbackTracker) -> Result<()> {
    let exe_path = format!("{}\\phonon.exe", config.install_dir);
    let hive = if config.per_machine {
        HKEY_LOCAL_MACHINE
    } else {
        HKEY_CURRENT_USER
    };

    let extensions = [
        ".flac", ".mp3", ".wav", ".ogg", ".opus", ".m4a",
        ".aac", ".wma", ".aiff", ".ape", ".dsd", ".dff",
    ];

    let prog_id = "Phonon.AudioFile";
    let prog_id_w = wide(prog_id);
    let exe_w = wide(&exe_path);
    let open_cmd = format!("\"{}\" \"%1\"", exe_path);
    let open_cmd_w = wide(&open_cmd);
    let icon_cmd = format!("\"{}\",0", exe_path);
    let icon_cmd_w = wide(&icon_cmd);

    tracker.track_reg_key(hive, prog_id);

    for ext in &extensions {
        tracker.track_reg_value(hive, ext, "");
        tracker.track_reg_key(hive, &format!("{}\\OpenWithProgids", ext));
    }

    unsafe {
        let mut hkey = HKEY::default();
        let key_path = wide(prog_id);
        let ret = RegCreateKeyExW(
            hive,
            PCWSTR(key_path.as_ptr()),
            0,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut hkey,
            None,
        );
        if ret.is_err() {
            return Err(anyhow!("创建 ProgID 失败: {}", ret.0));
        }
        set_reg_sz(hkey, "", &prog_id_w);
        set_reg_sz(hkey, "FriendlyTypeName", &prog_id_w);

        let mut cmd_key = HKEY::default();
        let cmd_path = wide(&format!("{}\\shell\\open\\command", prog_id));
        let ret = RegCreateKeyExW(
            hive,
            PCWSTR(cmd_path.as_ptr()),
            0,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut cmd_key,
            None,
        );
        if ret.is_ok() {
            set_reg_sz(cmd_key, "", &open_cmd_w);
            let _ = RegCloseKey(cmd_key);
        }

        let mut icon_key = HKEY::default();
        let icon_path = wide(&format!("{}\\DefaultIcon", prog_id));
        let ret = RegCreateKeyExW(
            hive,
            PCWSTR(icon_path.as_ptr()),
            0,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut icon_key,
            None,
        );
        if ret.is_ok() {
            set_reg_sz(icon_key, "", &icon_cmd_w);
            let _ = RegCloseKey(icon_key);
        }

        let _ = RegCloseKey(hkey);

        for ext in &extensions {
            let ext_key = wide(ext);
            let mut ext_hkey = HKEY::default();
            let ret = RegCreateKeyExW(
                hive,
                PCWSTR(ext_key.as_ptr()),
                0,
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_WRITE,
                None,
                &mut ext_hkey,
                None,
            );
            if ret.is_ok() {
                set_reg_sz(ext_hkey, "", &prog_id_w);

                let mut back_key = HKEY::default();
                let back_path = wide(&format!("{}\\OpenWithProgids", ext));
                let ret = RegCreateKeyExW(
                    hive,
                    PCWSTR(back_path.as_ptr()),
                    0,
                    PCWSTR::null(),
                    REG_OPTION_NON_VOLATILE,
                    KEY_WRITE,
                    None,
                    &mut back_key,
                    None,
                );
                if ret.is_ok() {
                    set_reg_sz(back_key, prog_id, &exe_w);
                    let _ = RegCloseKey(back_key);
                }

                let _ = RegCloseKey(ext_hkey);
            }
        }
    }

    // Notify shell to refresh file associations
    use windows::Win32::UI::Shell::{SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNF_IDLIST};
    unsafe {
        let _ = SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None);
    }

    Ok(())
}

// ============ PATH ============

fn add_to_path(config: &InstallConfig, tracker: &mut RollbackTracker) -> Result<()> {
    let hive = if config.per_machine {
        HKEY_LOCAL_MACHINE
    } else {
        HKEY_CURRENT_USER
    };
    let env_path_str = "System\\CurrentVersion\\Environment";
    let env_path = wide(env_path_str);

    unsafe {
        let mut hkey = HKEY::default();
        let ret = RegOpenKeyExW(
            hive,
            PCWSTR(env_path.as_ptr()),
            0,
            KEY_READ | KEY_WRITE,
            &mut hkey,
        );
        if ret.is_err() {
            return Err(anyhow!("打开 Environment 注册表失败: {}", ret.0));
        }

        let mut buf = [0u16; 16384];
        let mut size = (buf.len() * 2) as u32;
        let value_name = wide("Path");
        let ret = RegQueryValueExW(
            hkey,
            PCWSTR(value_name.as_ptr()),
            None,
            None,
            Some(buf.as_mut_ptr() as *mut u8),
            Some(&mut size),
        );
        if ret.is_err() {
            let _ = RegCloseKey(hkey);
            return Err(anyhow!("读取 PATH 失败: {}", ret.0));
        }

        let len = (size / 2) as usize;
        let current = String::from_utf16_lossy(&buf[..len.saturating_sub(1)]);

        if current.split(';').any(|p| p.eq_ignore_ascii_case(&config.install_dir)) {
            let _ = RegCloseKey(hkey);
            return Ok(());
        }

        let original_path: Vec<u16> = buf[..len].to_vec();
        tracker.track_reg_value_restore(hive, env_path_str, "Path", original_path);

        let new_path = if current.ends_with(';') || current.is_empty() {
            format!("{}{}", current, config.install_dir)
        } else {
            format!("{};{}", current, config.install_dir)
        };

        let new_w = wide(&new_path);
        set_reg_sz(hkey, "Path", &new_w);
        let _ = RegCloseKey(hkey);

        broadcast_env_change();
    }

    Ok(())
}

fn broadcast_env_change() {
    use windows::Win32::Foundation::{WPARAM, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        SendMessageTimeoutW, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE,
    };

    let env_str = wide("Environment");
    unsafe {
        let mut result = 0usize;
        let _ = SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            WPARAM(0),
            LPARAM(env_str.as_ptr() as isize),
            SMTO_ABORTIFHUNG,
            5000,
            Some(&mut result),
        );
    }
}

// ============ Registry helpers ============

unsafe fn set_reg_sz(hkey: HKEY, name: &str, value: &[u16]) {
    let name_w = wide(name);
    let bytes = std::slice::from_raw_parts(value.as_ptr() as *const u8, value.len() * 2);
    let _ = RegSetValueExW(hkey, PCWSTR(name_w.as_ptr()), 0, REG_SZ, Some(bytes));
}

unsafe fn set_reg_dword(hkey: HKEY, name: &str, value: u32) {
    let name_w = wide(name);
    let val_bytes = value.to_le_bytes();
    let _ = RegSetValueExW(hkey, PCWSTR(name_w.as_ptr()), 0, REG_DWORD, Some(&val_bytes));
}

// ============ Existing install detection ============

pub fn detect_existing_install() -> Option<String> {
    for hive in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        let key_path = "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Phonon";
        let key_path_w = wide(key_path);

        unsafe {
            let mut hkey = HKEY::default();
            let ret = RegOpenKeyExW(hive, PCWSTR(key_path_w.as_ptr()), 0, KEY_READ, &mut hkey);
            if ret.is_ok() {
                let mut buf = [0u16; 512];
                let mut size = (buf.len() * 2) as u32;
                let name_w = wide("InstallLocation");
                let ret = RegQueryValueExW(
                    hkey,
                    PCWSTR(name_w.as_ptr()),
                    None,
                    None,
                    Some(buf.as_mut_ptr() as *mut u8),
                    Some(&mut size),
                );
                let _ = RegCloseKey(hkey);
                if ret.is_ok() {
                    let len = (size / 2) as usize;
                    if len > 0 && buf[0] != 0 {
                        let s = String::from_utf16_lossy(&buf[..len.saturating_sub(1)]);
                        if !s.is_empty() {
                            return Some(s);
                        }
                    }
                }
            }
        }
    }
    None
}

// ============ Disk space check ============

/// Check if the target drive has enough free space (in bytes).
pub fn check_disk_space(path: &str) -> Option<u64> {
    // Find the drive root from the path (e.g., "C:\foo" → "C:\")
    let root = if path.len() >= 3 {
        &path[..3]
    } else {
        "C:\\"
    };
    let root_w = wide(root);
    let mut free_to_caller: u64 = 0;
    let mut total: u64 = 0;
    let mut free: u64 = 0;
    unsafe {
        if GetDiskFreeSpaceExW(
            PCWSTR(root_w.as_ptr()),
            Some(&mut free_to_caller),
            Some(&mut total),
            Some(&mut free),
        )
        .is_ok()
        {
            Some(free_to_caller)
        } else {
            None
        }
    }
}

/// Required install size in bytes (~200 MB).
pub const REQUIRED_SPACE_MB: u64 = 200;
pub const REQUIRED_SPACE_BYTES: u64 = REQUIRED_SPACE_MB * 1024 * 1024;

// ============ Running process detection ============

pub fn is_phonon_running() -> bool {
    unsafe {
        let class_name = wide("PhononMainWindow");
        let hwnd = FindWindowW(PCWSTR(class_name.as_ptr()), PCWSTR::null());
        if let Ok(hwnd) = hwnd {
            if !hwnd.0.is_null() {
                return true;
            }
        }
    }
    is_process_running_by_name("phonon.exe")
}

fn is_process_running_by_name(name: &str) -> bool {
    use windows::Win32::System::Diagnostics::ToolHelp::*;

    unsafe {
        let snapshot = match CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) {
            Ok(h) => h,
            Err(_) => return false,
        };

        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };

        let mut found = false;
        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                let proc_name = String::from_utf16_lossy(
                    &entry.szExeFile[..entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(0)],
                );
                if proc_name.eq_ignore_ascii_case(name) {
                    found = true;
                    break;
                }
                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }

        let _ = windows::Win32::Foundation::CloseHandle(snapshot);
        found
    }
}

pub fn kill_phonon_process() {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::*;
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_TERMINATE, TerminateProcess};

    unsafe {
        let snapshot = match CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) {
            Ok(h) => h,
            Err(_) => return,
        };

        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };

        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                let proc_name = String::from_utf16_lossy(
                    &entry.szExeFile[..entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(0)],
                );
                if proc_name.eq_ignore_ascii_case("phonon.exe") {
                    if let Ok(handle) = OpenProcess(PROCESS_TERMINATE, false, entry.th32ProcessID) {
                        let _ = TerminateProcess(handle, 0);
                        let _ = CloseHandle(handle);
                    }
                }
                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }

        let _ = CloseHandle(snapshot);
    }
}
