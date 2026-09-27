/**
 * smoke.spec.ts — 最小 Playwright smoke 测试，用来验证 page.goto + page.evaluate
 * 在 PLAYWRIGHT_E2E=1 (Vite HMR disabled) 模式下不会挂。
 *
 * 用法： npm run e2e -- smoke.spec.ts --retries=0 --reporter=list
 */
import { test, expect } from '@playwright/test';

test.describe.configure({ mode: 'serial' });
test.setTimeout(90_000);

test('smoke: goto + evaluate returns within 15s', async ({ page, context }) => {
  // 注入和 calibration 一样的 stub（完整 defaults + Tauri 2.x event 对象）。
  function stubFn(this: any) {
    try {
      var LS = window.localStorage;
      LS.setItem('phonon-eula-accepted', 'true');
      LS.setItem('phonon-lyrics-api-enabled', 'false');
      LS.setItem('phonon-storage-version', '2');
    } catch { /* ignore */ }
    if (typeof (window as any).__TAURI_INTERNALS__ !== 'undefined') return;

    var _cbId = 0;
    var _d: Record<string, any> = {
      'get_playback_state': { state: 'Idle', position_secs: 0, duration_secs: null, buffer_fill: 0, current_track: null, queue_length: 0, speed: 1.0 },
      'get_queue': [], 'play_index': null, 'remove_from_queue': null, 'reorder_queue': null,
      'add_to_queue': { added: 0, duplicates: 0 }, 'add_cue_to_queue': { added: 0, duplicates: 0 },
      'list_playlists': [], 'get_active_playlist': '__default__',
      'switch_playlist': null, 'delete_playlist': null, 'rename_playlist': null, 'reorder_playlists': null,
      'get_synced_folders': {}, 'sync_folder': null, 'unsync_folder': null, 'sync_folder_for_playlist': { added: 0, removed: 0 },
      'library_get_roots': [], 'library_set_roots': null, 'library_scan': null, 'scan_folder': [],
      'library_get_tracks_by_paths': [], 'list_devices': [], 'get_current_device': null, 'set_device': null,
      'list_dsp_processors': [], 'get_dsp_latency': 0, 'disable_dsp': null, 'enable_dsp': null, 'reorder_dsp': null,
      'get_replay_gain': 'auto', 'set_replay_gain': null,
      'get_smart_effect_mode': 'off', 'set_smart_effect_mode': null,
      'get_surround_sound_mode': 'off', 'set_surround_sound_mode': null,
      'get_bit_depth': 16, 'set_bit_depth': null,
      'get_output_sample_rate': 44100, 'set_output_sample_rate': null,
      'get_dsd_mode': 'none', 'set_dsd_mode': null,
      'get_eq_state': { bands: [], preamp_db: 0, mode: 'Graphic' },
      'get_track_metadata': { artist: '', title: '', album: '', album_artist: '', year: 0, trackno: 0, discno: 0, duration: 0 },
      'get_album_art': null, 'get_lyrics': null,
      'search_lyrics': { provider: 'stub', provider_type: 'offline', source_text: '', lyrics: [] },
      'search_lyrics_by_keyword': [],
      'parse_lrc_text': { provider: 'stub', provider_type: 'offline', source_text: '', lyrics: [] },
      'get_lyric_api_url': '', 'set_lyrics_api_url': null,
      'list_plugins': [], 'scan_vis_sources': [], 'import_plugin': [], 'remove_plugin': [], 'reload_plugin': [],
      'enable_plugin': null, 'disable_plugin': null,
      'load_time_stretch_plugin': null, 'unload_time_stretch_plugin': null,
      'add_wasm_dsp': null, 'remove_wasm_dsp': null, 'toggle_plugin_in_dsp': null,
      'get_plugin_config': null, 'set_plugin_config': null, 'set_plugin_limits': null,
      'get_hotplug_notifications': true, 'set_hotplug_notifications': null,
      'get_close_to_tray': false, 'set_close_to_tray': null,
      'get_startup_behavior': 'show_window', 'set_startup_behavior': null,
      'get_debug_log': false, 'set_debug_log': null, 'get_settings': {},
      'prepare_for_reload': null, 'set_window_title': null, 'get_hw_volume': null,
    };

    var _evtCounter = 0;
    var _evtListeners: Record<string, any[]> = {};
    function _evtRegister(event: string, cbId: number) {
      var id = 'stub-eid-' + (++_evtCounter);
      if (!_evtListeners[event]) _evtListeners[event] = [];
      _evtListeners[event].push({ id, cbId });
      return Promise.resolve({ id });
    }
    function _evtUnregister(event: string, id: string) {
      var list = _evtListeners[event] || [];
      for (var i = list.length - 1; i >= 0; i--) {
        if (list[i].id === id) list.splice(i, 1);
      }
      return Promise.resolve();
    }
    function _evtCount(event: string) {
      return (_evtListeners[event] || []).length;
    }
    function _copy(v: any) {
      if (Array.isArray(v)) return v.slice();
      if (v && typeof v === 'object') return Object.assign({}, v);
      return v;
    }
    function makeStubWindow(label: string) {
      var self: any = {
        label,
        listen: function (ev: string, _h: any) {
          return (window as any).__TAURI_INTERNALS__.listen(ev, _h);
        },
        emit: function () { return Promise.resolve(); },
        once: function (ev: string, h: any) {
          return self.listen(ev, h);
        },
        setTitle: function () { return Promise.resolve(); },
        isMaximized: function () { return Promise.resolve(false); },
        maximize: function () { return Promise.resolve(); },
        unmaximize: function () { return Promise.resolve(); },
        minimize: function () { return Promise.resolve(); },
        unminimize: function () { return Promise.resolve(); },
        show: function () { return Promise.resolve(); },
        hide: function () { return Promise.resolve(); },
        close: function () { return Promise.resolve(); },
        destroy: function () { return Promise.resolve(); },
        onMaximized: function (h: any) { return self.listen('tauri://maximized', h); },
        onUnmaximized: function (h: any) { return self.listen('tauri://restored', h); },
        onResized: function (h: any) { return self.listen('tauri://resize', h); },
        onMoved: function (h: any) { return self.listen('tauri://move', h); },
        onCloseRequested: function (h: any) { return self.listen('tauri://close-requested', h); },
      };
      return self;
    }

    (window as any).__TAURI_INTERNALS__ = {
      metadata: { windowsLabels: ['main'] },
      invoke: function (cmd: string) {
        var v = Object.prototype.hasOwnProperty.call(_d, cmd) ? _d[cmd] : undefined;
        return Promise.resolve(_copy(v));
      },
      transformCallback: function () { ++_cbId; return _cbId; },
      listen: function (event: string, _handler: any) {
        return (window as any).__TAURI_INTERNALS__.event
          .registerListener(event, -1)
          .then(function () {
            return function () {
              return (window as any).__TAURI_INTERNALS__.event.unregisterListener(event, -1);
            };
          });
      },
      emit: function () { return Promise.resolve(); },
      getCurrentWindow: function () { return makeStubWindow('stub'); },
      Core: { getCurrentWindow: function () { return makeStubWindow('stub'); } },
      event: {
        registerListener: _evtRegister,
        unregisterListener: _evtUnregister,
        listeners: _evtCount,
        emit: function () { return Promise.resolve(); },
      },
    };
    (window as any).__TAURI_STUB__ = true;
  }
  await context.addInitScript(stubFn);
  await page.setViewportSize({ width: 1440, height: 1024 });

  // ── 错误 / console 收集 ─────────────────────────────────────────────
  const pageErrors: string[] = [];
  const consoleErrors: string[] = [];
  page.on('pageerror', (e) => {
    const msg = String(e);
    pageErrors.push(msg.slice(0, 400));
    // stack (如果是 Error 对象)
    if ((e as any).stack) pageErrors.push('STACK:' + String((e as any).stack).slice(0, 800));
  });
  page.on('console', (msg) => {
    const type = msg.type();
    if (type === 'error' || type === 'warn') {
      consoleErrors.push(`[${type}] ${msg.text().slice(0, 300)}`);
    }
  });

  const t0 = Date.now();
  await page.goto('/', { waitUntil: 'domcontentloaded', timeout: 30_000 });
  console.log(`[smoke] goto done in ${Date.now() - t0}ms`);

  const t1 = Date.now();
  const v1 = await page.evaluate<number>(() => 1 + 1);
  console.log(`[smoke] eval(1+1)=${v1} took ${Date.now() - t1}ms`);
  expect(v1).toBe(2);

  // waitForFunction: #root mount 轮询（Vite 冷启动，domcontentloaded 早于
  // React module 编译完成），失败时把 pageErrors + consoleErrors 打出来
  let rootMounted = true;
  try {
    await page.waitForFunction(
      () => {
        const r = document.getElementById('root');
        return !!(r && r.firstElementChild);
      },
      { timeout: 25_000, polling: 200 },
    );
    console.log(`[smoke] #root mounted after goto+poll ${Date.now() - t0}ms`);
  } catch (e) {
    rootMounted = false;
    console.log('[smoke] #root NOT mounted after 25s. Diagnostics:');
    console.log('  pageErrors:', JSON.stringify(pageErrors).slice(0, 2000));
    console.log('  consoleErrors(First 15):', JSON.stringify(consoleErrors.slice(0, 15)).slice(0, 3000));
    // 尝试读 #root + html.body 快照
    try {
      const snap = await page.evaluate<any>(() => {
        var r = document.getElementById('root');
        return {
          rootExists: !!r,
          rootInnerHTML_prefix: r ? (r.innerHTML || '').slice(0, 500) : null,
          bodyChildTags: Array.from(document.body?.children ?? []).map((c) => (c as HTMLElement).tagName).slice(0, 20),
        };
      });
      console.log('  DOM snap:', JSON.stringify(snap).slice(0, 1500));
    } catch (e2) {
      console.log('  DOM snap eval failed:', String(e2).slice(0, 300));
    }
    throw e;
  }
  expect(rootMounted).toBe(true);

  // 关掉 EULA（最简版）
  const t3 = Date.now();
  await page.evaluate(() => {
    var body = document.body;
    if (!body) return;
    var c = body.children;
    for (var i = c.length - 1; i >= 0; i--) {
      var el = c[i] as HTMLElement;
      if (el.tagName !== 'DIV') continue;
      if (!el.querySelector('input[type="checkbox"]')) continue;
      if (!el.querySelector('button')) continue;
      el.remove();
      break;
    }
  });
  console.log(`[smoke] dismiss-modal took ${Date.now() - t3}ms`);
  await page.waitForTimeout(1000);

  const t4 = Date.now();
  const info = await page.evaluate<any>(() => {
    var tabsEl = document.querySelector('div.tabs');
    var tabsClass = tabsEl ? (tabsEl.className || '') : '';
    var btns = document.querySelectorAll('button[data-tab]');
    var ids: string[] = [];
    btns.forEach((b) => { var d = (b as any).dataset?.tab; if (d) ids.push(d); });
    var active = document.querySelector<HTMLButtonElement>('button.tab.active');
    return { tabsClass: tabsClass.slice(0, 200), tabIds: ids, activeTab: active?.dataset?.tab ?? '?' };
  });
  console.log(`[smoke] info took ${Date.now() - t4}ms:`, JSON.stringify(info).slice(0, 600));

  // 用 __phononTestSetActiveTab 钩子切换 tab + 300ms 内部等待 + DOM 读取
  // ⚠ 必须在 CurveCanvas 800ms paint 之前完成所有 DOM 读取
  const t5 = Date.now();
  const after = await page.evaluate(async () => {
    var hook = (window as any).__phononTestSetActiveTab as
      | ((tab: string) => boolean)
      | undefined;
    var attempted = typeof hook === 'function' ? hook('extensions') : false;
    // 等 React commit（300ms < CurveCanvas 800ms paint deadline）
    await new Promise<void>((resolve) => setTimeout(resolve, 300));
    var active = document.querySelector<HTMLButtonElement>('button.tab.active');
    var panel = document.querySelector<HTMLElement>('.calibration-panel');
    var sections = document.getElementsByClassName('calibration-section').length;
    return {
      attempted,
      activeTab: active?.dataset?.tab ?? '?',
      hasPanel: !!panel,
      sectionCount: sections,
    };
  }, null, { timeout: 10_000 });
  console.log(`[smoke] tab-switch+read took ${Date.now() - t5}ms:`, JSON.stringify(after));
  expect(after.attempted).toBe(true);
  expect(after.activeTab).toBe('extensions');
  expect(after.hasPanel).toBe(true);
  expect(after.sectionCount).toBeGreaterThanOrEqual(5);
});
