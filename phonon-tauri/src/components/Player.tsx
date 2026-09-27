import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { useI18n } from '../i18n'
import LyricsPanel from './LyricsPanel'
import { getTrackByPath, type TrackInfo } from '../api/library'
import { useProgress } from '../api/progressBus'
import { FavoriteIcon } from './TrackMeta'

interface PlaybackState {
  state: string
  position_secs: number
  duration_secs: number | null
  buffer_fill: number
  current_track: string | null
  queue_length: number
}

interface AlbumArtResponse {
  data: number[]
  mime_type: string
}

interface TrackMeta {
  title: string | null
  artist: string | null
  album: string | null
  duration: number | null
  sample_rate: number | null
  bit_depth: number | null
  channels: number | null
  bitrate: number | null
  format: string | null
}

interface Props {
  playback: PlaybackState
  onUpdate: () => void
}

function formatTime(secs: number) {
  const m = Math.floor(secs / 60)
  const s = Math.floor(secs % 60)
  return `${m}:${s.toString().padStart(2, '0')}`
}

export default function Player({ playback, onUpdate: _onUpdate }: Props) {
  const { t } = useI18n()
  // Position streams through the progress bus; subscribing here keeps the
  // ~20 Hz re-render local to Player + LyricsPanel instead of the App tree.
  const progress = useProgress()
  const position = progress.position
  const duration = progress.duration ?? playback.duration_secs
  const [coverUrl, setCoverUrl] = useState<string | null>(null)
  const [meta, setMeta] = useState<TrackMeta | null>(null)
  const [trackInfo, setTrackInfo] = useState<TrackInfo | null>(null)

  // Load album art + metadata when track changes
  useEffect(() => {
    let cancelled = false
    let url: string | null = null

    const load = async () => {
      setCoverUrl(null)
      setMeta(null)
      setTrackInfo(null)
      if (!playback.current_track) return

      // Load metadata
      try {
        const m = await invoke<TrackMeta>('get_track_metadata', {
          path: playback.current_track,
        })
        if (!cancelled) setMeta(m)
      } catch (e) {
        console.error('[Player] metadata failed:', e)
      }

      // Load library TrackInfo (favorite/rating). Tracks not in the
      // library (e.g. queued directly) just won't show the fav/rating UI.
      try {
        const info = await getTrackByPath(playback.current_track)
        if (!cancelled) setTrackInfo(info)
      } catch {
        /* not in library */
      }

      // Load album art
      try {
        const art = await invoke<AlbumArtResponse | null>('get_album_art', {
          path: playback.current_track,
        })
        if (cancelled || !art) return

        const blob = new Blob([new Uint8Array(art.data)], { type: art.mime_type })
        url = URL.createObjectURL(blob)
        setCoverUrl(url)
      } catch (_) { /* no art */ }
    }

    load()
    return () => {
      cancelled = true
      if (url) URL.revokeObjectURL(url)
    }
  }, [playback.current_track])

  const formatBitrate = (kbps: number) => {
    if (kbps >= 1000) return `${(kbps / 1000).toFixed(1)} Mbps`
    return `${kbps} kbps`
  }

  return (
    <div>
      {/* Unified Now Playing + Track Info */}
      <div className="card">
        <h3>{t('player.nowPlaying')}</h3>
        {playback.current_track ? (
          <>
          <div className="player-nowplaying">
            {/* Album Art — enlarged, centered at top */}
            <div className="player-cover-large" style={{ position: 'relative' }}>
              {coverUrl && (
                <img
                  src={coverUrl}
                  alt=""
                  style={{
                    position: 'absolute',
                    top: '-20%',
                    left: '-20%',
                    width: '140%',
                    height: '140%',
                    objectFit: 'cover',
                    filter: 'blur(30px) brightness(0.4)',
                    opacity: document.documentElement.style.getPropertyValue('--album-art-blur') === '1' ? 0.8 : 0,
                    transition: 'opacity 0.3s ease',
                    zIndex: 0,
                  }}
                />
              )}
              {coverUrl ? (
                <img src={coverUrl} alt={t('player.albumArt')} className="cover-img" style={{ position: 'relative', zIndex: 1 }} />
              ) : (
                <div className="cover-placeholder" style={{ position: 'relative', zIndex: 1 }}>
                  <svg viewBox="0 0 48 48" width="64" height="64" fill="none" stroke="currentColor" strokeWidth="1.5" opacity="0.3">
                    <circle cx="24" cy="24" r="20" />
                    <circle cx="24" cy="24" r="3" />
                    <line x1="24" y1="4" x2="24" y2="21" />
                    <line x1="24" y1="27" x2="24" y2="44" />
                    <line x1="4" y1="24" x2="21" y2="24" />
                    <line x1="27" y1="24" x2="44" y2="24" />
                  </svg>
                </div>
              )}
            </div>

            {/* Track info — centered below cover */}
            <div className="player-track-info">
              {/* Title */}
              <div className="track-title-main" title={meta?.title || extractFileName(playback.current_track)}>
                {meta?.title || extractFileName(playback.current_track)}
              </div>

              {/* Artist + Album */}
              {(meta?.artist || meta?.album) && (
                <div className="track-subtitle" title={`${meta?.artist || ''}${meta?.artist && meta?.album ? ' · ' : ''}${meta?.album || ''}`}>
                  {meta?.artist}
                  {meta?.artist && meta?.album && ' \u00B7 '}
                  {meta?.album}
                </div>
              )}

              {/* Playback time */}
              <div className="track-time-display">
                <span className="time-current">{formatTime(position)}</span>
                {duration ? (
                  <>
                    <span className="time-sep"> / </span>
                    <span className="time-duration">{formatTime(duration)}</span>
                  </>
                ) : null}
              </div>

              {/* Audio Quality Badges */}
              {meta && (
                <div className="track-quality">
                  {meta.format && <span className="quality-badge">{meta.format}</span>}
                  {meta.sample_rate && (
                    <span className="quality-badge">
                      {(meta.sample_rate / 1000).toFixed(1)} kHz
                    </span>
                  )}
                  {meta.bit_depth && (
                    <span className="quality-badge">{meta.bit_depth} bit</span>
                  )}
                  {meta.bitrate && (
                    <span className="quality-badge">{formatBitrate(meta.bitrate)}</span>
                  )}
                  {meta.channels && (
                    <span className="quality-badge">{meta.channels}ch</span>
                  )}
                </div>
              )}

              {/* Playback stats */}
              <div className="track-stats">
                <span>{playback.state}</span>
                <span className="stat-sep">|</span>
                <span>{t('player.buffer')} {(playback.buffer_fill * 100).toFixed(0)}%</span>
                <span className="stat-sep">|</span>
                <span>{t('player.queue')} {playback.queue_length}</span>
              </div>

              {/* Favorite + rating row — only when track is in the library. */}
              {trackInfo && (
                <div style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 16,
                  marginTop: 8,
                  flexWrap: 'wrap',
                }}>
                  <FavoriteIcon
                    track={trackInfo}
                    onChanged={(u) => setTrackInfo(u)}
                    size={18}
                  />
                  {trackInfo.play_count > 0 && (
                    <span style={{ fontSize: 'var(--text-xs)', color: 'var(--text-dim)' }}>
                      {t('library.playCount')}: {trackInfo.play_count}
                    </span>
                  )}
                </div>
              )}
            </div>
          </div>

        {/* Lyrics Panel */}
        <LyricsPanel trackPath={playback.current_track} positionSecs={position} />
      </>
      ) : (
          <div className="empty-state">
            <div className="icon">{'\u266B'}</div>
            <p>{t('player.noTrack')}</p>
            <p style={{ fontSize: 'var(--text-sm)' }}>{t('playlist.addFilesHint')}</p>
          </div>
        )}
      </div>
    </div>
  )
}

function extractFileName(path: string): string {
  const name = path.split(/[\\/]/).pop() || path
  return name.replace(/\.[^.]+$/, '')
}