/**
 * AlbumEditModal — bulk album fields editor (Plan A §A11 Step 2).
 *
 * Lets the user change album-level fields (album title, album_artist,
 * year, genre, album cover) and apply the same patch to ALL tracks in
 * the album via `library_edit_metadata_partial` called once per track.
 *
 * UX:
 *   • Initial state seeded from the album's first track.
 *   • Empty fields = "no change" for that field across all tracks.
 *   • Save: iterate tracks, call editMetadataPartial for each. Failures
 *     are collected per-track and surfaced as a single toast summary.
 *   • Album cover: picked via dialog → CropEditor → set cover_bytes on
 *     every track in the album (mime + bytes payload).
 */
import { useEffect, useState } from 'react'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import { useI18n } from '../../i18n'
import * as libraryApi from '../../api/library'
import type { TrackInfo, PartialMeta } from '../../api/library'
import CropEditor from '../CropEditor'

export interface AlbumEditModalProps {
  albumId: number
  albumTitle?: string
  onClose: () => void
  /** Pre-fetched album tracks (caller usually has them already). */
  initialTracks?: TrackInfo[]
  addToast?: (msg: string, kind?: 'info' | 'warn' | 'error') => void
  /** Called after bulk save with the list of updated track ids. */
  onSaved?: (trackIds: number[]) => void
}

export default function AlbumEditModal({
  albumId, albumTitle, onClose, initialTracks, addToast, onSaved,
}: AlbumEditModalProps) {
  const { t } = useI18n()
  const [tracks, setTracks] = useState<TrackInfo[]>(initialTracks || [])
  const [coverSource, setCoverSource] = useState<string | null>(null)
  const [coverBytes, setCoverBytes] = useState<{ bytes: number[]; mime: string } | null>(null)
  const [coverPreview, setCoverPreview] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)
  const [form, setForm] = useState({
    album: '', album_artist: '', year: '', genre: '',
  })

  useEffect(() => {
    let cancelled = false
    void (async () => {
      let ts = initialTracks || []
      if (ts.length === 0) {
        try {
          const r = await libraryApi.getTracks({ album_id: albumId })
          ts = r
        } catch (e) {
          if (!cancelled) addToast?.(String(e), 'error')
          return
        }
      }
      if (cancelled) return
      setTracks(ts)
      const first = ts[0]
      if (first) {
        setForm({
          album: first.album || '',
          album_artist: '',
          year: first.year != null ? String(first.year) : '',
          genre: first.genre || '',
        })
      }
    })()
    return () => { cancelled = true }
  }, [albumId, initialTracks]) // eslint-disable-line react-hooks/exhaustive-deps

  const handlePickCover = async () => {
    const picked = await openDialog({
      filters: [{ name: 'Images', extensions: ['jpg', 'jpeg', 'png', 'webp', 'bmp'] }],
      multiple: false,
    })
    if (!picked || typeof picked !== 'string') return
    setCoverSource(picked)
  }

  const handleCropConfirm = async (dataUrl: string) => {
    setCoverSource(null)
    const arr = await fetch(dataUrl).then((r) => r.arrayBuffer())
    setCoverBytes({ bytes: Array.from(new Uint8Array(arr)), mime: 'image/jpeg' })
    setCoverPreview(dataUrl)
  }

  const handleSave = async () => {
    if (tracks.length === 0) { onClose(); return }
    setSaving(true)
    const partial: PartialMeta = {}
    if (form.album) partial.album = form.album
    if (form.album_artist) partial.album_artist = form.album_artist
    if (form.year !== '') partial.year = Number(form.year) || null
    if (form.genre) partial.genre = form.genre
    if (coverBytes) partial.cover_bytes = coverBytes.bytes

    // No changes at all → just close.
    if (Object.keys(partial).length === 0) {
      setSaving(false)
      onClose()
      return
    }

    const ok: number[] = []
    const failed: string[] = []
    for (const tr of tracks) {
      try {
        await libraryApi.editMetadataPartial(tr.id, partial)
        ok.push(tr.id)
      } catch (e) {
        failed.push(`${tr.title || tr.file_path}: ${e}`)
      }
    }
    setSaving(false)
    if (failed.length === 0) {
      addToast?.(`${t('library.albumEdit.saved', '已保存')} (${ok.length}/${tracks.length})`, 'info')
    } else {
      addToast?.(`${t('library.albumEdit.partialFail', '部分失败')} ${failed.length}/${tracks.length}: ${failed[0]}`, 'warn')
    }
    onSaved?.(ok)
    onClose()
  }

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose() }
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [onClose])

  return (
    <div className="crop-overlay" onClick={(e) => { if (e.target === e.currentTarget) onClose() }}>
      <div className="modal-card" style={{ width: 'min(520px, 92vw)' }}>
        <div className="modal-header" style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', padding: '12px 16px', borderBottom: '1px solid var(--border, rgba(128,128,128,0.2))' }}>
          <h3 style={{ margin: 0 }}>{t('library.albumEdit.title')}{albumTitle ? ` · ${albumTitle}` : ''}</h3>
          <button className="btn-ghost" onClick={onClose}>×</button>
        </div>

        <div style={{ padding: 16, display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 12 }}>
          <div style={{ gridColumn: '1 / -1', display: 'flex', gap: 12, alignItems: 'center' }}>
            <div style={{ width: 96, height: 96, flexShrink: 0, border: '1px solid var(--border)', borderRadius: 4, overflow: 'hidden' }}>
              {coverPreview ? <img src={coverPreview} alt="" style={{ width: '100%', height: '100%', objectFit: 'cover' }} /> : (
                <div style={{ width: '100%', height: '100%', display: 'flex', alignItems: 'center', justifyContent: 'center', opacity: 0.4 }}>{'\uD83C\uDFB5'}</div>
              )}
            </div>
            <button className="btn btn-outline btn-sm" onClick={handlePickCover}>
              {t('library.albumEdit.changeCover')}
            </button>
          </div>
          <Field label={t('library.albumEdit.albumName')}>
            <input className="input" value={form.album} onChange={(e) => setForm({ ...form, album: e.target.value })} />
          </Field>
          <Field label={t('library.albumEdit.albumArtist')}>
            <input className="input" value={form.album_artist} onChange={(e) => setForm({ ...form, album_artist: e.target.value })} />
          </Field>
          <Field label={t('library.albumEdit.year')}>
            <input className="input" type="number" value={form.year} onChange={(e) => setForm({ ...form, year: e.target.value })} />
          </Field>
          <Field label={t('library.albumEdit.genre')}>
            <input className="input" value={form.genre} onChange={(e) => setForm({ ...form, genre: e.target.value })} />
          </Field>
          <div style={{ gridColumn: '1 / -1', fontSize: 'var(--text-sm)', opacity: 0.6 }}>
            {t('library.albumEdit.willEdit')}: {tracks.length} {t('library.stats.tracks')}
          </div>
        </div>

        <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 8, padding: 16 }}>
          <button className="btn btn-ghost" onClick={onClose}>{t('common.cancel')}</button>
          <button className="btn btn-primary btn-sm" onClick={handleSave} disabled={saving}>
            {saving ? t('common.saving') : t('common.save')}
          </button>
        </div>
      </div>

      {coverSource && (
        <CropEditor
          source={coverSource}
          onConfirm={handleCropConfirm}
          onCancel={() => setCoverSource(null)}
        />
      )}
    </div>
  )
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label style={{ display: 'flex', flexDirection: 'column', gap: 4, fontSize: 'var(--text-sm)' }}>
      <span style={{ opacity: 0.7 }}>{label}</span>
      {children}
    </label>
  )
}
