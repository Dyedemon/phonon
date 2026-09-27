import { test, expect } from '@playwright/test';

test('Minimal — React mount under playwright test runner', async ({ page }) => {
  // 只注入 stub，不加其他东西
  await test.info().attach('probe', { body: 'starting' });

  await (page.context() as any).addInitScript(() => {
    // ── 捕获所有可能的错误路径 ─────────────────────────────
    const allLogs: string[] = [];
    const pushLog = (prefix: string, args: any[]) => {
      try {
        allLogs.push(`[${prefix}] ` + args.map((a) => {
          if (typeof a === 'string') return a.slice(0, 400);
          if (a instanceof Error) return `${a.name}: ${a.message}\n${(a.stack || '').slice(0, 300)}`;
          try { return JSON.stringify(a).slice(0, 400); } catch { return String(a).slice(0, 200); }
        }).join(' | '));
      } catch { /* ignore */ }
    };
    const origError = console.error.bind(console);
    const origWarn = console.warn.bind(console);
    const origLog = console.log.bind(console);
    console.error = (...a: any[]) => { pushLog('console.error', a); origError(...a); };
    console.warn = (...a: any[]) => { pushLog('console.warn', a); origWarn(...a); };
    console.log = (...a: any[]) => { pushLog('console.log', a); origLog(...a); };
    window.addEventListener('error', (e) => pushLog('window.error', [e.error ?? e.message, e.filename, e.lineno, e.colno]));
    window.addEventListener('unhandledrejection', (e) => pushLog('unhandledrejection', [(e as any).reason]));
    (window as any).__all_logs__ = allLogs;

    // ── ENHANCED Tauri stub ─────────────────────────────
    try {
      // storage.ts 真实 key：EULA accepted = phonon-eula-accepted，字符串 'true'
      localStorage.setItem('phonon-eula-accepted', 'true');
      localStorage.setItem('phonon-lyrics-api-enabled', 'false');
    } catch { /* ignore */ }
    if (typeof (window as any).__TAURI_INTERNALS__ === 'undefined') {
      const _defaults: Record<string, any> = {
        'get_playback_state': { state: 'Idle', position_secs: 0, duration_secs: null, buffer_fill: 0, current_track: null, queue_length: 0, speed: 1.0 },
        'get_queue': [],
        'add_to_queue': { added: 0, duplicates: 0 },
        'list_playlists': [],
        'get_active_playlist': '__default__',
        'get_synced_folders': {},
        'library_get_roots': [],
        'scan_folder': [],
        'library_get_tracks_by_paths': [],
        'list_devices': [],
        'get_current_device': null,
        'list_dsp_processors': [],
        'get_dsp_latency': 0,
        'get_replay_gain': 'auto',
        'get_smart_effect_mode': 'off',
        'get_surround_sound_mode': 'off',
        'get_eq_state': { bands: [], preamp_db: 0, mode: 'Graphic' },
        'get_track_metadata': { artist: '', title: '', album: '', album_artist: '', year: 0, trackno: 0, discno: 0, duration: 0 },
        'get_album_art': null,
        'get_lyrics': null,
        'search_lyrics': { provider: 'stub', provider_type: 'offline', source_text: '', lyrics: [] },
        'search_lyrics_by_keyword': [],
        'fetch_lyrics_by_id': { provider: 'stub', provider_type: 'offline', source_text: '', lyrics: [] },
        'scan_lyric_sources': [],
        'read_lyric_script': '',
        'parse_lrc_text': { provider: 'stub', provider_type: 'offline', source_text: '', lyrics: [] },
        'list_plugins': [],
        'scan_vis_sources': [],
        'import_plugin': [],
        'remove_plugin': [],
        'reload_plugin': [],
        'enable_plugin': null,
        'disable_plugin': null,
        'load_time_stretch_plugin': null,
        'unload_time_stretch_plugin': null,
        'add_wasm_dsp': null,
        'remove_wasm_dsp': null,
        'toggle_plugin_in_dsp': null,
        'get_plugin_config': null,
        'set_plugin_config': null,
        'set_plugin_limits': null,
        'get_hotplug_notifications': true,
        'get_close_to_tray': false,
        'get_startup_behavior': 'show_window',
        'get_debug_log': false,
        'get_settings': {},
        'prepare_for_reload': null,
        'set_window_title': null,
        'get_hw_volume': null,
      };
      let _cbId = 0;
      (window as any).__TAURI_INTERNALS__ = {
        invoke: (cmd: string, args?: any) => {
          const v = Object.prototype.hasOwnProperty.call(_defaults, cmd) ? _defaults[cmd] : undefined;
          if (Array.isArray(v)) return Promise.resolve(v.slice());
          if (v && typeof v === 'object') return Promise.resolve({ ...v });
          return Promise.resolve(v);
        },
        transformCallback: (cb: (p: any) => void) => { return ++_cbId; },
        listen: () => Promise.resolve(() => {}),
        emit: () => Promise.resolve(),
        getCurrentWindow: () => ({ label: 'stub', listen: () => Promise.resolve(() => {}), emit: () => Promise.resolve() }),
      };
      (window as any).__TAURI_STUB__ = true;
    }
    (window as any).__dbg_tracer = 'injected';
  });

  // ── 抓 Playwright runtime 的 browser console ────────────────
  const runtimeLogs: string[] = [];
  page.on('console', (m) => {
    runtimeLogs.push(`[pw console.${m.type()}] ${m.text().slice(0, 400)}`);
    // 附加 stack trace（Playwright 支持 args 但部分情况需要 JSON 化）
    (async () => {
      try {
        for (const arg of m.args()) {
          const json = await arg.jsonValue().catch(() => null);
          if (json) runtimeLogs.push(`        arg_json: ${JSON.stringify(json).slice(0, 300)}`);
        }
      } catch { /* ignore */ }
    })();
  });
  page.on('pageerror', (e) => runtimeLogs.push('[pw pageerror] ' + e.message + '\n' + (e.stack || '').slice(0, 500)));

  await page.goto('http://localhost:5173/', { waitUntil: 'domcontentloaded', timeout: 30_000 });
  await page.waitForTimeout(6_000);

  const info = await page.evaluate(() => {
    const root = document.querySelector('#root');
    const tabs = document.querySelector('div.tabs');
    return {
      tracer: (window as any).__dbg_tracer ?? 'missing',
      hasTauri: typeof (window as any).__TAURI_INTERNALS__,
      hasRoot: !!root,
      rootChildren: root?.childElementCount ?? -1,
      tabsExists: !!tabs,
      preview: root ? root.innerHTML.slice(0, 600) : '(null)',
      scripts: Array.from(document.querySelectorAll('script')).slice(-6).map((s: any) => ({ src: s.src?.slice(0, 100), inline: s.textContent?.slice(0, 40) || '' })),
      allLogs: (window as any).__all_logs__ ?? [],
    };
  }, undefined, { timeout: 5_000 });
  console.log('[MINIMAL TEST] page info:', JSON.stringify(info, null, 2).slice(0, 3600));
  console.log('[MINIMAL TEST] runtime logs:', runtimeLogs.join('\n').slice(0, 3000));

  expect(info.hasRoot).toBe(true);
});

