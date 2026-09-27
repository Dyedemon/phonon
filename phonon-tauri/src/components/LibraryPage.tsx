/**
 * LibraryPage — top-level media library view.
 *
 * Sub-tabs: All / Favorites / Recent / Albums / Artists / Genres
 * Detail navigation: album/artist/genre detail pages via push/pop stack.
 */
import { Component, useCallback, useEffect, useMemo, useState, type ReactNode } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { useI18n } from '../i18n'
import EmptyStateGuide from './library/EmptyStateGuide'
import TrackTable from './library/TrackTable'
import AlbumGrid from './library/AlbumGrid'
import ArtistGenreList from './library/ArtistGenreList'
import AlbumDetailPage from './library/AlbumDetailPage'
import type { LibraryTrack, AlbumInfo, ArtistInfo } from '../api/library'
import { getRoots, getTracks, listAlbums, listArtists } from '../api/library'
import { addToQueue, clearQueue } from '../api/queue'
import { play } from '../api/player'

export type LibrarySubTab =
  | 'all'
  | 'favorites'
  | 'recent'
  | 'albums'
  | 'artists'
  | 'genres'

export type DetailStackEntry =
  | { kind: 'album'; id: number; title: string }
  | { kind: 'artist'; id: number; title: string }
  | { kind: 'genre'; id: number; title: string }

export interface LibraryPageProps {
  addToast?: (msg: string, kind?: 'info' | 'warn' | 'error') => void
  onNavigate?: (tab: string) => void
  /** Currently playing track file path — used to highlight the row. */
  currentTrackPath?: string | null
  /** Called after playback changes so App.tsx can refresh. */
  onPlaybackChange?: () => void
}

// ---------------------------------------------------------------------------
// Error boundary
// ---------------------------------------------------------------------------

class LibraryErrorBoundary extends Component<
  { children: ReactNode; retryKey: number; t: (k: string, f?: string) => string },
  { hasError: boolean }
> {
  state = { hasError: false }
  static getDerivedStateFromError() {
    return { hasError: true }
  }
  componentDidUpdate(prev: { retryKey: number }) {
    if (prev.retryKey !== this.props.retryKey && this.state.hasError) {
      this.setState({ hasError: false })
    }
  }
  render() {
    if (this.state.hasError) {
      return (
        <div className="empty-state" style={{ padding: 24, textAlign: 'center' }}>
          <div className="icon">{'\u26A0\uFE0F'}</div>
          <p>{this.props.t('library.errorBoundary')}</p>
        </div>
      )
    }
    return this.props.children
  }
}

// ---------------------------------------------------------------------------
// Page
// ---------------------------------------------------------------------------

export default function LibraryPage({ addToast, onNavigate, currentTrackPath, onPlaybackChange }: LibraryPageProps) {
  const { t } = useI18n()
  const [subTab, setSubTab] = useState<LibrarySubTab>('all')
  const [detailStack, setDetailStack] = useState<DetailStackEntry[]>([])
  const [retryKey, setRetryKey] = useState(0)
  const [searchQuery, setSearchQuery] = useState('')

  const [tracks, setTracks] = useState<LibraryTrack[]>([])
  const [albums, setAlbums] = useState<AlbumInfo[]>([])
  const [artists, setArtists] = useState<ArtistInfo[]>([])
  const [hasRoots, setHasRoots] = useState<boolean | null>(null)

  // Fetch tracks + albums + artists on mount and when retryKey changes.
  useEffect(() => {
    let cancelled = false
    void (async () => {
      try {
        const roots = await getRoots()
        if (cancelled) return
        setHasRoots(roots.length > 0)
        if (roots.length === 0) return
        const [trows, arows, artrows] = await Promise.all([
          getTracks({ favorites_only: false, limit: 10000 }),
          listAlbums(),
          listArtists(),
        ])
        if (cancelled) return
        setTracks(trows as unknown as LibraryTrack[])
        setAlbums(arows)
        setArtists(artrows)
      } catch (e) {
        if (!cancelled) addToast?.(t('library.empty.scanFailed').replace('{err}', String(e)), 'error')
      }
    })()
    return () => { cancelled = true }
  }, [retryKey]) // eslint-disable-line react-hooks/exhaustive-deps

  // Listen for library-changed events (favorite/rating toggled elsewhere).
  useEffect(() => {
    const handler = () => setRetryKey((k) => k + 1)
    window.addEventListener('library-changed', handler)
    return () => window.removeEventListener('library-changed', handler)
  }, [])

  const popDetail = () => setDetailStack((s) => s.slice(0, -1))

  const onScanned = (_inserted: number) => {
    setHasRoots(true)
    setRetryKey((k) => k + 1)
  }

  // ── Playback callbacks ──────────────────────────────────────
  const handlePlayTrack = useCallback(async (trackId: number) => {
    const track = tracks.find((tr) => tr.id === trackId)
    if (!track?.file_path) return
    try {
      await clearQueue()
      await addToQueue([track.file_path])
      await play()
      onPlaybackChange?.()
    } catch (e) {
      addToast?.(String(e), 'error')
    }
  }, [tracks, addToast, onPlaybackChange])

  const handleAddToQueue = useCallback(async (trackId: number) => {
    const track = tracks.find((tr) => tr.id === trackId)
    if (!track?.file_path) return
    try {
      await addToQueue([track.file_path])
      addToast?.(t('toast.addedToQueue', '已添加到播放队列'), 'info')
    } catch (e) {
      addToast?.(String(e), 'error')
    }
  }, [tracks, addToast, t])

  // Match current track path to a track id for highlight.
  const currentlyPlayingId = useMemo(() => {
    if (!currentTrackPath) return null
    return tracks.find((tr) => tr.file_path === currentTrackPath)?.id ?? null
  }, [tracks, currentTrackPath])

  // Filtered/sorted track lists per sub-tab.
  const displayTracks = useMemo(() => {
    let list = tracks
    if (subTab === 'favorites') {
      list = tracks.filter((tr) => tr.is_favorite)
    } else if (subTab === 'recent') {
      list = tracks
        .filter((tr) => tr.last_played_at != null)
        .sort((a, b) => (b.last_played_at! - a.last_played_at!))
    }
    if (searchQuery.trim()) {
      const q = searchQuery.toLowerCase()
      list = list.filter((tr) =>
        (tr.title?.toLowerCase().includes(q) ||
         tr.artist?.toLowerCase().includes(q) ||
         tr.album?.toLowerCase().includes(q))
      )
    }
    return list
  }, [tracks, subTab, searchQuery])

  // ── §5.8.2 add-roots modal ─────────────────────────────────
  const [askPlaylist, setAskPlaylist] = useState<{ folderPath: string } | null>(null)
  const onAskCreateSyncPlaylist = (folderPath: string) => setAskPlaylist({ folderPath })

  const confirmCreateSyncPlaylist = async () => {
    if (!askPlaylist) return
    const folderPath = askPlaylist.folderPath
    const basename = folderPath.split(/[\\/]/).pop() || folderPath
    try {
      await invoke('create_playlist', { name: basename })
      await invoke('sync_folder', { playlist: basename, path: folderPath }).catch(() => {})
      addToast?.(t('library.askCreateSyncPlaylist.yes') + ' — ' + basename, 'info')
      onNavigate?.('player')
    } catch (e) {
      addToast?.(String(e), 'error')
    } finally {
      setAskPlaylist(null)
    }
  }

  const isEmpty = hasRoots === false
  const top = detailStack[detailStack.length - 1]

  const subTabs: { id: LibrarySubTab; label: string }[] = [
    { id: 'all', label: t('library.subtab.all') },
    { id: 'favorites', label: t('library.subtab.favorites') },
    { id: 'recent', label: t('library.subtab.recent') },
    { id: 'albums', label: t('library.subtab.albums') },
    { id: 'artists', label: t('library.subtab.artists') },
    { id: 'genres', label: t('library.subtab.genres') },
  ]

  // Shared callback props for TrackTable
  const trackTableCallbacks = {
    currentlyPlayingId,
    onPlayTrack: handlePlayTrack,
    onAddToQueue: handleAddToQueue,
  }

  return (
    <div className="library-page" style={{ display: 'flex', flexDirection: 'column', height: '100%', minHeight: 0 }}>
      {/* Top toolbar */}
      <div className="toolbar" style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '8px 12px', flexWrap: 'wrap' }}>
        <input
          type="text"
          className="search-input"
          placeholder={t('library.search.placeholder')}
          value={searchQuery}
          onChange={(e) => setSearchQuery(e.target.value)}
          style={{ flex: 1, minWidth: 220, padding: '6px 10px' }}
        />
        <button
          className="btn btn-ghost btn-sm"
          onClick={() => onNavigate?.('settings')}
          title={t('library.menu')}
        >
          {'\u22EE'}
        </button>
        <div style={{ opacity: 0.7, fontSize: 'var(--text-sm)', marginLeft: 'auto' }}>
          {tracks.length} {t('library.stats.tracks')}
          {' / '}
          {albums.length} {t('library.stats.albums')}
          {' / '}
          {artists.length} {t('library.stats.artists')}
        </div>
      </div>

      {/* Sub-tab strip OR detail back-button */}
      {top ? (
        <div style={{ padding: '4px 12px', display: 'flex', alignItems: 'center', gap: 8 }}>
          <button className="btn btn-ghost btn-sm" onClick={popDetail}>
            {t('library.back')}
          </button>
          <span style={{ fontWeight: 600 }}>{top.title}</span>
        </div>
      ) : (
        <div className="library-subtabs" style={{ display: 'flex', gap: 4, padding: '4px 12px', borderBottom: '1px solid var(--border, rgba(128,128,128,0.2))' }}>
          {subTabs.map((st) => (
            <button
              key={st.id}
              className={`btn btn-sm ${subTab === st.id ? 'btn-primary' : 'btn-outline'}`}
              onClick={() => setSubTab(st.id)}
            >
              {st.label}
            </button>
          ))}
        </div>
      )}

      {/* Body */}
      <div style={{ flex: 1, minHeight: 0, overflow: 'auto' }}>
        <LibraryErrorBoundary retryKey={retryKey} t={t}>
          {isEmpty ? (
            <EmptyStateGuide onScanned={(folderPath, n) => { onScanned(n); onAskCreateSyncPlaylist(folderPath) }} addToast={addToast} />
          ) : top ? (
            <AlbumDetailPage
              albumId={top.id}
              addToast={addToast}
              {...trackTableCallbacks}
              onOpenArtist={(artistId, name) => setDetailStack((s) => [...s, { kind: 'artist', id: artistId, title: name }])}
            />
          ) : subTab === 'all' || subTab === 'favorites' || subTab === 'recent' ? (
            <TrackTable
              tracks={displayTracks}
              subTab={subTab}
              {...trackTableCallbacks}
            />
          ) : subTab === 'albums' ? (
            <AlbumGrid
              albums={albums}
              onOpenAlbum={(albumId, title) => setDetailStack((s) => [...s, { kind: 'album', id: albumId, title }])}
            />
          ) : (
            <ArtistGenreList
              kind={subTab === 'artists' ? 'artists' : 'genres'}
              onOpenArtist={(artistId, name) => setDetailStack((s) => [...s, { kind: 'artist', id: artistId, title: name }])}
              onOpenGenre={(genreId, name) => setDetailStack((s) => [...s, { kind: 'genre', id: genreId, title: name }])}
              {...trackTableCallbacks}
            />
          )}
        </LibraryErrorBoundary>
      </div>

      {/* §5.8.2 add-roots modal — "要不要同时创建同步歌单？" */}
      {askPlaylist && (
        <div
          className="crop-overlay"
          onClick={(e) => { if (e.target === e.currentTarget) setAskPlaylist(null) }}
          style={{
            position: 'fixed', inset: 0, zIndex: 9999,
            background: 'rgba(0,0,0,0.65)', display: 'flex',
            alignItems: 'center', justifyContent: 'center',
          }}
        >
          <div className="modal-card" style={{ width: 'min(440px, 92vw)' }}>
            <h3 style={{ margin: '0 0 8px' }}>{t('library.askCreateSyncPlaylist.title')}</h3>
            <p style={{ fontSize: 'var(--text-sm)', opacity: 0.7, margin: '0 0 16px' }}>
              {t('library.askCreateSyncPlaylist.hint')}
            </p>
            <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 8 }}>
              <button className="btn btn-ghost" onClick={() => setAskPlaylist(null)}>
                {t('library.askCreateSyncPlaylist.no')}
              </button>
              <button className="btn btn-primary btn-sm" onClick={confirmCreateSyncPlaylist}>
                {t('library.askCreateSyncPlaylist.yes')}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
