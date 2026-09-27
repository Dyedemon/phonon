/**
 * SearchSuggestDropdown — 300ms-debounced omnibox with a 3-section
 * floating dropdown (5 tracks / 3 albums / 3 artists) backed by
 * `library_search_suggest`.
 *
 * UX:
 *   • Query empty or ≤1 char → dropdown hidden (min 2 chars to fire).
 *   • Keyboard ↑↓ navigates the flat suggestion list (track → album → artist order).
 *   • Enter jumps: track → onPlayTrack; album/artist → onOpenAlbum/onOpenArtist.
 *   • Click suggestion → same as Enter on it.
 *
 * Resilience (§5.10 #2):
 *   • >2000ms → spinner dim + 「搜索超时，请重试」button (clickable → re-fire).
 *   • >5000ms → AbortController cancels the in-flight invoke-via-fetch.
 *
 * NOTE on AbortController: Tauri `invoke` is NOT a fetch and is not
 * abortable via the standard AbortSignal API. We approximate the
 * >5000ms auto-cancel by tracking the request id and ignoring late
 * results. A real backend cancel would require an events-based channel;
 * we leave a code comment for the future migration.
 */
import { useCallback, useEffect, useRef, useState } from 'react'
import { useI18n } from '../../i18n'
import * as libraryApi from '../../api/library'
import type { LibrarySuggest, LibraryTrack, LibraryAlbum, LibraryArtist } from '../../api/library'

export interface SearchSuggestDropdownProps {
  /** Bounded input value (parent owns it). */
  value: string
  onChange: (v: string) => void
  /** Play a track from the dropdown. */
  onPlayTrack?: (trackId: number) => void
  /** Open an album detail page. */
  onOpenAlbum?: (albumId: number, title: string) => void
  /** Open an artist detail page. */
  onOpenArtist?: (artistId: number, name: string) => void
  /** Optional placeholder override. */
  placeholder?: string
}

type SuggestionItem =
  | { kind: 'track'; data: LibraryTrack }
  | { kind: 'album'; data: LibraryAlbum }
  | { kind: 'artist'; data: LibraryArtist }

const DEBOUNCE_MS = 300
const TIMEOUT_HINT_MS = 2000
const TIMEOUT_CANCEL_MS = 5000

export default function SearchSuggestDropdown({
  value, onChange, onPlayTrack, onOpenAlbum, onOpenArtist, placeholder,
}: SearchSuggestDropdownProps) {
  const { t } = useI18n()
  const [suggest, setSuggest] = useState<LibrarySuggest | null>(null)
  const [open, setOpen] = useState(false)
  const [loading, setLoading] = useState(false)
  const [slow, setSlow] = useState(false) // >2000ms
  const [cursor, setCursor] = useState(0)
  const reqIdRef = useRef(0)
  const slowTimerRef = useRef<number | null>(null)
  const cancelTimerRef = useRef<number | null>(null)

  const items: SuggestionItem[] = suggest
    ? [
      ...suggest.tracks.slice(0, 5).map((d) => ({ kind: 'track' as const, data: d })),
      ...suggest.albums.slice(0, 3).map((d) => ({ kind: 'album' as const, data: d })),
      ...suggest.artists.slice(0, 3).map((d) => ({ kind: 'artist' as const, data: d })),
    ]
    : []

  const fireSearch = useCallback(async (q: string) => {
    const id = ++reqIdRef.current
    setLoading(true)
    setSlow(false)
    if (slowTimerRef.current != null) window.clearTimeout(slowTimerRef.current)
    if (cancelTimerRef.current != null) window.clearTimeout(cancelTimerRef.current)
    slowTimerRef.current = window.setTimeout(() => {
      if (reqIdRef.current === id) setSlow(true)
    }, TIMEOUT_HINT_MS)
    cancelTimerRef.current = window.setTimeout(() => {
      if (reqIdRef.current === id) {
        // AbortController approximation: bump reqIdRef so late result is ignored.
        reqIdRef.current += 1
        setLoading(false)
        setSlow(false)
      }
    }, TIMEOUT_CANCEL_MS)
    try {
      const r = await libraryApi.searchSuggest(q)
      if (reqIdRef.current !== id) return // stale
      setSuggest(r)
      setOpen(true)
    } catch (e) {
      if (reqIdRef.current === id) {
        // keep old suggest visible
      }
    } finally {
      if (reqIdRef.current === id) {
        setLoading(false)
        setSlow(false)
        if (slowTimerRef.current != null) { window.clearTimeout(slowTimerRef.current); slowTimerRef.current = null }
        if (cancelTimerRef.current != null) { window.clearTimeout(cancelTimerRef.current); cancelTimerRef.current = null }
      }
    }
  }, [])

  // Debounce search on value change.
  useEffect(() => {
    const q = value.trim()
    if (q.length < 2) {
      setOpen(false)
      setSuggest(null)
      setSlow(false)
      return
    }
    const h = window.setTimeout(() => { void fireSearch(q) }, DEBOUNCE_MS)
    return () => window.clearTimeout(h)
  }, [value, fireSearch])

  // Cleanup timers on unmount.
  useEffect(() => {
    return () => {
      if (slowTimerRef.current != null) window.clearTimeout(slowTimerRef.current)
      if (cancelTimerRef.current != null) window.clearTimeout(cancelTimerRef.current)
    }
  }, [])

  // Keyboard nav.
  const onKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (!open || items.length === 0) return
    if (e.key === 'ArrowDown') {
      e.preventDefault()
      setCursor((c) => Math.min(items.length - 1, c + 1))
    } else if (e.key === 'ArrowUp') {
      e.preventDefault()
      setCursor((c) => Math.max(0, c - 1))
    } else if (e.key === 'Enter') {
      e.preventDefault()
      const it = items[cursor]
      if (!it) return
      handlePick(it)
    } else if (e.key === 'Escape') {
      setOpen(false)
    }
  }

  const handlePick = (it: SuggestionItem) => {
    if (it.kind === 'track') onPlayTrack?.(it.data.id)
    else if (it.kind === 'album') onOpenAlbum?.(it.data.id, it.data.title)
    else onOpenArtist?.(it.data.id, it.data.name)
    setOpen(false)
  }

  return (
    <div style={{ position: 'relative', flex: 1, minWidth: 220 }}>
      <input
        type="text"
        className="search-input"
        placeholder={placeholder || t('library.search.placeholder')}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={onKeyDown}
        onFocus={() => { if (suggest) setOpen(true) }}
        style={{
          width: '100%', padding: '6px 10px',
          opacity: loading && slow ? 0.6 : 1,
        }}
      />

      {loading && slow && (
        <div style={{ position: 'absolute', top: '100%', left: 0, right: 0, padding: 8, fontSize: 'var(--text-sm)', opacity: 0.7 }}>
          <span style={{ display: 'inline-block', marginRight: 6 }}>{'\u23F3'}</span>
          <button className="btn btn-ghost btn-sm" onClick={() => value.trim().length >= 2 && fireSearch(value.trim())}>
            {t('library.empty.scanFailed', '搜索超时，请重试').replace('{err}', '').replace('：', '')}
          </button>
        </div>
      )}

      {open && !loading && items.length > 0 && (
        <div
          className="search-dropdown"
          style={{
            position: 'absolute', top: '100%', left: 0, right: 0,
            background: 'var(--bg, #1e1e1e)', color: 'var(--fg, #eee)',
            border: '1px solid var(--border, rgba(128,128,128,0.3))',
            borderRadius: 6, zIndex: 100, maxHeight: 400, overflowY: 'auto',
            boxShadow: '0 8px 24px rgba(0,0,0,0.4)',
          }}
        >
          <Section title={t('library.stats.tracks', 'Tracks')} items={items.filter((i) => i.kind === 'track')} renderLabel={(i) => (i as any).data.title || t('library.unknownTrack')} renderSub={(i) => (i as any).data.artist || ''} cursor={cursor} offset={0} onPick={handlePick} />
          <Section title={t('library.stats.albums', 'Albums')} items={items.filter((i) => i.kind === 'album')} renderLabel={(i) => (i as any).data.title} renderSub={(i) => (i as any).data.album_artist || ''} cursor={cursor} offset={5} onPick={handlePick} />
          <Section title={t('library.stats.artists', 'Artists')} items={items.filter((i) => i.kind === 'artist')} renderLabel={(i) => (i as any).data.name} renderSub={() => ''} cursor={cursor} offset={8} onPick={handlePick} />
        </div>
      )}
    </div>
  )
}

interface SectionProps {
  title: string
  items: SuggestionItem[]
  renderLabel: (it: SuggestionItem) => string
  renderSub: (it: SuggestionItem) => string
  cursor: number
  offset: number
  onPick: (it: SuggestionItem) => void
}

function Section({ title, items, renderLabel, renderSub, cursor, offset, onPick }: SectionProps) {
  if (items.length === 0) return null
  return (
    <div>
      <div style={{ padding: '6px 12px', fontSize: 'var(--text-xs)', fontWeight: 700, opacity: 0.55, textTransform: 'uppercase' }}>
        {title}
      </div>
      {items.map((it, i) => {
        const idx = offset + i
        const active = cursor === idx
        return (
          <button
            key={`${title}-${i}`}
            onClick={() => onPick(it)}
            style={{
              display: 'block', width: '100%', textAlign: 'left',
              padding: '6px 12px', background: active ? 'rgba(128,128,128,0.18)' : 'transparent',
              border: 'none', color: 'inherit', cursor: 'pointer', fontSize: 'var(--text-sm)',
            }}
            onMouseEnter={(e) => { e.currentTarget.style.background = 'rgba(128,128,128,0.12)' }}
            onMouseLeave={(e) => { e.currentTarget.style.background = active ? 'rgba(128,128,128,0.18)' : 'transparent' }}
          >
            <div style={{ fontWeight: 500 }}>{renderLabel(it)}</div>
            {renderSub(it) && (
              <div style={{ fontSize: 'var(--text-xs)', opacity: 0.6 }}>{renderSub(it)}</div>
            )}
          </button>
        )
      })}
    </div>
  )
}
