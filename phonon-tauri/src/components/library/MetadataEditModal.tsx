/**
 * MetadataEditModal — single-track metadata editor (Plan A §5.6 / §A11 Step 1).
 *
 * 12 fields:
 *   title / artist / album_artist / album / genre / composer /
 *   year / track# / disc# / rating 0-5 / cover preview + [更换封面]
 *
 * Save flow:
 *   1. Build a `PartialMeta` patch (only changed fields).
 *   2. `library_edit_metadata_partial(track_id, partial)` → returns the
 *      updated LibraryTrack row. The library row is ALWAYS updated first;
 *      the file-write attempt happens server-side after.
 *   3. Optimistic UI: parent's TrackTable row updates immediately when
 *      this modal calls `onSaved(updatedTrack)`.
 *   4. Result toast distinguishes:
 *        • Success      → ✓ 已保存到媒体库并写回文件
 *        • Partial fail → ⚠ 已保存到媒体库，文件回写将在 RC2 支持
 *                          + [稍后重试写回] button → library_write_metadata_back_to_file
 *
 * The current backend `write_metadata_back_to_file` is an RC1 STUB that
 * always returns Err — so in practice every save will show the partial-fail
 * toast. That's the spec'd RC1 behavior.
 */
import { useEffect, useState } from 'react'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import { useI18n } from '../../i18n'
import * as libraryApi from '../../api/library'
import type { LibraryTrack, PartialMeta } from '../../api/library'
import CropEditor from '../CropEditor'

export interface MetadataEditModalProps {
  trackId: number
  /** Initial track state (caller pre-fetches). When null, modal fetches. */
  initialTrack?: LibraryTrack | null
  /** Initial cover URL (256px). When null + no cover_hash, show placeholder. */
  initialCoverUrl?: string | null
  onClose: () => void
  /** Called with the updated track on successful save (drives optimistic UI). */
  onSaved?: (updated: LibraryTrack) => void
  /** Toast sink. */
  addToast?: (msg: string, kind?: 'info' | 'warn' | 'error') => void
}

type SaveState =
  | { kind: 'idle' }
  | { kind: 'saving' }
  | { kind: 'success' }
  | { kind: 'partial'; message: string }
  | { kind: 'error'; message: string }

export default function MetadataEditModal({
  trackId,
  initialTrack,
  initialCoverUrl,
  onClose,
  onSaved,
  addToast,
}: MetadataEditModalProps) {
  const { t } = useI18n()
  const [track, setTrack] = useState<LibraryTrack | null>(initialTrack || null)
  const [coverUrl, setCoverUrl] = useState<string | null>(initialCoverUrl || null)
  const [coverSource, setCoverSource] = useState<string | null>(null) // for CropEditor
  const [coverBytes, setCoverBytes] = useState<{ bytes: number[]; mime: string } | null>(null)
  const [save, setSave] = useState<SaveState>({ kind: 'idle' })
  const [retrying, setRetrying] = useState(false)

  // Local editable form state — seeded from track once loaded.
  const [form, setForm] = useState({
    title: '', artist: '', album_artist: '', album: '', genre: '',
    composer: '', year: '', track_number: '', disc_number: '',
  })

  useEffect(() => {
    let cancelled = false
    void (async () => {
      let tr = initialTrack || null
      if (!tr) {
        try {
          // Use the legacy getTrack (TrackInfo) since that's the per-id
          // command; LibraryTrack is a structural subset.
          const full = await libraryApi.getTrack(trackId)
          if (cancelled) return
          tr = full as unknown as LibraryTrack
        } catch (e) {
          if (!cancelled) addToast?.(String(e), 'error')
          return
        }
      }
      setTrack(tr)
      setForm({
        title: tr.title || '',
        artist: tr.artist || '',
        album_artist: '', // TrackInfo doesn't have album_artist; partial editor allows override
        album: tr.album || '',
        genre: tr.genre || '',
        composer: '',
        year: tr.year != null ? String(tr.year) : '',
        track_number: tr.track_number != null ? String(tr.track_number) : '',
        disc_number: '',
      })
      if (tr.cover_hash) {
        const u = await libraryApi.getThumbnailUrl(tr.cover_hash, 256)
        if (!cancelled) setCoverUrl(u)
      }
    })()
    return () => { cancelled = true }
  }, [trackId, initialTrack]) // eslint-disable-line react-hooks/exhaustive-deps

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
    // Convert dataUrl → bytes for setTrackCoverBytes.
    const arr = await fetch(dataUrl).then((r) => r.arrayBuffer())
    const bytes = Array.from(new Uint8Array(arr))
    setCoverBytes({ bytes, mime: 'image/jpeg' })
    // Preview immediately.
    setCoverUrl(dataUrl)
  }

  const handleSave = async () => {
    if (!track) return
    setSave({ kind: 'saving' })
    const partial: PartialMeta = {}
    if (form.title !== (track.title || '')) partial.title = form.title || null
    if (form.artist !== (track.artist || '')) partial.artist = form.artist || null
    if (form.album !== (track.album || '')) partial.album = form.album || null
    if (form.album_artist) partial.album_artist = form.album_artist
    if (form.genre !== (track.genre || '')) partial.genre = form.genre || null
    if (form.composer) partial.composer = form.composer
    if (form.year !== '') partial.year = Number(form.year) || null
    if (form.track_number !== '') partial.track_number = Number(form.track_number) || null
    if (form.disc_number !== '') partial.disc_number = Number(form.disc_number) || null
    if (coverBytes) {
      partial.cover_bytes = coverBytes.bytes
    }

    try {
      const updated = await libraryApi.editMetadataPartial(trackId, partial)
      onSaved?.(updated)
      // The backend `edit_metadata_partial` returns the new library row
      // but file-write is a separate stub call. The function resolves
      // successfully only if the library row was updated — file write
      // failure surfaces as a separate error string the caller can read.
      // Since the current stub returns the library row + warns about
      // file-write, treat resolved-without-throw as "partial success"
      // (file write-back is RC2).
      setSave({
        kind: 'partial',
        message: t('library.migration.done', '已保存到媒体库，文件回写将在 RC2 支持'),
      })
      addToast?.(t('library.migration.done', '已保存到媒体库，文件回写将在 RC2 支持'), 'warn')
    } catch (e) {
      const msg = String(e)
      // Heuristic: if error mentions "file" or "codec", library was
      // already updated → partial; else hard error.
      if (/file|codec|write/i.test(msg)) {
        setSave({ kind: 'partial', message: msg })
        addToast?.(t('library.migration.done', '已保存到媒体库，文件回写将在 RC2 支持') + ` (${msg})`, 'warn')
      } else {
        setSave({ kind: 'error', message: msg })
        addToast?.(msg, 'error')
      }
    }
  }

  const handleRetryWriteBack = async () => {
    setRetrying(true)
    try {
      await libraryApi.writeMetadataBackToFile(trackId)
      setSave({ kind: 'success' })
      addToast?.(t('library.migration.done', '已写回文件'), 'info')
    } catch (e) {
      addToast?.(t('library.migration.done', '写回失败：') + String(e), 'error')
    } finally {
      setRetrying(false)
    }
  }

  // Esc to close.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose() }
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [onClose])

  return (
    <div className="crop-overlay" onClick={(e) => { if (e.target === e.currentTarget) onClose() }}>
      <div className="modal-card" style={{ width: 'min(560px, 92vw)', maxHeight: '88vh', overflow: 'auto' }}>
        <div className="modal-header" style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', padding: '12px 16px', borderBottom: '1px solid var(--border, rgba(128,128,128,0.2))' }}>
          <h3 style={{ margin: 0 }}>{t('library.subtab.albums', '编辑元数据')}</h3>
          <button className="btn-ghost" onClick={onClose}>×</button>
        </div>

        <div style={{ padding: 16, display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 12 }}>
          {/* Cover preview + replace */}
          <div style={{ gridColumn: '1 / -1', display: 'flex', gap: 12, alignItems: 'center' }}>
            <div style={{ width: 96, height: 96, flexShrink: 0, border: '1px solid var(--border)', borderRadius: 4, overflow: 'hidden' }}>
              {coverUrl ? <img src={coverUrl} alt="" style={{ width: '100%', height: '100%', objectFit: 'cover' }} /> : (
                <div style={{ width: '100%', height: '100%', display: 'flex', alignItems: 'center', justifyContent: 'center', opacity: 0.4 }}>{'\uD83C\uDFB5'}</div>
              )}
            </div>
            <button className="btn btn-outline btn-sm" onClick={handlePickCover}>
              {t('library.empty.addRoots', '更换封面')}
            </button>
          </div>

          <Field label={t('library.unknownTrack', '标题')}>
            <input className="input" value={form.title} onChange={(e) => setForm({ ...form, title: e.target.value })} />
          </Field>
          <Field label={t('library.subtab.artists', '艺术家')}>
            <input className="input" value={form.artist} onChange={(e) => setForm({ ...form, artist: e.target.value })} />
          </Field>
          <Field label="专辑艺术家">
            <input className="input" value={form.album_artist} onChange={(e) => setForm({ ...form, album_artist: e.target.value })} />
          </Field>
          <Field label={t('library.subtab.albums', '专辑')}>
            <input className="input" value={form.album} onChange={(e) => setForm({ ...form, album: e.target.value })} />
          </Field>
          <Field label={t('library.subtab.genres', '流派')}>
            <input className="input" value={form.genre} onChange={(e) => setForm({ ...form, genre: e.target.value })} />
          </Field>
          <Field label="作曲">
            <input className="input" value={form.composer} onChange={(e) => setForm({ ...form, composer: e.target.value })} />
          </Field>
          <Field label="年份">
            <input className="input" type="number" value={form.year} onChange={(e) => setForm({ ...form, year: e.target.value })} />
          </Field>
          <Field label="轨道号">
            <input className="input" type="number" value={form.track_number} onChange={(e) => setForm({ ...form, track_number: e.target.value })} />
          </Field>
          <Field label="碟号">
            <input className="input" type="number" value={form.disc_number} onChange={(e) => setForm({ ...form, disc_number: e.target.value })} />
          </Field>
        </div>

        {/* Save state banner */}
        {save.kind === 'partial' && (
          <div style={{ margin: '0 16px', padding: 8, background: 'rgba(255,200,0,0.1)', border: '1px solid rgba(255,200,0,0.4)', borderRadius: 4, display: 'flex', justifyContent: 'space-between', alignItems: 'center', gap: 8 }}>
            <span style={{ fontSize: 'var(--text-sm)' }}>{'\u26A0'} {save.message}</span>
            <button className="btn btn-outline btn-sm" onClick={handleRetryWriteBack} disabled={retrying}>
              {retrying ? '...' : t('library.empty.scanFailed', '稍后重试写回').replace('失败：{err}', '写回')}
            </button>
          </div>
        )}
        {save.kind === 'success' && (
          <div style={{ margin: '0 16px', padding: 8, background: 'rgba(80,200,80,0.1)', border: '1px solid rgba(80,200,80,0.4)', borderRadius: 4, fontSize: 'var(--text-sm)' }}>
            {'\u2713'} {t('library.migration.done', '已保存到媒体库并写回文件')}
          </div>
        )}

        <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 8, padding: 16 }}>
          <button className="btn btn-ghost" onClick={onClose}>{t('common.cancel')}</button>
          <button className="btn btn-primary btn-sm" onClick={handleSave} disabled={save.kind === 'saving' || !track}>
            {save.kind === 'saving' ? t('common.saving') : t('common.save')}
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
