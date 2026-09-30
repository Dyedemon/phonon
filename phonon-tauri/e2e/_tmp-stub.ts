export function installStub(pluginSrc: string) {
  try {
    var LS0 = window.localStorage;
    LS0.setItem('phonon-enabled-vis-scripts', JSON.stringify(['vis_mode.js']));
  } catch (_e) { /* ignore */ }
  (window as any).__VIS_PLUGIN_SRC__ = pluginSrc;
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
      'prepare_for_reload': null, 'read_vis_script': window.__VIS_PLUGIN_SRC__, 'list_system_playlists': [], 'get_playlists': [], 'library_get_albums': [], 'get_volume_level': 0.8, 'sync_volume_from_device': 0.8, 'get_volume_mode': 'Software', 'get_hw_volume': null, 'set_window_title': null, 'get_hw_volume': null,
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
    (window as any).__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {}, registerListener: function () { return Promise.resolve(1); } };
    (window as any).__TAURI_STUB__ = true;
}
