import { useCallback } from 'react'
import { toggleFavorite, type TrackInfo } from '../api/library'
import { useI18n } from '../i18n'

// ── FavoriteIcon ───────────────────────────────────────────────────

interface FavoriteIconProps {
  track: TrackInfo
  /** Bumped after a toggle so the parent can refresh queue/playlist. */
  onChanged?: (updated: TrackInfo) => void
  size?: number
  /** Compact list-row variant (no label, just the heart). */
  compact?: boolean
}

/**
 * Heart toggle for a library track.
 *
 * Calls `library_toggle_favorite` and applies the returned state
 * optimistically. When `onChanged` is supplied, the parent receives the
 * refreshed `TrackInfo` so it can update its own cache without a re-fetch.
 */
export function FavoriteIcon({ track, onChanged, size = 16, compact = false }: FavoriteIconProps) {
  const { t } = useI18n()
  const fav = track.is_favorite

  const handleClick = useCallback(async (e: React.MouseEvent) => {
    e.stopPropagation()
    e.preventDefault()
    try {
      const newState = await toggleFavorite(track.id)
      const updated = { ...track, is_favorite: newState }
      onChanged?.(updated)
      window.dispatchEvent(new CustomEvent('library-changed'))
    } catch (err) {
      console.error('[FavoriteIcon] toggle failed:', err)
    }
  }, [track, onChanged])

  return (
    <button
      type="button"
      className={`fav-btn${fav ? ' is-fav' : ''}`}
      onClick={handleClick}
      onDoubleClick={(e) => e.stopPropagation()}
      title={fav ? t('library.unfavorite') : t('library.favorite')}
      aria-label={fav ? t('library.unfavorite') : t('library.favorite')}
      aria-pressed={fav}
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        gap: compact ? 0 : 4,
        background: 'none',
        border: 'none',
        cursor: 'pointer',
        padding: compact ? 2 : 4,
        color: fav ? 'var(--accent)' : 'var(--text-dim)',
        fontSize: `${size}px`,
        lineHeight: 1,
        transition: 'color 0.15s, transform 0.1s',
      }}
      onMouseDown={(e) => e.stopPropagation()}
      onMouseUp={(e) => e.stopPropagation()}
    >
      <span style={{ display: 'inline-block', transform: fav ? 'scale(1.05)' : 'scale(1)' }}>
        {fav ? '\u2665' : '\u2661'}
      </span>
      {!compact && <span style={{ fontSize: 'var(--text-xs)' }}>{t('library.favorite')}</span>}
    </button>
  )
}
