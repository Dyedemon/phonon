/**
 * AlbumGrid — 160×160px CSS grid (auto-fill) of album cards.
 *
 * Each card:
 *   ┌────────────┐
 *   │  cover     │  (lazy load via IntersectionObserver + retry)
 *   │  (▶ hover) │
 *   ├────────────┤
 *   │ album name │
 *   │ artist/yr  │
 *   └────────────┘
 *
 * Click → push `detailStack` → AlbumDetailPage.
 *
 * Uses the legacy `AlbumInfo` row from `library_list_albums` (richer than
 * `LibraryAlbum` because it carries `total_duration_ms`).
 */
import { useEffect, useRef, useState } from 'react'
import { useI18n } from '../../i18n'
import * as libraryApi from '../../api/library'
import type { AlbumInfo } from '../../api/library'

export interface AlbumGridProps {
  albums: AlbumInfo[]
  onOpenAlbum?: (albumId: number, title: string) => void
}

function LazyAlbumCover({ hash, title }: { hash: string | null; title: string }) {
  const [url, setUrl] = useState<string | null>(null)
  const [failed, setFailed] = useState(false)
  const ref = useRef<HTMLDivElement>(null)
  const [inView, setInView] = useState(false)
  const attemptRef = useRef(0)
  const [tick, setTick] = useState(0)

  useEffect(() => {
    const el = ref.current
    if (!el || inView) return
    const io = new IntersectionObserver((entries) => {
      for (const e of entries) {
        if (e.isIntersecting) { setInView(true); io.disconnect(); break }
      }
    }, { rootMargin: '200px 0px' })
    io.observe(el)
    return () => io.disconnect()
  }, [inView])

  useEffect(() => {
    if (!inView || failed || !hash) return
    let cancelled = false
    void libraryApi.getThumbnailUrl(hash, 256).then((u) => {
      if (cancelled) return
      if (u) setUrl(u)
      else if (attemptRef.current < 3) {
        const delay = [300, 900, 2700][attemptRef.current]
        attemptRef.current += 1
        console.warn(`[Library] album cover retry ${attemptRef.current}/3 in ${delay}ms`)
        window.setTimeout(() => setTick((n) => n + 1), delay)
      } else setFailed(true)
    })
    return () => { cancelled = true }
  }, [inView, failed, hash, tick])

  return (
    <div ref={ref} className="album-cover-wrap" style={{ width: 160, height: 160 }}>
      {failed ? (
        <div className="album-cover placeholder"><span style={{ fontSize: 32, opacity: 0.4 }}>{'\uD83C\uDFB5'}</span></div>
      ) : url ? (
        <img className="album-cover" src={url} alt={title} loading="lazy" />
      ) : (
        <div className="album-cover placeholder" />
      )}
    </div>
  )
}

export default function AlbumGrid({ albums, onOpenAlbum }: AlbumGridProps) {
  const { t } = useI18n()
  if (albums.length === 0) {
    return (
      <div className="empty-state" style={{ padding: 24, textAlign: 'center' }}>
        <div className="icon">{'\uD83D\uDCBF'}</div>
        <p>{t('library.historyEmpty')}</p>
      </div>
    )
  }
  return (
    <div
      className="album-grid"
      style={{
        display: 'grid',
        gridTemplateColumns: 'repeat(auto-fill, minmax(160px, 1fr))',
        gap: 16,
        padding: 16,
      }}
    >
      {albums.map((al) => (
        <button
          key={al.id}
          className="album-card"
          onClick={() => onOpenAlbum?.(al.id, al.title)}
          style={{
            background: 'transparent', border: 'none', color: 'inherit',
            cursor: 'pointer', textAlign: 'left', padding: 0,
          }}
        >
          <div className="album-cover-hover" style={{ position: 'relative' }}>
            <LazyAlbumCover hash={al.cover_hash} title={al.title} />
            <div className="album-play-overlay" style={{
              position: 'absolute', inset: 0,
              display: 'flex', alignItems: 'center', justifyContent: 'center',
              background: 'rgba(0,0,0,0.35)', opacity: 0, transition: 'opacity 120ms',
              borderRadius: 6,
            }}>
              <span style={{ fontSize: 36, color: 'white' }}>{'\u25B6'}</span>
            </div>
          </div>
          <div className="album-name" style={{ fontWeight: 600, marginTop: 6, fontSize: 'var(--text-sm)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
            {al.title || t('library.unknownTrack')}
          </div>
          <div className="album-meta" style={{ opacity: 0.65, fontSize: 'var(--text-sm)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
            {[al.album_artist, al.year].filter(Boolean).join(' · ') || '—'}
          </div>
        </button>
      ))}
    </div>
  )
}
