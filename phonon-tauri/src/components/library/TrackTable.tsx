/**
 * TrackTable — 9-column virtual track list (Plan A §5.3).
 *
 * Columns:
 *   1. Cover 64×64 (IntersectionObserver lazy load + retry — see §5.10 #1)
 *   2. Track #
 *   3. 标题
 *   4. 艺术家
 *   5. 专辑
 *   6. 时长 mm:ss
 *   7. 格式 badge (lossy/lossless color)
 *   8. ★ favorite (clickable inline — optimistic) — OR Last Played
 *      relative time when subTab === 'recent'
 *   9. ⭐ rating 0-5 stars (clickable inline)
 *
 * Virtual list: ≥10000 rows auto-enables. Fixed 72px row height.
 *   overScanRows = Math.ceil(containerHeight / 72) * 3   (3-viewport buffer)
 *   top/bottom spacer divs; visible window = [firstIdx - overScan, lastIdx + overScan)
 *   <10000 rows: render as plain DOM (no spacer divs).
 *
 * 9-item context menu: play / add to queue / add to playlist / ★ favorite /
 *   ⭐ rating submenu / edit metadata / 更换封面 / open file location /
 *   rescan metadata.
 *
 * Currently-playing highlight: row.track_id === currentlyPlayingId → accent.
 *
 * Resilience (§5.10 #1): cover load failure → exponential backoff
 *   300ms → 900ms → 2700ms, 3 retries total; permanent 🎵 placeholder
 *   after that. Single-line WARN console.log (no global throw).
 */
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useI18n } from '../../i18n'
import * as libraryApi from '../../api/library'
import type { LibraryTrack } from '../../api/library'

const ROW_HEIGHT = 72
const VIRTUAL_THRESHOLD = 10000

export interface TrackTableProps {
  tracks: LibraryTrack[]
  /** Track currently playing — its row gets an accent highlight. */
  currentlyPlayingId?: number | null
  /** Sub-tab context — only 'recent' differs (col 8 swaps to Last Played). */
  subTab?: 'all' | 'favorites' | 'recent'
  /** Open the metadata editor for a track id. */
  onEditMetadata?: (trackId: number) => void
  /** Play a track (replace queue + start). */
  onPlayTrack?: (trackId: number) => void
  /** Play Next — insert the selection after the current track. */
  onPlayNext?: (ids: number[]) => void
  /** Append the selection to the queue. */
  onAddToQueue?: (ids: number[]) => void
}

type ContextMenuState = {
  /** Operate on the whole selection (single row = [id]). */
  ids: number[]
  x: number
  y: number
} | null

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function fmtDuration(ms: number | null | undefined): string {
  if (ms == null || ms <= 0) return '--:--'
  const total = Math.round(ms / 1000)
  const m = Math.floor(total / 60)
  const s = total % 60
  return `${m}:${s.toString().padStart(2, '0')}`
}

function fmtRelative(unixSec: number | null | undefined, t: (k: string, f?: string) => string): string {
  if (unixSec == null || unixSec <= 0) return '—'
  const now = Math.floor(Date.now() / 1000)
  const diff = now - unixSec
  if (diff < 60) return t('library.justNow')
  if (diff < 3600) {
    const m = Math.floor(diff / 60)
    return t('library.minutesAgo').replace('{n}', String(m))
  }
  if (diff < 86400) {
    const h = Math.floor(diff / 3600)
    return t('library.hoursAgo').replace('{n}', String(h))
  }
  if (diff < 86400 * 7) {
    const d = Math.floor(diff / 86400)
    return t('library.daysAgo').replace('{n}', String(d))
  }
  const w = Math.floor(diff / (86400 * 7))
  return t('library.weeksAgo').replace('{n}', String(w))
}

function formatBadge(fmt: string | null | undefined): { label: string; cls: string } {
  const f = (fmt || '').toLowerCase()
  // lossless
  if (['flac', 'alac', 'wav', 'aiff', 'dsd', 'dsf'].includes(f)) {
    return { label: (fmt || 'FLAC').toUpperCase(), cls: 'fmt-lossless' }
  }
  // lossy
  if (['mp3', 'aac', 'm4a', 'ogg', 'opus', 'wma'].includes(f)) {
    return { label: (fmt || 'MP3').toUpperCase(), cls: 'fmt-lossy' }
  }
  return { label: fmt || '?', cls: 'fmt-unknown' }
}

// ---------------------------------------------------------------------------
// Cover cell with retry (§5.10 #1)
// ---------------------------------------------------------------------------

const RETRY_DELAYS = [300, 900, 2700] // exponential backoff

function CoverCell({ hash, title }: { hash: string | null; title: string | null }) {
  const [url, setUrl] = useState<string | null>(null)
  const [failed, setFailed] = useState(false)
  const attemptRef = useRef(0)
  const [retryTick, setRetryTick] = useState(0)

  // Fetch on mount + on retryTick change. Single attempt = fetch thumbnail
  // URL; on failure schedule next retry with exponential backoff.
  useEffect(() => {
    let cancelled = false
    if (!hash || failed) return
    void libraryApi.getThumbnailUrl(hash, 256).then((u) => {
      if (cancelled) return
      if (u) setUrl(u)
      else {
        // 3 retries total. After 3 → permanent placeholder.
        if (attemptRef.current < RETRY_DELAYS.length) {
          const delay = RETRY_DELAYS[attemptRef.current]
          attemptRef.current += 1
          // Single-line WARN per spec §5.10 #1 (no global throw).
          console.warn(`[Library] cover load failed (hash=${hash}), retry ${attemptRef.current}/${RETRY_DELAYS.length} in ${delay}ms`)
          window.setTimeout(() => setRetryTick((n) => n + 1), delay)
        } else {
          setFailed(true)
        }
      }
    })
    return () => { cancelled = true }
  }, [hash, failed, retryTick])

  if (failed) {
    return (
      <div className="track-cover placeholder" aria-label="no cover">
        <span style={{ fontSize: 22, opacity: 0.5 }}>{'\uD83C\uDFB5'}</span>
      </div>
    )
  }
  if (url) {
    return <img className="track-cover" src={url} alt={title || ''} loading="lazy" />
  }
  return <div className="track-cover placeholder" />
}

// ---------------------------------------------------------------------------
// Lazy-rendered row cover — IntersectionObserver ensures we only attach
// the CoverCell (and trigger its thumbnail fetch) when the row scrolls
// near the viewport.
// ---------------------------------------------------------------------------

function LazyCover({ hash, title }: { hash: string | null; title: string | null }) {
  const ref = useRef<HTMLDivElement>(null)
  const [inView, setInView] = useState(false)
  useEffect(() => {
    const el = ref.current
    if (!el) return
    if (inView) return
    const io = new IntersectionObserver(
      (entries) => {
        for (const e of entries) {
          if (e.isIntersecting) {
            setInView(true)
            io.disconnect()
            break
          }
        }
      },
      { rootMargin: '200px 0px' } // start fetching slightly before visible
    )
    io.observe(el)
    return () => io.disconnect()
  }, [inView])

  return (
    <div ref={ref} style={{ width: 64, height: 64 }}>
      {inView ? <CoverCell hash={hash} title={title} /> : <div className="track-cover placeholder" />}
    </div>
  )
}

// ---------------------------------------------------------------------------
// Main table
// ---------------------------------------------------------------------------

export default function TrackTable({
  tracks,
  currentlyPlayingId,
  subTab = 'all',
  onPlayTrack,
  onPlayNext,
  onAddToQueue,
  onEditMetadata,
}: TrackTableProps) {
  const { t } = useI18n()
  const [scrollTop, setScrollTop] = useState(0)
  const [viewportH, setViewportH] = useState(600)
  const [ctxMenu, setCtxMenu] = useState<ContextMenuState>(null)
  const [tracksState, setTracksState] = useState(tracks)
  // 批量选择：单击=单选，Ctrl/Cmd=加减选，Shift=范围选。右键菜单作用于
  // 整个选择集（单选时等价于旧行为）。
  const [selectedIds, setSelectedIds] = useState<Set<number>>(() => new Set())
  const [anchorId, setAnchorId] = useState<number | null>(null)

  // Keep local state synced when parent passes a fresh array (e.g. after
  // an optimistic update flows back through props). Use ref to detect
  // prop identity change. Selection is cleared too — stale ids would point
  // at rows that may no longer exist in this view.
  useEffect(() => {
    setTracksState(tracks)
    setSelectedIds(new Set())
    setAnchorId(null)
  }, [tracks])

  const containerRef = useRef<HTMLDivElement>(null)

  // Throttle scroll-driven state updates with rAF.
  const rafRef = useRef<number | null>(null)
  const handleScroll = useCallback((e: React.UIEvent<HTMLDivElement>) => {
    const top = e.currentTarget.scrollTop
    const h = e.currentTarget.clientHeight
    if (rafRef.current != null) return
    rafRef.current = window.requestAnimationFrame(() => {
      rafRef.current = null
      setScrollTop(top)
      setViewportH(h)
    })
  }, [])

  useEffect(() => {
    const c = containerRef.current
    if (c) setViewportH(c.clientHeight)
  }, [])

  // Close context menu on outside click / escape.
  useEffect(() => {
    if (!ctxMenu) return
    const onDoc = () => setCtxMenu(null)
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') setCtxMenu(null) }
    document.addEventListener('click', onDoc)
    document.addEventListener('keydown', onKey)
    return () => {
      document.removeEventListener('click', onDoc)
      document.removeEventListener('keydown', onKey)
    }
  }, [ctxMenu])

  const total = tracksState.length
  const useVirtual = total >= VIRTUAL_THRESHOLD
  const overScanRows = Math.ceil(viewportH / ROW_HEIGHT) * 3
  const firstVisible = Math.floor(scrollTop / ROW_HEIGHT)
  const lastVisible = Math.floor((scrollTop + viewportH) / ROW_HEIGHT)
  const startIdx = useVirtual ? Math.max(0, firstVisible - overScanRows) : 0
  const endIdx = useVirtual ? Math.min(total, lastVisible + overScanRows) : total

  // Optimistic toggle favorite.
  const toggleFav = useCallback(async (id: number, current: boolean) => {
    if (subTab === 'favorites' && current) {
      setTracksState((arr) => arr.filter((tr) => tr.id !== id))
    } else {
      setTracksState((arr) => arr.map((tr) => tr.id === id ? { ...tr, is_favorite: !current } : tr))
    }
    try {
      await libraryApi.toggleFavorite(id)
      window.dispatchEvent(new CustomEvent('library-changed'))
    } catch (e) {
      // rollback — re-fetch from parent to restore correct state
      setTracksState(tracks)
      console.warn(`[Library] toggleFavorite failed for ${id}: ${e}`)
    }
  }, [subTab, tracks])

  const renderRow = (track: LibraryTrack, idx: number) => {
    const playing = currentlyPlayingId === track.id
    const selected = selectedIds.has(track.id)
    const badge = formatBadge(track.format)
    return (
      <div
        key={track.id}
        className={`track-row ${playing ? 'playing' : ''} ${selected ? 'selected' : ''}`}
        style={{ height: ROW_HEIGHT }}
        onClick={(e) => {
          if (e.ctrlKey || e.metaKey) {
            setSelectedIds((prev) => {
              const next = new Set(prev)
              if (next.has(track.id)) next.delete(track.id)
              else next.add(track.id)
              return next
            })
            setAnchorId(track.id)
          } else if (e.shiftKey && anchorId != null) {
            const from = tracksState.findIndex((tr) => tr.id === anchorId)
            const to = tracksState.findIndex((tr) => tr.id === track.id)
            if (from >= 0 && to >= 0) {
              const [lo, hi] = from < to ? [from, to] : [to, from]
              setSelectedIds(new Set(tracksState.slice(lo, hi + 1).map((tr) => tr.id)))
            }
          } else {
            setSelectedIds(new Set([track.id]))
            setAnchorId(track.id)
          }
        }}
        onContextMenu={(e) => {
          e.preventDefault()
          // 右键落在选择集内 → 对整个选择集操作；否则重置为该行。
          const ids = selectedIds.has(track.id) ? [...selectedIds] : [track.id]
          if (!selectedIds.has(track.id)) {
            setSelectedIds(new Set([track.id]))
            setAnchorId(track.id)
          }
          setCtxMenu({ ids, x: e.clientX, y: e.clientY })
        }}
        onDoubleClick={() => onPlayTrack?.(track.id)}
      >
        {/* 1. Cover */}
        <div className="col-cover">
          <LazyCover hash={track.cover_hash} title={track.title} />
        </div>
        {/* 2. Track # */}
        <div className="col-num">{track.track_number ?? (idx + 1)}</div>
        {/* 3. Title */}
        <div className="col-title" title={track.title ?? ''}>
          {track.title || t('library.unknownTrack')}
        </div>
        {/* 4. Artist */}
        <div className="col-artist" title={track.artist ?? ''}>
          {track.artist || '—'}
        </div>
        {/* 5. Album */}
        <div className="col-album" title={track.album ?? ''}>
          {track.album || '—'}
        </div>
        {/* 6. Duration */}
        <div className="col-duration">{fmtDuration(track.duration_ms)}</div>
        {/* 7. Format badge */}
        <div className="col-format">
          <span className={`fmt-badge ${badge.cls}`}>{badge.label}</span>
        </div>
        {/* 8. ★ favorite OR Last Played (recent) */}
        <div className="col-fav">
          {subTab === 'recent' ? (
            <span className="last-played">{fmtRelative(track.last_played_at, t)}</span>
          ) : (
            <button
              type="button"
              className={`fav-btn ${track.is_favorite ? 'on' : ''}`}
              onClick={(e) => {
                e.stopPropagation()
                void toggleFav(track.id, track.is_favorite)
              }}
              aria-label={track.is_favorite ? t('library.unfavorite') : t('library.favorite')}
            >
              {/* 与播放器 FavoriteIcon 一致的爱心（此前是星形，两处不一致） */}
              {track.is_favorite ? '\u2665' : '\u2661'}
            </button>
          )}
        </div>
      </div>
    )
  }

  const visibleSlice = useMemo(
    () => tracksState.slice(startIdx, endIdx),
    [tracksState, startIdx, endIdx],
  )

  const topSpacerH = useVirtual ? startIdx * ROW_HEIGHT : 0
  const bottomSpacerH = useVirtual ? (total - endIdx) * ROW_HEIGHT : 0

  return (
    <div className="track-table-wrap" style={{ height: '100%', minHeight: 0, display: 'flex', flexDirection: 'column' }}>
      {/* Header row */}
      <div className="track-row track-header" style={{ height: 36, fontWeight: 600, fontSize: 'var(--text-sm)', opacity: 0.7 }}>
        <div className="col-cover">{t('library.colCover', '◐')}</div>
        <div className="col-num">#</div>
        <div className="col-title">{t('library.colTitle', '标题')}</div>
        <div className="col-artist">{t('library.subtab.artists')}</div>
        <div className="col-album">{t('library.subtab.albums')}</div>
        <div className="col-duration">⏱</div>
        <div className="col-format">fmt</div>
        <div className="col-fav">{subTab === 'recent' ? t('library.lastPlayed') : '★'}</div>
      </div>

      <div
        ref={containerRef}
        className="track-table-body"
        onScroll={handleScroll}
        style={{ flex: 1, minHeight: 0, overflowY: 'auto', position: 'relative' }}
      >
        {total === 0 ? (
          <div className="empty-state" style={{ padding: 24, textAlign: 'center' }}>
            <div className="icon">{'\uD83C\uDFB5'}</div>
            <p>{t('library.historyEmpty')}</p>
          </div>
        ) : (
          <>
            {useVirtual && <div style={{ height: topSpacerH, pointerEvents: 'none' }} />}
            {visibleSlice.map((tr, i) => renderRow(tr, startIdx + i))}
            {useVirtual && <div style={{ height: bottomSpacerH, pointerEvents: 'none' }} />}
          </>
        )}
      </div>

      {/* Context menu portal */}
      {ctxMenu && (
        <ContextMenu
          ids={ctxMenu.ids}
          x={ctxMenu.x}
          y={ctxMenu.y}
          t={t}
          onClose={() => setCtxMenu(null)}
          onPlay={onPlayTrack}
          onPlayNext={onPlayNext}
          onAddToQueue={onAddToQueue}
          onEditMetadata={onEditMetadata}
        />
      )}
    </div>
  )
}

// ---------------------------------------------------------------------------
// Context menu (9 items)
// ---------------------------------------------------------------------------

interface ContextMenuProps {
  /** Operate on the whole selection (single row = [id]). */
  ids: number[]
  x: number
  y: number
  t: (k: string, f?: string) => string
  onClose: () => void
  onPlay?: (id: number) => void
  onPlayNext?: (ids: number[]) => void
  onAddToQueue?: (ids: number[]) => void
  onEditMetadata?: (id: number) => void
}

function ContextMenu({
  ids, x, y, t, onClose,
  onPlay, onPlayNext, onAddToQueue, onEditMetadata,
}: ContextMenuProps) {
  // Clamp to viewport so the menu doesn't get clipped off-screen.
  const cx = Math.min(x, window.innerWidth - 240)
  const cy = Math.min(y, window.innerHeight - 360)
  const batchSuffix = ids.length > 1 ? ` (${ids.length})` : ''
  const first = ids[0]
  // `disabled` = 前后端链路尚未接通的功能，灰显避免"点了没反应"。
  const items: { label: string; action?: () => void; separatorAfter?: boolean; disabled?: boolean }[] = [
    { label: t('context.play', '播放'), action: () => onPlay?.(first) },
    { label: t('context.playNext', '下一首播放') + batchSuffix, action: () => onPlayNext?.(ids) },
    { label: t('context.addToQueue', '加入队列') + batchSuffix, action: () => onAddToQueue?.(ids) },
    { label: t('context.addToPlaylist', '加入歌单'), disabled: true },
    { label: '★ ' + t('library.favorite'), separatorAfter: ids.length === 1, disabled: ids.length > 1 /* per-row toggle lives on the star column */ },
    { label: '⭐ ' + t('library.rating'), separatorAfter: ids.length === 1, disabled: true /* submenu placeholder */ },
    { label: t('context.editMetadata', '编辑元数据') + ' ✎' + (ids.length > 1 ? batchSuffix : ''), action: ids.length === 1 ? () => onEditMetadata?.(first) : undefined, disabled: ids.length > 1 /* batch edit lives in AlbumEditModal for now */ },
    { label: t('context.replaceCover', '替换封面'), disabled: true },
    { label: t('context.openFileLocation', '打开所在位置'), disabled: true /* needs opener plugin */ },
    { label: t('context.rescanMetadata', '重新扫描元数据'), disabled: true /* no per-track rescan command yet */ },
  ]
  return (
    <div
      className="ctx-menu"
      style={{
        position: 'fixed', left: cx, top: cy, zIndex: 1000,
        background: 'var(--bg, #1e1e1e)', color: 'var(--fg, #eee)',
        border: '1px solid var(--border, rgba(128,128,128,0.3))',
        borderRadius: 6, padding: 4, minWidth: 200, boxShadow: '0 8px 24px rgba(0,0,0,0.4)',
        fontSize: 'var(--text-sm)',
      }}
      onClick={(e) => e.stopPropagation()}
    >
      {items.map((it, i) => (
        <div key={i}>
          <button
            className="ctx-item"
            style={{
              display: 'block', width: '100%', textAlign: 'left',
              background: 'transparent', color: 'inherit', border: 'none',
              padding: '6px 12px', cursor: it.disabled ? 'default' : it.action ? 'pointer' : 'default',
              borderRadius: 4, opacity: it.disabled ? 0.38 : 1,
            }}
            onClick={() => { if (!it.disabled) it.action?.(); onClose() }}
            onMouseEnter={(e) => { if (it.action && !it.disabled) e.currentTarget.style.background = 'rgba(128,128,128,0.2)' }}
            onMouseLeave={(e) => { e.currentTarget.style.background = 'transparent' }}
          >
            {it.label}
          </button>
          {it.separatorAfter && <div style={{ height: 1, background: 'var(--border, rgba(128,128,128,0.2))', margin: '4px 0' }} />}
        </div>
      ))}
    </div>
  )
}
