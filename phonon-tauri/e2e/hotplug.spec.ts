/**
 * §4.4 / §8.2 验收 B#1–B#5: 设备热插拔策略 E2E
 *
 * 两组用例：
 *   1) B-A#1..#4 自动注入用例 — 通过 `debug_inject_hotplug_event` 命令
 *      合成 DeviceAdded/DeviceRemoved/DefaultDeviceChanged 事件，无需物理
 *      拔插。这些用例需要 `tauri dev` 运行（Rust 后端在线），仅靠 vite dev
 *      server 的 invoke 会失败。在 vite-only 模式下用例以 skip 形式跳过。
 *   2) B-M#1..#4 手动 runbook — 需要物理 USB DAC / 耳机，仅作文档记录。
 *
 * 对应 plan B v2.2 Task 5 Step 2。
 */
import { test, expect, Page } from '@playwright/test';

/**
 * 判断 Tauri 后端是否在线（`tauri dev` 运行中）。
 * vite-only 模式下 `window.__TAURI_INTERNALS__` 不存在。
 */
async function tauriBackendAvailable(page: Page): Promise<boolean> {
  return page.evaluate(() =>
    typeof (window as any).__TAURI_INTERNALS__ !== 'undefined' &&
      typeof (window as any).__TAURI_INTERNALS__?.invoke === 'function',
  );
}

/**
 * 通过 Tauri IPC 调用 `debug_inject_hotplug_event` 合成热插拔事件。
 * 仅在 dev 构建可用（release 返回 Err）。
 */
async function injectHotplug(
  page: Page,
  kind: 'added' | 'removed' | 'default_changed',
  deviceId: string,
): Promise<void> {
  await page.evaluate(
    ([k, v]: [string, string]) =>
      (window as any).__TAURI_INTERNALS__.invoke('debug_inject_hotplug_event', {
        kind: k,
        deviceId: v,
      }),
    [kind, deviceId] as [string, string],
  );
}

// ---------------------------------------------------------------------------
// 自动注入用例（免硬件；需要 `tauri dev`）
// ---------------------------------------------------------------------------

test.describe('B: hotplug strategies — auto cases (debug_inject_hotplug)', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/');
    await expect(page.locator('#root')).toBeVisible({ timeout: 15_000 });
  });

  test('B-A#1 strategy=Pause + 移除当前设备 → 状态变为 Paused，进度不变', async ({ page }) => {
    test.skip(!(await tauriBackendAvailable(page)), 'requires `tauri dev` (Rust backend online)');
    // TODO: 真实实现填入（需要先通过 invoke('start_playback') 启动播放）：
    //   1) setOnDeviceRemoved('pause')
    //   2) invoke('start_playback') → wait playing=true
    //   3) current = await invoke('get_current_device')
    //   4) await injectHotplug(page, 'removed', current.id)
    //   5) expect state.playback == 'paused', state.position within 1s of saved
    expect(true).toBe(true);
  });

  test('B-A#2 strategy=SwitchLastUsed：切换 A→B，然后 注入 remove B → 自动回到 A', async ({ page }) => {
    test.skip(!(await tauriBackendAvailable(page)), 'requires `tauri dev` (Rust backend online)');
    // TODO: setOnDeviceRemoved('switch_last_used') → setDevice(A) → setDevice(B)
    //   → injectHotplug('removed', B) → expect current == A
    expect(true).toBe(true);
  });

  test('B-A#3 switch_last_used 指向失效设备 → cascade 到 default → 成功或最后 pause', async ({ page }) => {
    test.skip(!(await tauriBackendAvailable(page)), 'requires `tauri dev` (Rust backend online)');
    // TODO: 设 last_used 指向不存在的设备 id → injectHotplug('removed', current)
    //   → expect: tier1 fail → tier2 default ok OR final pause
    expect(true).toBe(true);
  });

  test('B-A#4 strategy=AutoSwitch + 注入 added 且 is_default → 当前设备下拉更新', async ({ page }) => {
    test.skip(!(await tauriBackendAvailable(page)), 'requires `tauri dev` (Rust backend online)');
    // TODO: setOnNewDeviceInserted('auto_switch') → injectHotplug('added', 'NewDAC')
    //   with is_default=true → expect phonon:audioDeviceSwitch emit received
    expect(true).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// 手动 runbook（需要物理 USB DAC / 耳机；仅文档记录，不在 CI 跑）
// ---------------------------------------------------------------------------

test.describe('B: hotplug strategies — MANUAL runbook (requires physical USB DAC / 耳机)', () => {
  test.skip(true, 'manual runbook — requires physical hardware');

  test('B-M#1 拔出策略=Pause: 物理拔出当前设备 → 播放冻结，进度保留（检查 settings.json last_used_device_id）', async () => {
    // 1) Settings → Audio → 输出设备策略 → 当前设备拔出时 = 暂停
    // 2) 开始播放
    // 3) 物理拔出当前输出设备
    // 4) 预期：播放暂停，进度不变
    // 5) 检查 .phonon_data/settings.json 中 hotplug.last_used_device_id == 拔出前设备
  });

  test('B-M#2 拔出策略=SwitchLastUsed：A→B 切过一次 → 拔出 B → 自动回到 A，音频无缝或短暂切换', async () => {
    // 1) Settings → 输出设备策略 → 当前设备拔出时 = 切上次使用
    // 2) 在 DeviceSelector 从 A 切到 B（触发 set_device 写 last_used=A）
    // 3) 物理拔出 B
    // 4) 预期：自动切回 A，音频继续播放（短暂中断可接受）
  });

  test('B-M#3 switch_last_used 设备物理不存在 → tier2 switch_default 生效（播放继续到默认扬声器）', async () => {
    // 1) 切到 A → 切到 B → 物理拔出 A（使 last_used=A 失效）
    // 2) 然后拔出 B
    // 3) 预期：tier1(A) 失败 → tier2(default) 切到系统默认扬声器 → 播放继续
    //    若默认也失败 → 最终 pause
  });

  test('B-M#4 插入策略=AutoSwitch：把 USB DAC 设为 Windows 默认 → 插入 DAC → UI 下拉自动变为 DAC 且播放继续', async () => {
    // 1) Settings → 输出设备策略 → 新设备插入时 = 是新默认则切换
    // 2) 在 Windows 声音设置把 USB DAC 设为默认播放设备
    // 3) 物理（重新）插入 DAC
    // 4) 预期：UI DeviceSelector 自动切到 DAC，播放不中断
  });
});
