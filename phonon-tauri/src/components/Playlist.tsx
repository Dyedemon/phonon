import { useState, useEffect, useCallback, useRef } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { useI18n } from '../i18n'
import { getTrackByPath, type TrackInfo } from '../api/library'
import { type UserPlaylist } from '../api/queue'
import { FavoriteIcon } from './TrackMeta'

interface QueueItem {
  path: string
  title: string
  artist: string | null
  duration: number | null
}

interface AlbumArtResponse {
  data: number[]
  mime_type: string
}

interface Props {
  onUpdate: () => void
  compact?: boolean
  addToast: (message: string, kind?: 'info' | 'warn' | 'error') => void
}

export default function Playlist({ onUpdate, compact: _compact, addToast: _addToast }: Props) {
  const { t } = useI18n()
  const [queue, setQueue] = useState<QueueItem[]>([])
  const [selected, setSelected] = useState<Set<number>>(new Set())
  const [contextMenu, setContextMenu] = useState<{ x: number; y: number; index: number } | null>(null)
  const [trackInfoModal, setTrackInfoModal] = useState<{
    title: string | null; artist: string | null; album: string | null
    duration: number | null; sample_rate: number | null; channels: number | null
    format: string | null
  } | null>(null)
  const [dragIndex, setDragIndex] = useState<number | null>(null)
  const [activePlaylistName, setActivePlaylistName] = useState('')
  const [systemPlaylists, setSystemPlaylists] = useState<UserPlaylist[]>([])
  const [coverUrls, setCoverUrls] = useState<Map<string, string>>(new Map())
  const [batchMode, setBatchMode] = useState(false)
  const [showBatchMenu, setShowBatchMenu] = useState(false)
  const [searchQuery, setSearchQuery] = useState('')
  const contextRef = useRef<HTMLDivElement>(null)
  const batchRef = useRef<HTMLDivElement>(null)
  // path → TrackInfo, resolved lazily so the playlist can show
  // favorite/rating UI for tracks that exist in the library.
  const [trackMap, setTrackMap] = useState<Map<string, TrackInfo>>(new Map())

  const refresh = useCallback(async () => {
    try {
      const [items, activeName, sysList] = await Promise.all([
        invoke<QueueItem[]>('get_queue'),
        invoke<string>('get_active_playlist').catch(() => ''),
        invoke<UserPlaylist[]>('list_system_playlists').catch(() => []),
      ])
      setQueue(items)
      setActivePlaylistName(activeName)
      setSystemPlaylists(sysList)
    } catch (_) { /* ignore */ }
  }, [])

  useEffect(() => {
    refresh()
  }, [refresh])

  // 队列从任意来源变更（3D 控制台、拖入文件、清空等）都自动刷新——
  // 五个队列变更命令在 Rust 侧 emit 'queue-changed'
  useEffect(() => {
    let unlisten: (() => void) | null = null
    let cancelled = false
    listen('queue-changed', () => { void refresh() })
      .then((fn) => { if (cancelled) fn(); else unlisten = fn })
      .catch(() => {})
    return () => {
      cancelled = true
      if (unlisten) unlisten()
    }
  }, [refresh])

  // Resolve library TrackInfo for each queue item (path → TrackInfo).
  // We only fetch paths not already cached, and skip silently on failure
  // (tracks not in the library simply won't show fav/rating UI).
  useEffect(() => {
    let cancelled = false
    const missing = queue.filter((q) => !trackMap.has(q.path)).map((q) => q.path)
    if (missing.length === 0) return

    const resolve = async () => {
      const updates = new Map<string, TrackInfo>()
      // Sequential to avoid hammering the backend; the queue is usually
      // short and getTrackByPath is a cheap indexed lookup.
      for (const path of missing) {
        if (cancelled) return
        try {
          const info = await getTrackByPath(path)
          if (info) updates.set(path, info)
        } catch {
          /* not in library — skip */
        }
      }
      if (!cancelled && updates.size > 0) {
        setTrackMap((prev) => new Map([...prev, ...updates]))
      }
    }
    resolve()
    return () => { cancelled = true }
  }, [queue, trackMap])

  // Propagate a fav/rating change back into the local cache so the UI
  // updates instantly without a re-fetch.
  const handleTrackChanged = useCallback((updated: TrackInfo) => {
    setTrackMap((prev) => {
      const next = new Map(prev)
      next.set(updated.file_path, updated)
      return next
    })
  }, [])

  // Load album art for visible tracks.
  // Depend on actual queue paths (not just length) so reorders and
  // replacements trigger cover reloads and old URLs get revoked.
  const queuePaths = queue.map(q => q.path).join('\n')
  useEffect(() => {
    let cancelled = false
    const newlyCreated: string[] = []

    const loadCovers = async () => {
      const newUrls = new Map<string, string>()
      const oldKeys = new Set(coverUrls.keys())

      const paths = queuePaths.split('\n')
      for (const key of paths) {
        oldKeys.delete(key) // still needed, don't revoke
        if (coverUrls.has(key)) {
          newUrls.set(key, coverUrls.get(key)!)
          continue
        }
        try {
          const art = await invoke<AlbumArtResponse | null>('get_album_art', {
            path: key,
          })
          if (cancelled) return
          if (art) {
            const blob = new Blob([new Uint8Array(art.data)], { type: art.mime_type })
            const url = URL.createObjectURL(blob)
            newlyCreated.push(url)
            newUrls.set(key, url)
          }
        } catch (_) {
          /* no art */
        }
      }

      if (cancelled) return
      setCoverUrls(newUrls)

      // Revoke URLs that are no longer needed
      for (const oldKey of oldKeys) {
        const oldUrl = coverUrls.get(oldKey)
        if (oldUrl) URL.revokeObjectURL(oldUrl)
      }
    }

    loadCovers()

    return () => {
      cancelled = true
      // Revoke any URLs created during this effect run if it was cancelled
      for (const url of newlyCreated) {
        URL.revokeObjectURL(url)
      }
    }
  }, [queuePaths])

  // Close context menu on outside click
  useEffect(() => {
    const handler = () => setContextMenu(null)
    document.addEventListener('click', handler)
    return () => document.removeEventListener('click', handler)
  }, [])

  // Close batch menu on outside click
  useEffect(() => {
    if (!showBatchMenu) return
    const handler = (e: MouseEvent) => {
      if (batchRef.current && !batchRef.current.contains(e.target as Node)) {
        setShowBatchMenu(false)
      }
    }
    document.addEventListener('mousedown', handler)
    return () => document.removeEventListener('mousedown', handler)
  }, [showBatchMenu])

  // ── Selection & Actions ───────────────────────────────────

  const selectAll = () => {
    setSelected(new Set(queue.map((_, i) => i)))
    setShowBatchMenu(false)
  }

  const deselectAll = () => {
    setSelected(new Set())
    setShowBatchMenu(false)
  }

  const exitBatchMode = () => {
    setBatchMode(false)
    setSelected(new Set())
    setShowBatchMenu(false)
  }

  const toggleCheckbox = (idx: number) => {
    const next = new Set(selected)
    if (next.has(idx)) next.delete(idx)
    else next.add(idx)
    setSelected(next)
  }

  const toggleSelect = (idx: number, e: React.MouseEvent) => {
    const next = new Set(selected)
    if (e.ctrlKey || e.metaKey) {
      if (next.has(idx)) next.delete(idx)
      else next.add(idx)
    } else if (e.shiftKey && selected.size > 0) {
      const selArr = Array.from(selected).sort((a, b) => a - b)
      const last = selArr[selArr.length - 1]
      const [from, to] = idx < last ? [idx, last] : [last, idx]
      for (let i = from; i <= to; i++) next.add(i)
    } else {
      next.clear()
      next.add(idx)
    }
    setSelected(next)
  }

  const removeSelected = async () => {
    if (selected.size === 0) return
    await invoke('remove_from_queue', { indices: Array.from(selected) })
    exitBatchMode()
    refresh()
  }

  // ── Context Menu ──────────────────────────────────────────

  const handleContextMenu = (e: React.MouseEvent, idx: number) => {
    e.preventDefault()
    setContextMenu({ x: e.clientX, y: e.clientY, index: idx })
    if (!selected.has(idx)) {
      setSelected(new Set([idx]))
    }
  }

  const playSelected = async () => {
    const idx = contextMenu?.index
    if (idx === undefined) return
    await invoke('play_index', { index: idx })
    refresh()
    onUpdate()
    setContextMenu(null)
  }

  const removeItem = async () => {
    if (contextMenu) {
      await invoke('remove_from_queue', { indices: [contextMenu.index] })
      setSelected(new Set())
      refresh()
      setContextMenu(null)
    }
  }

  // ── Mouse-based Reorder (HTML5 drag broken in WebView2) ────

  const reorderPickRef = useRef<number | null>(null)

  // Cancel drag on global mouseup (e.g. user releases outside the list)
  useEffect(() => {
    const cancelDrag = () => {
      reorderPickRef.current = null
      setDragIndex(null)
    }
    window.addEventListener('mouseup', cancelDrag)
    return () => window.removeEventListener('mouseup', cancelDrag)
  }, [])

  // Pick up: mousedown on item records the source index
  const handleReorderMouseDown = (e: React.MouseEvent, idx: number) => {
    if (e.button !== 0) return
    if (systemPlaylists.some(pl => pl.name === activePlaylistName)) return
    e.stopPropagation()
    reorderPickRef.current = idx
    setDragIndex(idx)
  }

  // Drop: mouseup on any item completes the reorder
  const handleReorderMouseUp = async (_e: React.MouseEvent, to: number) => {
    const from = reorderPickRef.current
    reorderPickRef.current = null
    setDragIndex(null)
    if (from === null || from === to) return
    await invoke('reorder_queue', { from, to })
    refresh()
  }

  // ── Render ────────────────────────────────────────────────

  const formatDuration = (d: number | null) => {
    if (d === null) return '--:--'
    const m = Math.floor(d / 60)
    const s = Math.floor(d % 60)
    return `${m}:${s.toString().padStart(2, '0')}`
  }

  const formatExt = (path: string) => path.split('.').pop()?.toUpperCase() || ''

  return (
    <div>
      {/* Queue List */}
      <div className="card" style={{ padding: 0, overflow: 'visible' }}>
        {/* Batch header */}
        {queue.length > 0 && (
          <div style={{
            display: 'flex', alignItems: 'center', padding: '6px 10px',
            borderBottom: '1px solid var(--border)', gap: 8,
            position: 'sticky', top: 0, zIndex: 10,
            background: 'var(--bg-card)',
          }}>
            {/* Actions dropdown — only in batch mode */}
            {batchMode && (
              <div ref={batchRef} style={{ position: 'relative' }}>
                <button
                  className="btn btn-outline btn-sm"
                  onClick={(e) => { e.stopPropagation(); setShowBatchMenu(!showBatchMenu) }}
                  style={{ fontSize: 'var(--text-xs)' }}
                >
                  {t('playlist.actions', 'Actions')}
                </button>
                {showBatchMenu && (
                  <div className="batch-menu" style={{
                    position: 'absolute', top: '100%', left: 0, marginTop: 4,
                    background: 'var(--bg-card)', border: '1px solid var(--border-glow)',
                    borderRadius: 'var(--radius)', padding: 4, zIndex: 200,
                    boxShadow: '0 4px 16px rgba(0,0,0,0.5)', minWidth: 160, display: 'flex', flexDirection: 'column', gap: 2,
                  }}>
                    <button className="batch-item" onClick={selectAll}>
                      {t('playlist.selectAll', 'Select All')}
                    </button>
                    <button className="batch-item" onClick={deselectAll}>
                      {t('playlist.clearSelection', 'Clear Selection')}
                    </button>
                    <div style={{ height: 1, background: 'var(--border)', margin: '2px 0' }} />
                    <button className="batch-item batch-item-danger" onClick={removeSelected} disabled={selected.size === 0}>
                      {t('playlist.deleteSelected').replace('{n}', String(selected.size))}
                    </button>
                  </div>
                )}
              </div>
            )}

            {/* Toggle button: Batch / Done */}
            <button
              className={`btn btn-sm ${batchMode ? 'btn-primary' : 'btn-outline'}`}
              onClick={() => batchMode ? exitBatchMode() : setBatchMode(true)}
              style={{ fontSize: 'var(--text-xs)' }}
            >
              {batchMode ? t('playlist.done', 'Done') : t('playlist.batch', 'Batch')}
            </button>

            {/* Search input */}
            <input
              type="text"
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              placeholder={t('playlist.searchPlaceholder')}
              style={{
                flex: 1, maxWidth: 220, fontSize: 'var(--text-xs)', padding: '4px 8px',
                background: 'var(--bg)', border: '1px solid var(--border)',
                borderRadius: 4, color: 'var(--text)', outline: 'none',
              }}
              onFocus={(e) => e.target.style.borderColor = 'var(--accent)'}
              onBlur={(e) => e.target.style.borderColor = 'var(--border)'}
            />

            {/* Selected count / track count */}
            <span style={{ fontSize: 'var(--text-xs)', color: 'var(--text-dim)', marginLeft: 'auto' }}>
              {batchMode
                ? `${selected.size} ${t('playlist.selected')}`
                : `${queue.length} ${t('playlist.tracks')}`
              }
            </span>
          </div>
        )}
        {queue.length === 0 ? (
          <div className="empty-state">
            <div className="icon">{'\uD83C\uDFB5'}</div>
            <p>{t('playlist.noTracks', 'Playlist is empty')}</p>
            <p style={{ fontSize: 'var(--text-sm)' }}>{t('playlist.emptyHint', 'Add files, folders, or CUE sheets to begin')}</p>
          </div>
        ) : (
          <div>
            {(() => {
              const isFavorites = systemPlaylists.some(pl => pl.name === activePlaylistName && pl.system_id === 'favorites')
              return queue
                .map((item, idx) => ({ item, originalIndex: idx }))
                .filter(({ item }) => {
                  // Hide unfavorited tracks when viewing the favorites system playlist
                  if (isFavorites) {
                    const trackInfo = trackMap.get(item.path)
                    if (trackInfo && !trackInfo.is_favorite) return false
                  }
                if (!searchQuery.trim()) return true
                const q = searchQuery.toLowerCase()
                return (
                  item.title.toLowerCase().includes(q) ||
                  (item.artist || '').toLowerCase().includes(q) ||
                  item.path.toLowerCase().includes(q)
                )
              })
              .map(({ item, originalIndex }) => {
                const idx = originalIndex
                return (
              <div
                key={`${item.path}-${idx}`}
                className={`list-item${selected.has(idx) ? ' selected' : ''}${dragIndex === idx ? ' dragging' : ''}`}
                data-idx={idx}
                style={{
                  background: selected.has(idx) ? 'var(--bg-hover)' : undefined,
                  opacity: dragIndex === idx ? 0.5 : 1,
                  borderTop: undefined,
                  cursor: dragIndex === idx ? 'grabbing' : 'grab',
                }}
                onMouseDown={(e) => handleReorderMouseDown(e, idx)}
                onMouseUp={(e) => handleReorderMouseUp(e, idx)}
                onClick={(e) => {
                  if (batchMode) {
                    toggleCheckbox(idx)
                  } else {
                    toggleSelect(idx, e)
                  }
                }}
                onDoubleClick={async () => {
                  setSelected(new Set([idx]))
                  await invoke('play_index', { index: idx })
                  refresh()
                  onUpdate()
                }}
                onContextMenu={(e) => handleContextMenu(e, idx)}
              >
                {/* Checkbox — only visible in batch mode */}
                {batchMode && (
                  <input
                    type="checkbox"
                    className="list-checkbox"
                    checked={selected.has(idx)}
                    onChange={() => toggleCheckbox(idx)}
                    onClick={(e) => e.stopPropagation()}
                    draggable={false}
                  />
                )}

                {/* Album art thumbnail */}
                <div className="art-thumb" draggable={false}>
                  {coverUrls.has(item.path) ? (
                    <img src={coverUrls.get(item.path)} alt="" draggable={false} />
                  ) : (
                    <div className="art-placeholder" draggable={false}>{'\u266B'}</div>
                  )}
                </div>

                <span className="index" draggable={false}>{idx + 1}</span>
                <div className="info" draggable={false}>
                  <div className="title" draggable={false}>{item.title}</div>
                  <div className="subtitle" draggable={false}>
                    {item.artist || t('playlist.unknownArtist')}
                  </div>
                  <div className="meta" draggable={false}>
                    {formatDuration(item.duration)}
                    <span className="meta-sep">|</span>
                    {formatExt(item.path)}
                  </div>
                </div>

                {/* Favorite — only when the track is in the library. */}
                {trackMap.has(item.path) && (
                  <div
                    className="list-meta-actions"
                    style={{ display: 'flex', alignItems: 'center', gap: 6, paddingRight: 8 }}
                    draggable={false}
                    onMouseDown={(e) => e.stopPropagation()}
                  >
                    <FavoriteIcon
                      track={trackMap.get(item.path)!}
                      onChanged={handleTrackChanged}
                      compact
                      size={14}
                    />
                  </div>
                )}
              </div>
                )
              })
            })()}
          </div>
        )}
      </div>

      {/* Context Menu */}
      {contextMenu && (
        <div
          ref={contextRef}
          className="context-menu"
          style={{
            position: 'fixed',
            left: contextMenu.x,
            top: contextMenu.y,
            zIndex: 1000,
          }}
        >
          <div className="context-item" onClick={playSelected}>
            {t('playlist.playFromHere')}
          </div>
          <div className="context-item" onClick={removeItem}>
            {t('playlist.remove')}
          </div>
          <div className="context-separator" />
          <div className="context-item" onClick={async () => {
            const idx = contextMenu.index
            setContextMenu(null)
            try {
              const meta = await invoke<{
                title: string | null; artist: string | null; album: string | null
                duration: number | null; sample_rate: number | null; channels: number | null
                format: string | null
              }>('get_track_metadata', { path: queue[idx].path })
              setTrackInfoModal(meta)
            } catch (e) {
              console.error('Failed to get metadata:', e)
            }
          }}>
            {t('playlist.trackInfo')}
          </div>
        </div>
      )}

      {trackInfoModal && (
        <div className="modal-overlay" onClick={() => setTrackInfoModal(null)}>
          <div className="modal" onClick={e => e.stopPropagation()}>
            <h3 style={{ margin: '0 0 16px 0' }}>{t('trackInfo.title', 'Track Info')}</h3>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
              <div><strong>{t('trackInfo.titleName', 'Title')}:</strong> {trackInfoModal.title || t('playlist.unknown', 'Unknown')}</div>
              <div><strong>{t('trackInfo.artist', 'Artist')}:</strong> {trackInfoModal.artist || t('playlist.unknown', 'Unknown')}</div>
              <div><strong>{t('trackInfo.album', 'Album')}:</strong> {trackInfoModal.album || t('playlist.unknown', 'Unknown')}</div>
              <div><strong>{t('trackInfo.duration', 'Duration')}:</strong> {trackInfoModal.duration?.toFixed(1) || t('playlist.na', 'N/A')} s</div>
              <div><strong>{t('trackInfo.sampleRate', 'Sample Rate')}:</strong> {trackInfoModal.sample_rate || t('playlist.na', 'N/A')} Hz</div>
              <div><strong>{t('trackInfo.channels', 'Channels')}:</strong> {trackInfoModal.channels || t('playlist.na', 'N/A')}</div>
              <div><strong>{t('trackInfo.format', 'Format')}:</strong> {trackInfoModal.format || t('playlist.na', 'N/A')}</div>
            </div>
            <div style={{ marginTop: 20, textAlign: 'right' }}>
              <button className="btn btn-primary" onClick={() => setTrackInfoModal(null)}>
                {t('common.close', 'Close')}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}