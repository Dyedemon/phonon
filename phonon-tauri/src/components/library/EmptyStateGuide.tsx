/**
 * Empty-state guide shown when the library has no roots / no tracks yet.
 *
 * Spec §5.9 ASCII card design — two primary buttons:
 *   • 「⚙ 添加音乐目录…」→ opens a folder picker, sets roots, kicks scan
 *   • 「📁 先扫描一个文件夹试试」→ quick-pick a single folder, scan only
 *
 * A scan progress bar appears below the card while a scan is running, fed
 * by the `library-scan-progress` event stream.
 *
 * Style: reuses the existing `.card` / `.btn` / `.empty-state` classNames
 * already used across DspPanel/Settings so this view stays visually
 * consistent with the rest of the app.
 */
import { useEffect, useState } from 'react'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import { useI18n } from '../../i18n'
import * as libraryApi from '../../api/library'
import type { ScanProgress } from '../../api/library'

export interface EmptyStateGuideProps {
  /** Optional parent callback when the user has added roots and started a
   * scan — used by LibraryPage to switch out of the empty state AND to
   * trigger the §5.8.2 "create sync playlist?" modal.
   * @param folderPath  the first folder the user picked (basename used
   *                   as the playlist name when they say "yes").
   * @param inserted   total new + updated track count from the scan.
   */
  onScanned?: (folderPath: string, inserted: number) => void
  /** Toast sink for surfacing scan failures / progress summaries. */
  addToast?: (msg: string, kind?: 'info' | 'warn' | 'error') => void
}

export default function EmptyStateGuide({ onScanned, addToast }: EmptyStateGuideProps) {
  const { t } = useI18n()
  const [scanning, setScanning] = useState(false)
  const [progress, setProgress] = useState<ScanProgress | null>(null)

  // Subscribe to scan-progress events while a scan is running. We only
  // subscribe for the lifetime of this empty-state view — once the user
  // has a populated library, the parent LibraryPage takes over event
  // listening.
  useEffect(() => {
    if (!scanning) {
      setProgress(null)
      return
    }
    let unlisten: (() => void) | null = null
    let cancelled = false
    void libraryApi.onScanProgress((p) => {
      setProgress(p)
    }).then((un) => {
      if (cancelled) un()
      else unlisten = un
    })
    return () => {
      cancelled = true
      if (unlisten) unlisten()
    }
  }, [scanning])

  const pct = progress && progress.total > 0
    ? Math.min(100, Math.round((progress.processed / progress.total) * 100))
    : 0

  const handleAddRoots = async () => {
    const picked = await openDialog({ directory: true, multiple: true })
    if (!picked || (Array.isArray(picked) && picked.length === 0)) {
      addToast?.(t('library.addRootsCancel'), 'info')
      return
    }
    const roots = Array.isArray(picked) ? picked : [picked]
    const firstPath = Array.isArray(picked) ? picked[0] : picked
    try {
      await libraryApi.setRoots(roots)
      setScanning(true)
      const stats = await libraryApi.scan()
      setScanning(false)
      const n = stats.tracks_inserted + stats.tracks_updated
      addToast?.(t('library.empty.scanDone').replace('{n}', String(n)), 'info')
      onScanned?.(firstPath, n)
    } catch (e) {
      setScanning(false)
      addToast?.(t('library.empty.scanFailed').replace('{err}', String(e)), 'error')
    }
  }

  const handleQuickScan = async () => {
    const picked = await openDialog({ directory: true, multiple: false })
    if (!picked || typeof picked !== 'string') {
      addToast?.(t('library.addRootsCancel'), 'info')
      return
    }
    try {
      await libraryApi.setRoots([picked])
      setScanning(true)
      const stats = await libraryApi.scan()
      setScanning(false)
      const n = stats.tracks_inserted + stats.tracks_updated
      addToast?.(t('library.quickScanDone').replace('{n}', String(n)), 'info')
      onScanned?.(picked, n)
    } catch (e) {
      setScanning(false)
      addToast?.(t('library.quickScanFailed').replace('{err}', String(e)), 'error')
    }
  }

  return (
    <div className="empty-state" style={{ padding: '32px 24px', textAlign: 'center' }}>
      <div className="icon" style={{ fontSize: 48, opacity: 0.6 }}>{'\uD83C\uDFB5'}</div>
      <h3 style={{ margin: '12px 0 4px' }}>{t('library.empty.title')}</h3>
      <p style={{ margin: '0 0 20px', opacity: 0.7 }}>{t('library.empty.hint')}</p>
      <div style={{ display: 'flex', gap: 12, justifyContent: 'center', flexWrap: 'wrap' }}>
        <button
          className="btn btn-primary"
          onClick={handleAddRoots}
          disabled={scanning}
        >
          {t('library.empty.addRoots')}
        </button>
        <button
          className="btn btn-outline"
          onClick={handleQuickScan}
          disabled={scanning}
        >
          {t('library.empty.quickScan')}
        </button>
      </div>
      {scanning && (
        <div style={{ marginTop: 24, maxWidth: 480, margin: '24px auto 0' }}>
          <div style={{ fontSize: 'var(--text-sm)', opacity: 0.75, marginBottom: 6 }}>
            {progress
              ? t('library.empty.scanning')
                  .replace('{done}', String(progress.processed))
                  .replace('{total}', String(progress.total))
              : t('library.empty.scanning')
                  .replace('{done}', '0')
                  .replace('{total}', '?')}
          </div>
          <div style={{
            height: 6, width: '100%', background: 'rgba(128,128,128,0.25)',
            borderRadius: 3, overflow: 'hidden',
          }}>
            <div style={{
              height: '100%', width: `${pct}%`,
              background: 'var(--accent, #4a90e2)',
              transition: 'width 120ms linear',
            }} />
          </div>
        </div>
      )}
    </div>
  )
}
