/**
 * AlbumDetailPage — Apple-Music-style album header + track list.
 *
 * Layout:
 *   ┌──────────────────────────────────────────────────────┐
 *   │ ┌────────┐  Album Name (xxl bold)                    │
 *   │ │ cover  │  Artist → (click → push artist detail)     │
 *   │ │ 300px  │  Year · track_count × total_duration        │
 *   │ └────────┘  Genre                                    │
 *   │            [▶ 播放专辑] [+ 加入队列] [✎ 编辑专辑]   │
 *   ├──────────────────────────────────────────────────────┤
 *   │ TrackTable (album_id filter)                         │
 *   └──────────────────────────────────────────────────────┘
 *
 * Used for both the "album detail" and "artist detail" stack entries.
 * When pushed from artist context, the header reads "Artist Name"
 * instead of album title and shows the artist's album grid above the
 * track table (driven by props).
 */
import { useEffect, useState } from 'react'
import { useI18n } from '../../i18n'
import * as libraryApi from '../../api/library'
import type { LibraryTrack, AlbumInfo } from '../../api/library'
import TrackTable from './TrackTable'

export interface AlbumDetailPageProps {
  albumId: number
  /** Toast sink. */
  addToast?: (msg: string, kind?: 'info' | 'warn' | 'error') => void
  /** Currently-playing track id (passed through to TrackTable). */
  currentlyPlayingId?: number | null
  onPlayTrack?: (trackId: number) => void
  onAddToQueue?: (trackId: number) => void
  onAddToPlaylist?: (trackId: number) => void
  onEditMetadata?: (trackId: number) => void
  onReplaceCover?: (trackId: number) => void
  onRescanMetadata?: (trackId: number) => void
  /** Open album editor (AlbumEditModal). */
  onEditAlbum?: (albumId: number) => void
  /** Push artist detail onto the navigation stack. */
  onOpenArtist?: (artistId: number, name: string) => void
}

function fmtHMS(ms: number | null | undefined): string {
  if (ms == null || ms <= 0) return '--'
  const s = Math.round(ms / 1000)
  const h = Math.floor(s / 3600)
  const m = Math.floor((s % 3600) / 60)
  const ss = s % 60
  if (h > 0) return `${h}:${m.toString().padStart(2, '0')}:${ss.toString().padStart(2, '0')}`
  return `${m}:${ss.toString().padStart(2, '0')}`
}

export default function AlbumDetailPage({
  albumId,
  addToast,
  currentlyPlayingId,
  onPlayTrack,
  onAddToQueue,
  onAddToPlaylist,
  onEditMetadata,
  onReplaceCover,
  onRescanMetadata,
  onEditAlbum,
  onOpenArtist,
}: AlbumDetailPageProps) {
  const { t } = useI18n()
  const [album, setAlbum] = useState<AlbumInfo | null>(null)
  const [tracks, setTracks] = useState<LibraryTrack[]>([])
  const [coverUrl, setCoverUrl] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    void (async () => {
      try {
        const all = await libraryApi.listAlbums()
        const a = all.find((x) => x.id === albumId) || null
        if (cancelled) return
        setAlbum(a)
        if (a?.cover_hash) {
          const u = await libraryApi.getThumbnailUrl(a.cover_hash, 512)
          if (!cancelled) setCoverUrl(u)
        }
        const trows = await libraryApi.getTracks({ album_id: albumId })
        if (cancelled) return
        setTracks(trows as unknown as LibraryTrack[])
      } catch (e) {
        if (!cancelled) addToast?.(t('library.empty.scanFailed').replace('{err}', String(e)), 'error')
      }
    })()
    return () => { cancelled = true }
  }, [albumId]) // eslint-disable-line react-hooks/exhaustive-deps

  const playAlbum = () => {
    const first = tracks[0]
    if (first) onPlayTrack?.(first.id)
  }
  const addAlbumToQueue = () => {
    tracks.forEach((tr) => onAddToQueue?.(tr.id))
  }

  return (
    <div className="album-detail" style={{ display: 'flex', flexDirection: 'column', height: '100%', minHeight: 0 }}>
      {/* Header: 300px cover + right column */}
      <div className="album-detail-header" style={{
        display: 'flex', gap: 24, padding: 24,
        borderBottom: '1px solid var(--border, rgba(128,128,128,0.2))',
      }}>
        <div className="album-detail-cover" style={{ width: 300, height: 300, flexShrink: 0 }}>
          {coverUrl ? (
            <img src={coverUrl} alt={album?.title || ''} style={{ width: '100%', height: '100%', objectFit: 'cover', borderRadius: 6 }} />
          ) : (
            <div style={{ width: '100%', height: '100%', display: 'flex', alignItems: 'center', justifyContent: 'center', background: 'rgba(128,128,128,0.15)', borderRadius: 6 }}>
              <span style={{ fontSize: 64, opacity: 0.4 }}>{'\uD83C\uDFB5'}</span>
            </div>
          )}
        </div>
        <div className="album-detail-meta" style={{ display: 'flex', flexDirection: 'column', gap: 8, flex: 1, minWidth: 0 }}>
          <h1 style={{ fontSize: 28, fontWeight: 700, margin: 0 }}>
            {album?.title || t('library.unknownTrack')}
          </h1>
          {album?.album_artist && (
            <button
              className="btn btn-ghost btn-sm"
              onClick={() => onOpenArtist?.(0, album.album_artist!)}
              style={{ alignSelf: 'flex-start', padding: 0 }}
            >
              {album.album_artist}
            </button>
          )}
          <div style={{ opacity: 0.7, fontSize: 'var(--text-base)' }}>
            {album?.year ? String(album.year) : '—'}
            {' · '}
            {tracks.length} {t('library.stats.tracks')}
            {album?.total_duration_ms ? ` · ${fmtHMS(album.total_duration_ms)}` : ''}
          </div>
          <div style={{ display: 'flex', gap: 8, marginTop: 8 }}>
            <button className="btn btn-primary btn-sm" onClick={playAlbum} disabled={tracks.length === 0}>
              {'\u25B6'} {t('library.history', 'Play album')}
            </button>
            <button className="btn btn-outline btn-sm" onClick={addAlbumToQueue} disabled={tracks.length === 0}>
              {t('library.menu', 'Add to queue')}
            </button>
            <button className="btn btn-outline btn-sm" onClick={() => onEditAlbum?.(albumId)}>
              {'\u270E'} {t('library.subtab.albums', 'Edit album')}
            </button>
          </div>
        </div>
      </div>

      <div style={{ flex: 1, minHeight: 0 }}>
        <TrackTable
          tracks={tracks}
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
