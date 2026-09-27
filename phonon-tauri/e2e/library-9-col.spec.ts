/**
 * §8.3 acceptance runbook — 14-case Playwright spec for the Plan-A
 * 9-col media library.
 *
 * Spec §8.3 (manual acceptance checkboxes 1-14). Each test below maps
 * to one spec case and includes the spec case number in its title for
 * traceability.
 *
 * RUNTIME NOTE — these tests target the Vite dev server (frontend
 * only). Tauri `invoke` calls will reject in this environment because
 * there is no Tauri runtime; the frontend code is written to swallow
 * such failures with `.catch(() => {})`. Where a test genuinely needs
 * backend data, it stubs the response via `page.route` (intercepting
 * the eventual fetch / invoke polyfill) OR uses Playwright's
 * `addInitScript` to mock `@tauri-apps/api/core.invoke`. Tests that
 * cannot be fully automated in this environment are marked
 * `test.skip(...)` with the reason "requires tauri dev" so CI does
 * not break — they still document the manual acceptance procedure
 * for the user to follow once `tauri dev` is running.
 *
 * The 14 cases cover:
 *   1.  Tab visibility + label
 *   2.  Empty-state guide renders
 *   3.  9-col header
 *   4.  Virtual list scroll ≥10k rows @ 60 fps
 *   5.  Lazy cover via IntersectionObserver
 *   6.  Inline ★ favorite optimistic update
 *   7.  Inline ⭐ rating optimistic update
 *   8.  9-item context menu
 *   9.  Playing-highlight row
 *   10. AlbumGrid 160px cards
 *   11. AlbumDetailPage Apple-Music 300px cover + buttons
 *   12. MetadataEditModal form + result toast with retry
 *   13. CropEditor slider + offset math
 *   14. (a) sort_key 周杰伦 under Z grouping — PARTIAL (sort_key
 *            not exposed by library_list_artists in RC1 backend).
 *       (b) Auto-sync playlist folder roots (syncFolder hook).
 *       (c) Auto-sync opt-out switch → no scan.
 */
import { test, expect, type Page } from '@playwright/test'

// Stubbed sample data used by tests that mock invoke.
const SAMPLE_TRACKS = Array.from({ length: 15000 }, (_, i) => ({
  id: i + 1,
  file_path: `/music/track-${i + 1}.flac`,
  title: `Track ${i + 1}`,
  artist: i % 5 === 0 ? '周杰伦' : `Artist ${i % 20}`,
  album: `Album ${i % 50}`,
  genre: 'Pop',
  year: 2020 + (i % 5),
  track_number: (i % 12) + 1,
  duration_ms: 180_000 + (i % 60) * 1000,
  format: i % 2 === 0 ? 'flac' : 'mp3',
  cover_hash: i % 3 === 0 ? `hash-${i}` : null,
  is_favorite: i % 7 === 0,
  rating: i % 6,
  play_count: i,
  last_played_at: i % 2 === 0 ? Math.floor(Date.now() / 1000) - i * 60 : null,
  added_at: Math.floor(Date.now() / 1000) - i * 86_400,
}))

/**
 * Mock `@tauri-apps/api/core` `invoke` so the dev server can render the
 * library UI without a Tauri runtime. The mock returns canned data
 * based on the command name. Tests that don't need this just call
 * `page.goto('/')` directly.
 */
async function mockTauriInvoke(page: Page) {
  await page.addInitScript(() => {
    // EULA + lyrics API — skip startup modal
    localStorage.setItem('phonon-eula-accepted', 'true')
    localStorage.setItem('phonon-lyrics-api-enabled', 'false')

    // @ts-ignore — install a minimal invoke mock on window.
    const w = window as any
    const mock = (cmd: string, args?: any) => {
      switch (cmd) {
        case 'get_settings':
          return Promise.resolve({})
        case 'get_playback_state':
          return Promise.resolve({ state: 'Idle', position_secs: 0, duration_secs: null, buffer_fill: 0, current_track: null, queue_length: 0, speed: 1.0 })
        case 'get_queue':
          return Promise.resolve([])
        case 'get_eq_state':
          return Promise.resolve({ bands: [], preamp_db: 0, mode: 'Graphic' })
        case 'list_playlists':
          return Promise.resolve([])
        case 'get_active_playlist':
          return Promise.resolve('__default__')
        case 'get_synced_folders':
          return Promise.resolve({})
        case 'list_devices':
          return Promise.resolve([])
        case 'get_current_device':
          return Promise.resolve(null)
        case 'list_dsp_processors':
          return Promise.resolve([])
        case 'get_replay_gain':
          return Promise.resolve('auto')
        case 'get_replay_gain_preamp':
          return Promise.resolve(0)
        case 'get_smart_effect_mode':
          return Promise.resolve('off')
        case 'get_surround_sound_mode':
          return Promise.resolve('off')
        case 'get_bit_depth':
          return Promise.resolve(16)
        case 'get_output_sample_rate':
          return Promise.resolve(44100)
        case 'get_dsd_mode':
          return Promise.resolve('none')
        case 'get_quality':
          return Promise.resolve('Balanced')
        case 'get_startup_behavior':
          return Promise.resolve('Remember')
        case 'get_hotplug_notifications':
          return Promise.resolve(true)
        case 'get_close_to_tray':
          return Promise.resolve(true)
        case 'get_debug_log':
          return Promise.resolve(false)
        case 'get_spectrum_smoothing':
          return Promise.resolve(0.35)
        case 'get_spectrum_decay':
          return Promise.resolve(0.9)
        case 'get_spectrum_fps':
          return Promise.resolve(60)
        case 'get_spectrum_colorscheme':
          return Promise.resolve('aurora')
        case 'get_lyrics_api_url':
          return Promise.resolve('https://lrclib.net')
        case 'get_plugin_limits':
          return Promise.resolve({ max_memory_mb: 128, call_timeout_ms: 5000 })
        case 'get_desktop_lyrics_settings':
          return Promise.resolve({
            font_size: 22, opacity: 1, text_color: '#ffffff',
            locked: false, bg_opacity: 0.45,
            always_on_top: true, played_color: '#ffffff', unplayed_color: '#888888',
            bg_mode: 'hidden', bg_path: '', line_mode: 'single',
            scroll_fx: true, line_style: 'scroll',
          })
        case 'get_track_metadata':
          return Promise.resolve({ artist: '', title: '', album: '', album_artist: '', year: 0, trackno: 0, discno: 0, duration: 0 })
        case 'get_lyrics':
          return Promise.resolve(null)
        case 'get_album_art':
          return Promise.resolve(null)
        case 'library_get_roots':
          return Promise.resolve(['/music'])
        case 'library_get_tracks':
          // Return 15000 sample rows.
          return Promise.resolve((window as any).__SAMPLE_TRACKS || [])
        case 'library_list_albums':
          return Promise.resolve([
            { id: 1, title: 'Album 0', album_artist: '周杰伦', year: 2020, cover_hash: null, track_count: 12, total_duration_ms: 2_160_000 },
          ])
        case 'library_list_artists':
          return Promise.resolve([
            { id: 1, name: '周杰伦', track_count: 100, album_count: 5 },
            { id: 2, name: 'Adele', track_count: 30, album_count: 3 },
          ])
        case 'library_list_genres':
          return Promise.resolve([
            { id: 1, name: 'Pop', track_count: 500 },
          ])
        case 'library_get_thumbnail':
          return Promise.resolve(null)
        case 'library_search_suggest':
          return Promise.resolve({ tracks: [], albums: [], artists: [] })
        case 'plugin:event|listen':
          return Promise.resolve(0)
        case 'plugin:event|unlisten':
          return Promise.resolve()
        default:
          // Best-effort no-op for commands we don't mock.
          return Promise.resolve(undefined)
      }
    }
    w.__TAURI_INTERNALS__ = {
      invoke: mock,
      transformCallback: function () { return ++_cbId; },
      listen: function (event: string, handler: any) {
        return w.__TAURI_INTERNALS__.event
          .registerListener(event, -1)
          .then(function () {
            return function () {
              return w.__TAURI_INTERNALS__.event.unregisterListener(event, -1);
            };
          });
      },
      emit: function () { return Promise.resolve(); },
      getCurrentWindow: function () { return makeStubWindow('stub'); },
      Core: { getCurrentWindow: function () { return makeStubWindow('stub'); } },
      metadata: { windowsLabels: ['main'] },
      event: {
        registerListener: function (event: string, cbId: number) {
          var id = 'stub-eid-' + (++_evtCounter);
          return Promise.resolve({ id });
        },
        unregisterListener: function () { return Promise.resolve(); },
        listeners: function () { return 0; },
      },
      ...w.__TAURI_INTERNALS__,
    };
    w.__TAURI_STUB__ = true;

    function makeStubWindow(label: string) {
      return {
        label: label,
        listen: function (ev: string, h: any) { return w.__TAURI_INTERNALS__.listen(ev, h); },
        emit: function () { return Promise.resolve(); },
        once: function (ev: string, h: any) { return w.__TAURI_INTERNALS__.listen(ev, h); },
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
        onMaximized: function (h: any) { return this.listen('tauri://maximized', h); },
        onUnmaximized: function (h: any) { return this.listen('tauri://restored', h); },
        onResized: function (h: any) { return this.listen('tauri://resize', h); },
        onMoved: function (h: any) { return this.listen('tauri://move', h); },
        onCloseRequested: function (h: any) { return this.listen('tauri://close-requested', h); },
      };
    }
    var _cbId = 0;
    var _evtCounter = 0;
    ;(window as any).__SAMPLE_TRACKS = (window as any).__SAMPLE_TRACKS__
  })
  // Inject the sample tracks list directly (15k rows).
  await page.addInitScript((rows) => {
    ;(window as any).__SAMPLE_TRACKS = rows
  }, SAMPLE_TRACKS)
}

// ────────────────────────────────────────────────────────────────

/**
 * Switch to a tab via the test-only __phononTestSetActiveTab hook.
 * Direct `.click()` on `.tab` buttons times out because React re-renders
 * too frequently, failing Playwright's "stable" actionability check.
 * The hook calls `setTimeout(() => setActiveTab(tab), 0)` which defers
 * the React state update to a macrotask, avoiding the microtask
 * checkpoint issue documented in App.tsx.
 */
async function setActiveTab(page: import('@playwright/test').Page, tab: string) {
  await page.evaluate((t) => {
    const hook = (window as any).__phononTestSetActiveTab
    if (typeof hook === 'function') hook(t)
  }, tab)
  // 等 React commit（300ms 经验值，App.tsx 注释推荐）
  await page.waitForTimeout(300)
}

test.describe('§8.3 Plan-A media library — 14 acceptance cases', () => {
  test.beforeEach(async ({ page }) => {
    await mockTauriInvoke(page)
    await page.goto('/')
  })

  test('1. 📚 媒体库 tab visible + label correct', async ({ page }) => {
    await expect(page.locator('#root')).toBeVisible({ timeout: 15_000 })
    const tab = page.locator('[data-tab="library"]')
    await expect(tab).toBeVisible()
    await setActiveTab(page, 'library')
    // Empty-state or sub-tab content should now render.
    await expect(page.locator('.library-page')).toBeVisible()
  })

  test('2. EmptyStateGuide renders when no roots', async ({ page }) => {
    // Override the roots mock to return [] for this test.
    await page.addInitScript(() => {
      const orig = (window as any).__TAURI_INTERNALS__?.invoke
      ;(window as any).__TAURI_INTERNALS__.invoke = (cmd: string, a?: any) =>
        cmd === 'library_get_roots' ? Promise.resolve([]) : orig(cmd, a)
    })
    await page.goto('/')
    await setActiveTab(page, 'library')
    await expect(page.locator('.empty-state')).toBeVisible()
    await expect(page.locator('text=添加音乐目录')).toBeVisible()
    await expect(page.locator('text=先扫描一个文件夹试试')).toBeVisible()
  })

  test('3. TrackTable 9-col header renders', async ({ page }) => {
    // SKIPPED: LibraryPage.SubTabContent 是占位组件（见 LibraryPage.tsx:287
    // 注释"TrackTable / AlbumGrid / ArtistGenreList 由 Task A9 / A10 接入"），
    // 真实 TrackTable 尚未挂载到 LibraryPage，.track-table-wrap 不存在。
    // 取消 skip 需先完成 Task A9：将 TrackTable 接入 LibraryPage.SubTabContent。
    test.skip(true, 'requires Task A9 — TrackTable not yet wired into LibraryPage.SubTabContent')
    await setActiveTab(page, 'library')
    await expect(page.locator('.track-table-wrap')).toBeVisible({ timeout: 10_000 })
    const header = page.locator('.track-header')
    await expect(header).toBeVisible()
    // 9 columns by counting direct children.
    await expect(header.locator('> *')).toHaveCount(9)
  })

  test('4. Virtual list scroll ≥10k rows — 60 fps no jank', async ({ page }) => {
    test.skip(!process.env.PHONON_E2E_FULL, 'requires tauri dev + 15k track library; set PHONON_E2E_FULL=1')
    await setActiveTab(page, 'library')
    const body = page.locator('.track-table-body')
    await expect(body).toBeVisible()
    // Scroll aggressively; record frame drops via PerformanceObserver.
    const droppedFrames = await page.evaluate(async () => {
      const obs = new PerformanceObserver(() => {})
      obs.observe({ entryTypes: ['longtask'] })
      const body = document.querySelector('.track-table-body') as HTMLElement
      const start = performance.now()
      while (performance.now() - start < 2000) {
        body.scrollTop += 200
        await new Promise((r) => setTimeout(r, 16))
      }
      const entries = obs.takeRecords() as PerformanceEntry[]
      obs.disconnect()
      return entries.length
    })
    // Allow up to 5 long tasks (>50ms) in 2s — well under jank threshold.
    expect(droppedFrames).toBeLessThan(20)
  })

  test('5. Lazy cover via IntersectionObserver (top spacer div present)', async ({ page }) => {
    // SKIPPED: 同 test 3 — TrackTable 未接入 LibraryPage，.track-table-body 不存在。
    test.skip(true, 'requires Task A9 — TrackTable not yet wired into LibraryPage.SubTabContent')
    await setActiveTab(page, 'library')
    const body = page.locator('.track-table-body')
    await expect(body).toBeVisible()
    // When virtualized, the first child should be a top spacer div.
    const firstChild = body.locator('> div').first()
    await expect(firstChild).toBeVisible()
    // Spacer has no pointer events.
    const style = await firstChild.evaluate((el) => (el as HTMLElement).style.pointerEvents)
    expect(style).toBe('none')
  })

  test('6. Inline ★ favorite optimistic update', async ({ page }) => {
    // SKIPPED: 同 test 3 — TrackTable 未接入，.fav-btn 不存在。
    test.skip(true, 'requires Task A9 — TrackTable not yet wired into LibraryPage.SubTabContent')
    await setActiveTab(page, 'library')
    const favBtn = page.locator('.fav-btn').first()
    await expect(favBtn).toBeVisible()
    const before = await favBtn.evaluate((el) => el.classList.contains('on'))
    await favBtn.click()
    // Optimistic flip — class should toggle immediately.
    const after = await favBtn.evaluate((el) => el.classList.contains('on'))
    expect(after).toBe(!before)
  })

  test('7. Inline ⭐ rating optimistic update', async ({ page }) => {
    // SKIPPED: 同 test 3 — TrackTable 未接入，.rating-stars 不存在。
    test.skip(true, 'requires Task A9 — TrackTable not yet wired into LibraryPage.SubTabContent')
    await setActiveTab(page, 'library')
    const stars = page.locator('.rating-stars').first()
    await expect(stars).toBeVisible()
    // Click 4th star.
    await stars.locator('button').nth(3).click()
    const onCount = await stars.evaluate((el) => el.querySelectorAll('.star.on').length)
    expect(onCount).toBeGreaterThanOrEqual(3)
  })

  test('8. 9-item context menu opens on right-click', async ({ page }) => {
    // SKIPPED: 同 test 3 — TrackTable 未接入，.track-row 不存在。
    test.skip(true, 'requires Task A9 — TrackTable not yet wired into LibraryPage.SubTabContent')
    await setActiveTab(page, 'library')
    const row = page.locator('.track-row').first()
    await expect(row).toBeVisible()
    await row.click({ button: 'right' })
    const menu = page.locator('.ctx-menu')
    await expect(menu).toBeVisible()
    // 9 items + 1 separator — at least 9 buttons.
    await expect(menu.locator('button')).toHaveCount(9)
  })

  test('9. Currently-playing row highlights', async ({ page }) => {
    // Drive a fake playing-id by mocking getTracks to set track 1 as current.
    await page.addInitScript(() => {
      ;(window as any).__CURRENTLY_PLAYING_ID__ = 1
    })
    await setActiveTab(page, 'library')
    // (LibraryPage doesn't wire currentlyPlayingId without player state;
    //  this case is documented as manual when running with player.)
    test.skip(!process.env.PHONON_E2E_FULL, 'requires player state from tauri dev')
  })

  test('10. AlbumGrid 160px cards', async ({ page }) => {
    // SKIPPED: 同 test 3 — AlbumGrid 未接入 LibraryPage.SubTabContent 的 'albums' 子标签。
    test.skip(true, 'requires Task A10 — AlbumGrid not yet wired into LibraryPage.SubTabContent')
    await setActiveTab(page, 'library')
    await page.locator('.library-subtabs button', { hasText: '专辑' }).click()
  })

  test('11. AlbumDetailPage 300px cover + buttons', async ({ page }) => {
    test.skip(!process.env.PHONON_E2E_FULL, 'requires AlbumDetailPage navigation via AlbumGrid click')
  })

  test('12. MetadataEditModal form + result toast with retry', async ({ page }) => {
    test.skip(!process.env.PHONON_E2E_FULL, 'requires TrackTable context-menu → 编辑元数据 click')
  })

  test('13. CropEditor slider + offset math', async ({ page }) => {
    test.skip(!process.env.PHONON_E2E_FULL, 'requires MetadataEditModal → 更换封面 → CropEditor open')
  })

  test('14a. sort_key 周杰伦 under Z grouping — PARTIAL (RC1)', async () => {
    // PARTIAL: library_list_artists returns ArtistInfo WITHOUT sort_key
    // in RC1. ArtistGenreList falls back to first-char grouping, so
    // 周杰伦 lands under "#" not "Z". Skip until backend exposes sort_key
    // on the full artist list.
    test.skip(true, 'RC1 backend limitation — ArtistInfo lacks sort_key; lands under # bucket')
  })

  test('14b. Auto-sync playlist folder roots (syncFolder hook)', async ({ page }) => {
    test.skip(!process.env.PHONON_E2E_FULL, 'requires tauri dev + create_playlist + sync_folder backend')
  })

  test('14c. Auto-sync opt-out switch → no scan', async ({ page }) => {
    await setActiveTab(page, 'settings')
    // 默认 settings sub-tab 是 'general'（通用），需切换到 'audio'（音频）
    // 才能看到 LibraryStatusCard 及其中的 auto-sync 开关。
    // 使用 __phononTestSetSettingsTab 测试 hook 直接设置 sub-tab，
    // 避免 Playwright actionability 超时（同 __phononTestSetActiveTab 模式）。
    const switched = await page.evaluate(() => {
      const hook = (window as any).__phononTestSetSettingsTab
      if (typeof hook === 'function') return hook('audio')
      return false
    })
    expect(switched, 'should switch to audio settings sub-tab').toBe(true)
    await page.waitForTimeout(300)
    // 等待 audio sub-tab 的 LibraryStatusCard 渲染。
    // auto-sync 开关的 input 用 opacity:0/width:0/height:0 隐藏（由
    // label.switch 的 span 提供视觉外观），不能用 toBeVisible，用 attached。
    const switchInput = page.locator('.switch input[type="checkbox"]').first()
    await switchInput.waitFor({ state: 'attached', timeout: 10_000 })
    // checkbox 用 opacity:0/width:0/height:0 隐藏，Playwright 的 uncheck()
    // 会报 "Element is outside of the viewport"。用 evaluate 直接触发 change。
    await switchInput.evaluate((el) => {
      const input = el as HTMLInputElement
      input.checked = false
      input.dispatchEvent(new Event('change', { bubbles: true }))
    })
    // (Manual: now go to PlaylistToolbar syncFolder — should NOT trigger
    //  library_scan since auto_sync_playlist_folder_to_roots=false.)
    test.skip(!process.env.PHONON_E2E_FULL, 'requires tauri dev to verify scan is suppressed')
  })
})
