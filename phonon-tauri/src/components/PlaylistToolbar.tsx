import { useState, useEffect, useCallback, useRef } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-dialog'
import { useI18n } from '../i18n'

interface UserPlaylist {
  name: string
  track_count: number
  /** Stable identifier for system playlists: "favorites" | "recent" | null */
  system_id?: string | null
}

interface Props {
  refreshTrigger?: number
  onUpdate: () => void
  addToast: (message: string, kind?: 'info' | 'warn' | 'error') => void
}

export default function PlaylistToolbar({ refreshTrigger, onUpdate, addToast }: Props) {
  const { t } = useI18n()
  const [playlists, setPlaylists] = useState<UserPlaylist[]>([])
  const [systemPlaylists, setSystemPlaylists] = useState<UserPlaylist[]>([])
  const [activePlaylist, setActivePlaylist] = useState('')
  const [showPlaylistMenu, setShowPlaylistMenu] = useState(false)
  const [showNewInput, setShowNewInput] = useState(false)
  const [newPlaylistName, setNewPlaylistName] = useState('')
  const [, setHoveredPlaylist] = useState<string | null>(null)
  const [dragIndex, setDragIndex] = useState<number | null>(null)
  const [renamingName, setRenamingName] = useState<string | null>(null)
  const [renameValue, setRenameValue] = useState('')
  const [syncedFolder, setSyncedFolder] = useState<string>('')
  const [confirmModal, setConfirmModal] = useState<{ title: string; message: string; onConfirm: () => void } | null>(null)
  const renameInputRef = useRef<HTMLInputElement>(null)
  const menuRef = useRef<HTMLDivElement>(null)

  const refreshPlaylists = useCallback(async () => {
    try {
      const [list, sysList] = await Promise.all([
        invoke<UserPlaylist[]>('list_playlists'),
        invoke<UserPlaylist[]>('list_system_playlists').catch(() => [] as UserPlaylist[]),
      ])
      setPlaylists(list)
      setSystemPlaylists(sysList)
      const active = await invoke<string>('get_active_playlist')
      setActivePlaylist(active)
      // 获取当前歌单绑定的同步文件夹
      if (active) {
        const synced = await invoke<Record<string, string>>('get_synced_folders')
        setSyncedFolder(synced[active] || '')
      } else {
        setSyncedFolder('')
      }
    } catch (_) { /* ignore */ }
  }, [])

  useEffect(() => {
    refreshPlaylists()
  }, [refreshPlaylists])

  useEffect(() => {
    if (refreshTrigger !== undefined) {
      refreshPlaylists()
    }
  }, [refreshTrigger, refreshPlaylists])

  // Close dropdown on outside click
  useEffect(() => {
    if (!showPlaylistMenu) return
    const handler = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setShowPlaylistMenu(false)
      }
    }
    document.addEventListener('mousedown', handler)
    return () => document.removeEventListener('mousedown', handler)
  }, [showPlaylistMenu])

  const handleCreatePlaylist = async () => {
    const name = newPlaylistName.trim()
    if (!name) return
    try {
      await invoke('create_playlist', { name })
      await invoke('switch_playlist', { name })
      setActivePlaylist(name)
      setShowNewInput(false)
      setNewPlaylistName('')
      refreshPlaylists()
      onUpdate()
      addToast(`${t('toast.playlistCreated')}: ${name}`)
    } catch (e) {
      addToast(String(e), 'error')
    }
  }

  const handleSwitchPlaylist = async (pl: UserPlaylist) => {
    try {
      if (pl.system_id) {
        await invoke('switch_system_playlist', { name: pl.name })
      } else {
        await invoke('switch_playlist', { name: pl.name })
      }
      setActivePlaylist(pl.name)
      setShowPlaylistMenu(false)
      refreshPlaylists()
      onUpdate()
    } catch (e) {
      addToast(String(e), 'error')
    }
  }

  const handleDeletePlaylist = (name: string) => {
    setConfirmModal({
      title: t('common.confirm'),
      message: `${t('playlist.deleteConfirm', '确定要删除')}"${name}"${t('playlist.deleteConfirmSuffix', '歌单吗？')}`,
      onConfirm: async () => {
        setConfirmModal(null)
        try {
          await invoke('delete_playlist', { name })
          if (activePlaylist === name) setActivePlaylist('')
          refreshPlaylists()
          onUpdate()
          addToast(`${t('toast.playlistDeleted')}: ${name}`)
        } catch (e) {
          addToast(String(e), 'error')
        }
      },
    })
  }

  const startRename = (name: string) => {
    setRenamingName(name)
    setRenameValue(name)
    setTimeout(() => renameInputRef.current?.focus(), 0)
  }

  const handleRename = async () => {
    if (!renamingName) return
    const newName = renameValue.trim()
    if (!newName || newName === renamingName) {
      setRenamingName(null)
      return
    }
    try {
      await invoke('rename_playlist', { oldName: renamingName, newName })
      if (activePlaylist === renamingName) setActivePlaylist(newName)
      refreshPlaylists()
      addToast(`${t('toast.renamedTo')}: ${newName}`)
    } catch (e) {
      addToast(String(e), 'error')
    } finally {
      setRenamingName(null)
    }
  }

  // Exit new playlist / rename editing on outside click
  useEffect(() => {
    if (!showNewInput && renamingName === null) return
    const handler = (e: MouseEvent) => {
      const target = e.target as Node
      if (menuRef.current && !menuRef.current.contains(target)) {
        if (showNewInput) {
          setShowNewInput(false)
          setNewPlaylistName('')
        }
        if (renamingName !== null) {
          handleRename()
        }
      }
    }
    const keyHandler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        if (showNewInput) {
          setShowNewInput(false)
          setNewPlaylistName('')
        }
        if (renamingName !== null) {
          setRenamingName(null)
        }
      }
    }
    document.addEventListener('mousedown', handler)
    document.addEventListener('keydown', keyHandler)
    return () => {
      document.removeEventListener('mousedown', handler)
      document.removeEventListener('keydown', keyHandler)
    }
  }, [showNewInput, renamingName])

  // ── Playlist drag reorder ──────────────────────────────────

  const handleDragStart = (index: number) => {
    setDragIndex(index)
  }

  const handleDragEnd = async (dropIndex: number) => {
    if (dragIndex === null || dragIndex === dropIndex) {
      setDragIndex(null)
      return
    }
    const newOrder = [...playlists.map(p => p.name)]
    const [moved] = newOrder.splice(dragIndex, 1)
    newOrder.splice(dropIndex, 0, moved)
    setPlaylists(prev => {
      const updated = [...prev]
      const [moved] = updated.splice(dragIndex, 1)
      updated.splice(dropIndex, 0, moved)
      return updated
    })
    setDragIndex(null)
    try {
      await invoke('reorder_playlists', { order: newOrder })
    } catch (e) {
      addToast(String(e), 'error')
      refreshPlaylists() // revert on failure
    }
  }

  const isSystemPlaylist = useCallback((name: string): boolean => {
    return systemPlaylists.some(pl => pl.name === name)
  }, [systemPlaylists])

  // ── File addition ──────────────────────────────────────────

  const sysBlock = (): boolean => {
    if (isSystemPlaylist(activePlaylist)) {
      addToast(t('playlist.systemNoAdd', '系统歌单不支持添加文件'), 'warn')
      return true
    }
    return false
  }

  const addFiles = async () => {
    if (sysBlock()) return
    try {
      const selected = await open({
        multiple: true,
        filters: [{
          name: 'Audio',
          extensions: ['mp3', 'flac', 'wav', 'ogg', 'aac', 'm4a', 'wma', 'opus', 'aiff', 'ape', 'wv', 'cue'],
        }],
      })
      if (!selected) return
      const paths = Array.isArray(selected) ? selected : [selected]
      const result = await invoke<{ added: number; duplicates: number }>('add_to_queue', { paths })
      onUpdate()
      if (result.added > 0) addToast(`${t('toast.added')} ${result.added} ${t('playlist.tracks')}`)
      if (result.duplicates > 0) addToast(`${result.duplicates} ${t('toast.alreadyInQueue')}`, 'warn')
    } catch (e) {
      addToast(String(e), 'error')
    }
  }

  // 导入文件夹 — 一次性扫描，不自动监听
  const importFolder = async () => {
    if (sysBlock()) return
    try {
      const selected = await open({ directory: true, multiple: false })
      if (!selected) return
      const path = selected as string

      // §5.8.1 import fast path: if the picked folder already lives
      // under a configured library root, we can attach full metadata to
      // the queue items by resolving paths via library_get_tracks_by_paths.
      // PARTIAL: the legacy add_to_queue backend accepts only paths
      // (no metadata payload), so we can't fully eliminate placeholder
      // QueueItems here. We fetch library rows for diagnostics (so the
      // user sees "N 首已匹配媒体库 metadata") but still call the legacy
      // scan_folder + add_to_queue flow. Full fast path requires backend
      // wiring add_to_queue to consult the library in-process — out of
      // scope for this RC1 frontend task.
      try {
        const results = await invoke<{ uri: string; name: string }[]>('scan_folder', { path })
        const paths = results.map((r: { uri: string }) => r.uri)
        if (paths.length === 0) return

        // Best-effort: check how many paths match existing library metadata
        try {
          const roots = await invoke<string[]>('library_get_roots').catch(() => [] as string[])
          const underRoot = roots.some((r) => {
            const norm = (s: string) => s.replace(/\\/g, '/').replace(/\/$/, '')
            const np = norm(path)
            const nr = norm(r)
            return np === nr || np.startsWith(nr + '/')
          })
          if (underRoot) {
            const libRows = await invoke<unknown[]>('library_get_tracks_by_paths', { paths }).catch(() => [])
            addToast(`${libRows.length}/${paths.length} ${t('toast.added', '已匹配媒体库 metadata')}`)
          }
        } catch (_) { /* ignore — fast path is best-effort */ }

        const result = await invoke<{ added: number; duplicates: number }>('add_to_queue', { paths })
        onUpdate()
        if (result.added > 0) addToast(`${t('toast.added')} ${result.added} ${t('playlist.tracks')}`)
        if (result.duplicates > 0) addToast(`${result.duplicates} ${t('toast.alreadyInQueue')}`, 'warn')
      } catch (e) {
        addToast(String(e), 'error')
      }
    } catch (e) {
      addToast(String(e), 'error')
    }
  }

  // 同步文件夹 — 扫描并绑定到当前歌单，自动监听文件变化
  // 已绑定时点击则弹出确认解除绑定
  const syncFolder = async () => {
    if (sysBlock()) return
    try {
      if (!activePlaylist) {
        addToast(t('playlist.syncNeedPlaylist', '请先选择歌单'), 'warn')
        return
      }
      // 已绑定 — 弹出确认解除
      if (syncedFolder) {
        const folderName = syncedFolder.split(/[\\/]/).pop() || syncedFolder
        setConfirmModal({
          title: t('playlist.unsyncTitle', '解除同步绑定'),
          message: `${t('playlist.confirmUnsync', '确认解除当前绑定的同步文件夹：')}${folderName}`,
          onConfirm: async () => {
            setConfirmModal(null)
            await invoke('unsync_folder', { playlist: activePlaylist })
            setSyncedFolder('')
            onUpdate()
            addToast(t('toast.unsynced', '已解除同步绑定'))
          },
        })
        return
      }
      // 未绑定 — 选择文件夹并绑定
      const selected = await open({ directory: true, multiple: false })
      if (!selected) return
      const path = selected as string
      const results = await invoke<{ uri: string; name: string }[]>('scan_folder', { path })
      const paths = results.map((r: { uri: string }) => r.uri)
      if (paths.length === 0) {
        addToast(t('toast.noAudioFound', '未找到支持的音频文件'), 'warn')
        return
      }
      const result = await invoke<{ added: number; duplicates: number }>('add_to_queue', { paths })
      // sync_folder must succeed — don't swallow errors
      await invoke('sync_folder', { playlist: activePlaylist, path })
      setSyncedFolder(path)
      onUpdate()

      // ── §5.8.1 hook — auto-add the synced folder to library roots ──
      let rootsAdded = false
      try {
        const s = await invoke<{ library?: { auto_sync_playlist_folder_to_roots?: boolean } }>('get_settings')
        const enabled = s?.library?.auto_sync_playlist_folder_to_roots ?? true
        if (enabled) {
          const existing = await invoke<string[]>('library_get_roots').catch(() => [] as string[])
          const norm = (p: string) => p.replace(/\\/g, '/').replace(/\/$/, '')
          const np = norm(path)
          const already = existing.some((r) => {
            const nr = norm(r)
            return np === nr || np.startsWith(nr + '/') || nr.startsWith(np + '/')
          })
          if (!already) {
            const merged = Array.from(new Set([...existing, path]))
            await invoke('library_set_roots', { roots: merged })
            rootsAdded = true
          }
          await invoke('library_scan').catch(() => {})
        }
      } catch { /* best-effort */ }

      // Single consolidated toast
      const parts: string[] = []
      if (result.added > 0) parts.push(`${result.added} ${t('playlist.tracks')}`)
      if (result.duplicates > 0) parts.push(`${result.duplicates} ${t('toast.alreadyInQueue')}`)
      if (parts.length === 0) parts.push(t('toast.synced', '已同步'))
      const msg = rootsAdded
        ? `${t('toast.syncedAndScanned', '已同步并扫描媒体库')} · ${parts.join('，')}`
        : `${t('toast.synced', '已同步')} · ${parts.join('，')}`
      addToast(msg)
    } catch (e) {
      addToast(String(e), 'error')
    }
  }

  const addCueFile = async () => {
    if (sysBlock()) return
    try {
      const selected = await open({
        multiple: false,
        filters: [{ name: 'CUE Sheet', extensions: ['cue'] }],
      })
      if (!selected) return
      const path = Array.isArray(selected) ? selected[0] : selected
      const result = await invoke<{ added: number; duplicates: number }>('add_cue_to_queue', { path })
      onUpdate()
      if (result.added > 0) addToast(`${t('toast.added')} ${result.added} ${t('toast.fromCue')}`)
      if (result.duplicates > 0) addToast(`${result.duplicates} ${t('toast.alreadyInQueue')}`, 'warn')
    } catch (e) {
      addToast(String(e), 'error')
    }
  }

  const btnStyle = { fontSize: 'var(--text-sm)', padding: '5px 10px' }
  const displayName = activePlaylist || t('playlist.playlists')

  const iconSvg = (d: string) => (
    <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" style={{ verticalAlign: 'middle', marginRight: 4 }}>
      <path d={d} />
    </svg>
  )

  return (
    <div ref={menuRef} className="playlist-toolbar">
      {/* Add buttons — always visible; clicking during system playlist shows a hint */}
      <div className="playlist-add-btns">
        <button className="btn btn-primary btn-sm" style={btnStyle} onClick={addFiles}>
          {iconSvg('M12 5v14M5 12h14')}{t('playlist.addFiles', 'Add Files')}
        </button>
        <button className="btn btn-outline btn-sm" style={btnStyle} onClick={importFolder}>
          {iconSvg('M22 19a2 2 0 01-2 2H4a2 2 0 01-2-2V5a2 2 0 012-2h5l2 3h9a2 2 0 012 2z')}{t('playlist.importFolder', '导入文件夹')}
        </button>
        <button
          className="btn btn-sm btn-outline"
          style={btnStyle}
          onClick={syncFolder}
          {...(syncedFolder ? { 'data-tooltip': `${t('playlist.syncedFolder', '已绑定')}: ${syncedFolder.split(/[\\/]/).pop() || syncedFolder}` } : {})}
        >
          {syncedFolder
            ? iconSvg('M18 6L6 18M6 6l12 12')
            : iconSvg('M21 12a9 9 0 11-6.219-8.56 M21 3v6h-6')}
          {syncedFolder
            ? t('playlist.unsyncFolder', '解除绑定')
            : t('playlist.syncFolder', '同步文件夹')}
        </button>
        <button className="btn btn-outline btn-sm" style={btnStyle} onClick={addCueFile}>
          {iconSvg('M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z M14 2v6h6 M16 13H8 M16 17H8 M10 9H8')}{t('playlist.addCue', 'Add CUE')}
        </button>
      </div>

      {/* System playlists — favorites & recent */}
      <div className="playlist-system-section">
        {systemPlaylists.map((pl) => {
          const isFav = pl.system_id === 'favorites'
          const isRec = pl.system_id === 'recent'
          return (
            <div
              key={pl.name}
              className={`playlist-item ${activePlaylist === pl.name ? 'active' : ''} system-playlist`}
              onMouseEnter={() => setHoveredPlaylist(pl.name)}
              onMouseLeave={() => setHoveredPlaylist(null)}
            >
              <button
                className="playlist-item-btn"
                onClick={() => handleSwitchPlaylist(pl)}
                style={{ cursor: 'pointer' }}
              >
                {isFav && <span style={{ marginRight: 8, color: 'var(--accent)', fontSize: 'var(--text-base)' }}>{'\u2665'}</span>}
                {isRec && <span style={{ marginRight: 8, opacity: 0.7, fontSize: 'var(--text-base)' }}>{'\u23F1'}</span>}
                <span className="playlist-item-name">{pl.name}</span>
                <span className="playlist-item-count" style={{ marginLeft: 'auto' }}>{pl.track_count}{t('playlist.trackUnit', '首')}</span>
              </button>
            </div>
          )
        })}
      </div>

      <div className="playlist-divider" />

      {/* Playlist expand/collapse header */}
      <div
        className="playlist-header"
        onClick={() => setShowPlaylistMenu(!showPlaylistMenu)}
      >
        <span className="playlist-header-title">{displayName}</span>
        <span className="playlist-header-arrow">
          {showPlaylistMenu ? '\u25BC' : '\u25B6'}
        </span>
      </div>

      {/* Playlist list — expands below header */}
      {showPlaylistMenu && (
        <div className="playlist-list">
          {playlists.map((pl, index) => {
            return (
            <div
              key={pl.name}
              className={`playlist-item ${activePlaylist === pl.name ? 'active' : ''}`}
              style={{ opacity: dragIndex === index ? 0.4 : 1 }}
              onMouseEnter={() => setHoveredPlaylist(pl.name)}
              onMouseLeave={() => setHoveredPlaylist(null)}
              onMouseDown={() => handleDragStart(index)}
              onMouseUp={() => handleDragEnd(index)}
            >
              {renamingName === pl.name ? (
                <input
                  ref={renameInputRef}
                  type="text"
                  value={renameValue}
                  onChange={(e) => setRenameValue(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter') handleRename()
                    if (e.key === 'Escape') setRenamingName(null)
                  }}
                  onBlur={handleRename}
                  onMouseDown={(e) => e.stopPropagation()}
                  onClick={(e) => e.stopPropagation()}
                  className="playlist-rename-input"
                />
              ) : (
                <button
                  className="playlist-item-btn"
                  onClick={() => handleSwitchPlaylist(pl)}
                  onMouseDown={(e) => e.stopPropagation()}
                  style={{ cursor: dragIndex !== null ? 'grabbing' : 'pointer' }}
                >
                  <span className="playlist-item-name">{pl.name}</span>
                </button>
              )}
              {renamingName !== pl.name && (
                <div className="playlist-item-actions">
                  <button
                    className="playlist-action-btn"
                    onClick={(e) => { e.stopPropagation(); startRename(pl.name) }}
                    onMouseDown={(e) => e.stopPropagation()}
                  >
                    ✎
                  </button>
                  <button
                    className="playlist-action-btn"
                    onClick={(e) => { e.stopPropagation(); handleDeletePlaylist(pl.name) }}
                    onMouseDown={(e) => e.stopPropagation()}
                  >
                    &times;
                  </button>
                </div>
              )}
              {renamingName !== pl.name && (
                <span className="playlist-item-count">{pl.track_count}{t('playlist.trackUnit', '首')}</span>
              )}
            </div>
            )
          })}
          {playlists.length === 0 && (
            <div className="playlist-empty">{t('playlist.noPlaylists', '暂无歌单')}</div>
          )}
          <div className="playlist-divider" />
          {showNewInput ? (
            <div className="playlist-new-input">
              <input
                type="text"
                value={newPlaylistName}
                onChange={(e) => setNewPlaylistName(e.target.value)}
                onKeyDown={(e) => { if (e.key === 'Enter') handleCreatePlaylist() }}
                placeholder={t('playlist.namePlaceholder', '歌单名称')}
                className="playlist-new-input-field"
                autoFocus
              />
              <button className="btn btn-primary btn-sm" style={btnStyle} onClick={handleCreatePlaylist}>
                {t('common.create', '创建')}
              </button>
            </div>
          ) : (
            <button
              className="btn btn-outline btn-sm playlist-new-btn"
              style={btnStyle}
              onClick={() => { setShowNewInput(true); setNewPlaylistName('') }}
            >
              + {t('playlist.newPlaylist', 'New Playlist')}
            </button>
          )}
        </div>
      )}

      {/* Confirm modal */}
      {confirmModal && (
        <div
          style={{
            position: 'fixed', inset: 0, zIndex: 10000,
            background: 'rgba(0,0,0,0.6)', backdropFilter: 'blur(4px)',
            display: 'flex', alignItems: 'center', justifyContent: 'center',
          }}
          onClick={() => setConfirmModal(null)}
        >
          <div
            style={{
              background: 'var(--bg-card)', border: '1px solid var(--border-glow)',
              borderRadius: 'var(--radius)', padding: '20px 24px',
              minWidth: 320, maxWidth: 420,
              boxShadow: '0 12px 40px rgba(0,0,0,0.5)',
            }}
            onClick={(e) => e.stopPropagation()}
          >
            <h3 style={{ margin: '0 0 8px', fontSize: 'var(--text-lg)' }}>{confirmModal.title}</h3>
            <p style={{ margin: '0 0 16px', fontSize: 'var(--text-base)', color: 'var(--text-dim)', whiteSpace: 'pre-wrap', lineHeight: 1.6 }}>
              {confirmModal.message}
            </p>
            <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 8 }}>
              <button className="btn btn-outline" onClick={() => setConfirmModal(null)}>{t('common.cancel')}</button>
              <button className="btn btn-primary" onClick={confirmModal.onConfirm}>{t('common.confirm')}</button>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}