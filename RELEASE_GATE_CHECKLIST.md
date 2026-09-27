# §16 Release Gate — 发布前人工复核清单

CI Workflow（`.github/workflows/release.yml`）已经实现了自动化 Gate：
1. `pre-release-gate`（Ubuntu）— 类型检查 / 构建 / deny / 测试 / Updater 公钥非占位 / third-party 文本校验
2. 三平台 `build` 矩阵 — 签名导入 + codesign/notarization/staple + SHA256SUMS 生成
3. `release-verification`（Ubuntu）— 5 道 Gate（完整性/Updater sig/Authenticode/Gatekeeper evidence/AppImage sanity）
4. `release`（必须等 2+3 都过了）

本 checklist 覆盖 **CI 无法自动化的部分**（CI 跑在 Linux 上无法调用 spctl/Get-AuthenticodeSignature/SmartScreen 浏览器下载测试）。**发布负责人必须逐项打勾，缺少任何一项时都要走 §5 的 FALLBACK 流程，不能静默跳过。**

---

## 0. 前置：CI 通过了吗？

- [ ] `pre-release-gate` job 通过（绿）
- [ ] 三平台 `build`（windows / macos-universal / linux）全部通过（绿）
- [ ] `release-verification` job 5 个 Gate 全部 PASSED / 明确 SKIPPED 原因已记录
  - Gate 1/5 Release payload consistency — 需 **PASSED**
  - Gate 2/5 Updater signature coverage — 有 `TAURI_SIGNING_PRIVATE_KEY` 时需 **PASSED**，否则 SKIPPED 允许
  - Gate 3/5 Windows Authenticode — 有 `WINDOWS_CODE_SIGN_PFX` 时需 **PASSED**，否则 SKIPPED 允许
  - Gate 4/5 macOS Gatekeeper evidence — 有 `APPLE_CERTIFICATE` 时需输出 **本地复核 REQUIRED**（按 §2 执行），否则 SKIPPED 允许
  - Gate 5/5 Linux AppImage sanity — 需 **PASSED**
- [ ] `release` job 已创建 draft release（不要取消 draft 直到本 checklist 全过）

---

## 1. 完整性锚点：SHA256SUMS 交叉校验

下载 draft release 里的 SHA256SUMS 和全部 bundles，**在 2 台不同机器上**分别执行：

```bash
# macOS / Linux
sha256sum -c SHA256SUMS

# Windows (PowerShell)
Get-Content SHA256SUMS | ForEach-Object {
  $hash,$name = $_ -split '\s\s+',2
  $actual = (Get-FileHash -Algorithm SHA256 $name).Hash.ToLower()
  if ($hash -ne $actual) { Write-Error "MISMATCH: $name" } else { "OK: $name" }
}
```

- [ ] 全部 bundles 校验和 SHA256SUMS 吻合（**无 MISMATCH 行**）
- [ ] updater.json 中每个 platform 条目的 signature 字段非空字符串（或：已确认 TAURI_SIGNING_PRIVATE_KEY 未启用 → 接受空）

---

## 2. macOS 平台人工复核（需 macOS 13+ 真机）

### 2.1 Gatekeeper / notarization ticket

下载 `.dmg`，挂载后把 `.app` 拖到 ~/Desktop：

```bash
# 1) codesign 深度验证（= 系统实际执行前做的）
codesign --verify --deep --strict --verbose=4 /Applications/Phonon.app

# 2) Gatekeeper 评估（= 右键「打开」/ 双击时系统问你的依据）
spctl --assess --type execute -vv /Applications/Phonon.app
# 期望输出：accepted · source=Notarized Developer ID · 无 override: 标记

# 3) 公证票据 stapled（= 断网下也能通过 Gatekeeper）
stapler validate /Applications/Phonon.app
# 期望输出：The validate action worked!
stapler validate Phonon_*.dmg
```

- [ ] `codesign --deep --strict` ✔
- [ ] `spctl --assess` → accepted + Notarized Developer ID ✔
- [ ] `stapler validate` .app + .dmg ✔

### 2.2 FALLBACK（当 CI 中 APPLE_CERTIFICATE 未配置）

CI 已明确 SKIPPED Gate 4，此时：
- [ ] **不能**发布到「正式 Release」渠道，只能标记为 `Internal Testing / Pre-release`
- [ ] 在 Release Notes 顶部加 banner：`⚠️ 本版本未经过 Apple 公证，首次打开需右键 → 打开`
- [ ] 把 `draft_release=true`，勾选 `This is a pre-release`

---

## 3. Windows 平台人工复核（需 Win10+ 真机）

### 3.1 Authenticode 签名 + SmartScreen 可启动性

下载 `*.nsis.exe` installer 和 `*.portable.exe`：

```powershell
# Authenticode 签名有效性
Get-AuthenticodeSignature .\Phonon_*.nsis.exe | Format-List
Get-AuthenticodeSignature .\Phonon_*.portable.exe | Format-List
# 期望 Status = Valid；SignerCertificate 主题 CN = 签发组织名

# 直接安装 + 启动（检查 SmartScreen 弹窗）
.\Phonon_*.nsis.exe
```

- [ ] `Get-AuthenticodeSignature` 两个 bundle 都是 **Status: Valid**
- [ ] 运行 nsis installer：
  - 若证书为 **EV**：无 SmartScreen 弹窗 → ✅
  - 若证书为 **OV (标准)**：首次运行可能出现「不认识的应用」，点「仍要运行」后能正常启动（**SmartScreen reputation 需要下载量积累，首次出包不要求 0 弹窗**）→ ✅
  - 若**未签名**（WINDOWS_CODE_SIGN_PFX 未配置）→ 按 §3.2 FALLBACK
- [ ] 安装后 App 能双击打开、托盘菜单出现、主窗口渲染正常
- [ ] portable 版本解压即用，不报错

### 3.2 FALLBACK（未配置 WINDOWS_CODE_SIGN_PFX）

CI 已 SKIPPED Gate 3：
- [ ] 只能发布 **Pre-release**，Release Notes 顶部加：`⚠️ Windows 包未代码签名，SmartScreen 会拦截。安装需在拦截弹窗点「更多信息 → 仍要运行」`
- [ ] 建议发布后 48 小时内导入 EV/OV 证书并重签（osslsigncode 或 signtool），用相同 semver tag 重新推送 Release Assets

---

## 4. Linux AppImage 人工复核（Ubuntu 22.04+ 真机 / VM）

```bash
chmod +x Phonon_*.AppImage
./Phonon_*.AppImage --appimage-extract && ls squashfs-root/AppRun squashfs-root/.DirIcon squashfs-root/usr/bin/phonon
# → 三样都存在
rm -rf squashfs-root

# 实测启动（需要 DISPLAY / X11/Wayland；无头环境用 xvfb-run 兜底）
xvfb-run -a ./Phonon_*.AppImage --appimage-offset  # 纯 smoke：能输出 offset 就是 ELF 合法
```

- [ ] `./Phonon_*.AppImage` 启动：托盘 + 主窗口正常
- [ ] `.deb` 在干净 Ubuntu 22.04 容器内：`sudo apt install ./Phonon_*.deb` → 无 broken dependencies；`phonon` 命令可启动

---

## 5. Tauri Updater 端到端签名验证（最关键）

> 这是 §16 Gate 的核心：客户端 Updater 必须真正能校验通过 updater.json 平台条目的 .sig。

准备 2 台机器（旧版 App 已安装）：
1. 把 draft release 的 bundles + updater.json 放到任意 HTTPS 静态目录（或用 GitHub Release 的原始 raw URL）
2. 在旧版 App 的 `tauri.conf.json` → `plugins.updater.endpoints` 临时指向这个 updater.json
3. 启动旧 App，触发检查更新：

```bash
# 同时用 tauri 自带 CLI 离线验证
npm run tauri signer verify \
  --pubkey $(cat tauri.conf.json | jq -r '.plugins.updater.pubkey') \
  --signature $(cat updater.json | jq -r '.platforms["windows-x86_64"].signature') \
  Phonon_*.nsis.exe
# 期望输出：Signature verified ✓
```

- [ ] **Windows**：客户端点击「更新」，下载后能校验通过并自动重启（无「签名校验失败」弹窗）
- [ ] **macOS**：同上，DMG 下载 + 签名校验通过
- [ ] **Linux**：AppImage .zsync 增量差分下载 OK，或全量 AppImage 校验通过
- [ ] `tauri signer verify` 手工 3 平台全部输出 `Signature verified`

### Updater FALLBACK（发现某平台 sig 不匹配）

- 立刻把 draft release 保持为 draft，**不要发布**
- 删除已推送的 tag：`git push origin :refs/tags/vX.Y.Z && git tag -d vX.Y.Z`
- 重新生成 signer key：`npm run signer:generate`，把新公钥回填 `tauri.conf.json`，重新打 tag vX.Y.Z+1
- 重跑整个 workflow

---

## 6. 正式出包 go/no-go 汇总

全部 PASS 后才能取消 draft 并点「Publish release」：

| Section | Verdict | 备注 |
|---|---|---|
| 0. CI Gates | ▢ PASS / ▢ BLOCK | |
| 1. SHA256SUMS | ▢ PASS / ▢ BLOCK | |
| 2. macOS Gatekeeper | ▢ PASS / ▢ FALLBACK (Pre-release only) | |
| 3. Windows Authenticode/SmartScreen | ▢ PASS / ▢ FALLBACK (Pre-release only) | |
| 4. Linux AppImage | ▢ PASS / ▢ BLOCK | |
| 5. Updater E2E sig verify | ▢ PASS / ▢ BLOCK | |

- [ ] **最终 go/no-go**：所有 BLOCK 都已解决或按 FALLBACK 正确标记 Pre-release banner
- [ ] GitHub Release 已 Publish（取消 Draft），且「Pre-release」勾选按 §2/§3 FALLBACK 要求
- [ ] updater.json 公钥和 tauri.conf.json 匹配，Release Assets 里的 SHA256SUMS 已同步

---

## 7. 回滚 / 紧急回退路径

若发布后 24 小时内发现严重问题（崩溃、音频失真、数据丢失、Updater 签名不匹配）：

1. **立刻标记 Pre-release / Draft**：在 GitHub Release 页面 → Edit → 勾选「This is a pre-release」或再次勾选 Draft，阻止新用户从 Latest 获取
2. **撤回 updater.json**：把 `https://releases.phonon.app/updater.json` 的 platforms 对象清空（变成 `{}`），用户侧会显示「已是最新版本」，不会继续拉坏包
3. **发 hotfix tag vX.Y.Z+1**：按 checklist 从第 0 条开始重新走一遍；Updater E2E (§5) 要求跑两次（旧版→新版 升级；新版→新版 无重复升级）
4. **归档故障 bundle**：在 Release Notes 上写明受影响版本范围 + 临时卸载命令，不要真的从 Release Assets 删除（保留调查证据 + SHA256 追溯）
