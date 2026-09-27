/**
 * ArtistGenreList — A-Z sticky-header list of artists (or genres).
 *
 * Backend returns rows ordered by name. Front-end groups by first char:
 *   • ASCII letter (a-z, A-Z) → uppercase A-Z sticky header bucket
 *   • Non-ASCII (# prefix bucket for all non A-Z — Chinese, etc.)
 *
 * PARTIAL: the plan §5.4 expects pinyin-derived `sort_key` from the
 * backend so that 周杰伦 lands under "Z". However, the existing
 * `library_list_artists` command returns `ArtistInfo` which has NO
 * `sort_key` field (only the new `library_search_suggest` returns
 * `LibraryArtist` with sort_key, and only for the top 3). To avoid
 * shipping a half-baked pinyin table on the frontend, we group by the
 * raw first character of `name` — Chinese artists land under "#".
 * Once the backend exposes sort_key on the full artist list, this
 * component can switch to it without any other change.
 *
 * Click artist → push detail stack (reuse AlbumDetailPage structure
 * showing that artist's albums grid + tracks below).
 */
import { useEffect, useMemo, useState } from 'react'
import { useI18n } from '../../i18n'
import * as libraryApi from '../../api/library'
import type { ArtistInfo, GenreInfo, LibraryTrack } from '../../api/library'
import TrackTable from './TrackTable'

export interface ArtistGenreListProps {
  /** 'artists' or 'genres' — which list to render. */
  kind: 'artists' | 'genres'
  /** Open an artist detail page (push stack). */
  onOpenArtist?: (artistId: number, name: string) => void
  /** Open a genre detail page (push stack). */
  onOpenGenre?: (genreId: number, name: string) => void
  /** Currently-playing track id (passed through to TrackTable). */
  currentlyPlayingId?: number | null
  onPlayTrack?: (trackId: number) => void
  onAddToQueue?: (trackId: number) => void
  onAddToPlaylist?: (trackId: number) => void
  onEditMetadata?: (trackId: number) => void
  onReplaceCover?: (trackId: number) => void
  onRescanMetadata?: (trackId: number) => void
}

interface Bucket<T> {
  key: string
  items: T[]
}

function bucketKey(name: string): string {
  if (!name) return '#'
  const ch = name.charAt(0).toUpperCase()
  if (ch >= 'A' && ch <= 'Z') return ch
  return '#'
}

function bucketize<T extends { name: string; id: number }>(items: T[]): Bucket<T>[] {
  const m = new Map<string, T[]>()
  for (const it of items) {
    const k = bucketKey(it.name)
    if (!m.has(k)) m.set(k, [])
    m.get(k)!.push(it)
  }
  return [...m.entries()]
    .sort(([a], [b]) => (a === '#' ? 1 : b === '#' ? -1 : a.localeCompare(b)))
    .map(([key, items]) => ({ key, items }))
}

export default function ArtistGenreList({
  kind,
  onOpenArtist,
  onOpenGenre,
  currentlyPlayingId,
  onPlayTrack,
  onAddToQueue,
  onAddToPlaylist,
  onEditMetadata,
  onReplaceCover,
  onRescanMetadata,
}: ArtistGenreListProps) {
  const { t } = useI18n()
  const [artists, setArtists] = useState<ArtistInfo[]>([])
  const [genres, setGenres] = useState<GenreInfo[]>([])
  const [error, setError] = useState<string | null>(null)

  // Detail state for inline expansion (album grid + tracks).
  const [openId, setOpenId] = useState<number | null>(null)
  const [openTracks, setOpenTracks] = useState<LibraryTrack[]>([])

  useEffect(() => {
    let cancelled = false
    void (async () => {
      try {
        if (kind === 'artists') {
          const r = await libraryApi.listArtists()
          if (!cancelled) setArtists(r)
        } else {
          const r = await libraryApi.listGenres()
          if (!cancelled) setGenres(r)
        }
      } catch (e) {
        if (!cancelled) setError(String(e))
      }
    })()
    return () => { cancelled = true }
  }, [kind])

  const buckets = useMemo(() => {
    if (kind === 'artists') return bucketize(artists)
    return bucketize(genres)
  }, [kind, artists, genres])

  const openDetail = async (id: number) => {
    setOpenId(id)
    setOpenTracks([])
    try {
      const trows = await libraryApi.getTracks({ artist_id: id, limit: 1000 })
      setOpenTracks(trows as unknown as LibraryTrack[])
    } catch (e) {
      setError(String(e))
    }
  }

  if (error) {
    return (
      <div className="empty-state" style={{ padding: 24, textAlign: 'center' }}>
        <p style={{ color: 'var(--danger, #e66)' }}>{error}</p>
      </div>
    )
  }

  if (openId !== null) {
    const current = kind === 'artists'
      ? artists.find((a) => a.id === openId)
      : genres.find((g) => g.id === openId)
    return (
      <div style={{ display: 'flex', flexDirection: 'column', height: '100%', minHeight: 0 }}>
        <div style={{ padding: '4px 12px', display: 'flex', alignItems: 'center', gap: 8 }}>
          <button className="btn btn-ghost btn-sm" onClick={() => setOpenId(null)}>
            {t('library.back')}
          </button>
          <span style={{ fontWeight: 600 }}>{current?.name || '—'}</span>
        </div>
        <div style={{ flex: 1, minHeight: 0, overflow: 'auto' }}>
          <TrackTable
            tracks={openTracks}
            currentlyPlayingId={currentlyPlayingId}
            subTab="all"
            onPlayTrack={onPlayTrack}
            onAddToQueue={onAddToQueue}
            onAddToPlaylist={onAddToPlaylist}
            onEditMetadata={onEditMetadata}
            onReplaceCover={onReplaceCover}
            onRescanMetadata={onRescanMetadata}
          />
        </div>
      </div>
    )
  }

  if (buckets.length === 0) {
    return (
      <div className="empty-state" style={{ padding: 24, textAlign: 'center' }}>
        <div className="icon">{'\uD83C\uDFA4'}</div>
        <p>{t('library.historyEmpty')}</p>
      </div>
    )
  }

  return (
    <div style={{ height: '100%', overflowY: 'auto', padding: '8px 0' }}>
      {buckets.map((b) => (
        <div key={b.key} className="bucket">
          <div
            className="bucket-header"
            style={{
              position: 'sticky', top: 0, zIndex: 1,
              padding: '4px 16px',
              background: 'var(--bg, #1e1e1e)',
              fontWeight: 700, fontSize: 'var(--text-base)', opacity: 0.85,
            }}
          >
            {b.key}
          </div>
          {b.items.map((item) => (
            <button
              key={item.id}
              className="list-item"
              onClick={() => {
                if (kind === 'artists') {
                  if (onOpenArtist) { onOpenArtist(item.id, item.name); return; }
                } else {
                  if (onOpenGenre) { onOpenGenre(item.id, item.name); return; }
                }
                openDetail(item.id);
              }}
              style={{
                display: 'flex', justifyContent: 'space-between', alignItems: 'center',
                padding: '8px 16px', width: '100%',
                background: 'transparent', border: 'none', color: 'inherit',
                cursor: 'pointer', textAlign: 'left',
              }}
            >
              <span style={{ fontWeight: 500 }}>{item.name}</span>
              <span style={{ opacity: 0.6, fontSize: 'var(--text-sm)' }}>
                {item.track_count} {t('library.stats.tracks')}
              </span>
            </button>
          ))}
        </div>
      ))}
    </div>
  )
}
