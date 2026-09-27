import { useRef, useState, useEffect, useCallback, type MouseEvent, type CSSProperties } from 'react'
import { useI18n } from '../i18n'

// ============================================================================
//  CropEditor — Frame-based crop with drag-to-move and resize handles.
//
//  The image is fitted to a display area. A crop frame (initially centered,
//  square) can be:
//    • Dragged by clicking inside the frame → move
//    • Resized by dragging 8 handles (4 corners + 4 edges)
//
//  Output: the cropped region is drawn at native image resolution (up to 800px
//  max) for high quality.
// ============================================================================

const MIN_FRAME = 60 // Minimum crop frame size in display px
const DISPLAY_MAX = 500 // Max display area dimension

type DragMode = 'move' | 'n' | 's' | 'e' | 'w' | 'nw' | 'ne' | 'sw' | 'se' | null

interface DragState {
  mode: DragMode
  startMouseX: number
  startMouseY: number
  startFrame: { x: number; y: number; w: number; h: number }
}

export default function CropEditor({
  source,
  onConfirm,
  onCancel,
}: {
  source: string
  onConfirm: (dataUrl: string) => void
  onCancel: () => void
}) {
  const { t } = useI18n()
  const imgRef = useRef<HTMLImageElement>(null)
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const containerRef = useRef<HTMLDivElement>(null)

  const [natural, setNatural] = useState<{ w: number; h: number } | null>(null)
  const [displayW, setDisplayW] = useState(0)
  const [displayH, setDisplayH] = useState(0)
  // Frame position and size in display coordinates
  const [frame, setFrame] = useState({ x: 0, y: 0, w: 0, h: 0 })
  const [lockAspect, setLockAspect] = useState(true)
  const dragRef = useRef<DragState | null>(null)
  // Flag to suppress the next overlay click when mouseup happens on the overlay
  // during a drag — prevents the modal from closing when releasing outside.
  const suppressNextOverlayClickRef = useRef(false)

  // When image loads, compute display dimensions and initial frame.
  const onImgLoad = () => {
    const img = imgRef.current
    if (!img) return
    const w = img.naturalWidth
    const h = img.naturalHeight
    if (w === 0 || h === 0) return
    setNatural({ w, h })

    // Fit image into display area
    const maxDim = DISPLAY_MAX
    const ratio = Math.min(maxDim / w, maxDim / h, 1)
    const dw = Math.round(w * ratio)
    const dh = Math.round(h * ratio)
    setDisplayW(dw)
    setDisplayH(dh)

    // Initial frame: centered square, 80% of the shorter dimension
    const frameSize = Math.round(Math.min(dw, dh) * 0.8)
    setFrame({
      x: Math.round((dw - frameSize) / 2),
      y: Math.round((dh - frameSize) / 2),
      w: frameSize,
      h: frameSize,
    })
  }

  const clampFrame = useCallback(
    (f: { x: number; y: number; w: number; h: number }) => {
      const w = Math.max(MIN_FRAME, Math.min(f.w, displayW))
      const h = Math.max(MIN_FRAME, Math.min(f.h, displayH))
      const x = Math.max(0, Math.min(f.x, displayW - w))
      const y = Math.max(0, Math.min(f.y, displayH - h))
      return { x, y, w, h }
    },
    [displayW, displayH],
  )

  const startDrag = (e: MouseEvent, mode: DragMode) => {
    if (!mode) return
    e.preventDefault()
    e.stopPropagation()
    dragRef.current = {
      mode,
      startMouseX: e.clientX,
      startMouseY: e.clientY,
      startFrame: { ...frame },
    }
  }

  // Global mousemove/up so dragging continues even outside the container.
  useEffect(() => {
    const onMove = (e: globalThis.MouseEvent) => {
      const d = dragRef.current
      if (!d || !displayW || !displayH) return
      const dx = e.clientX - d.startMouseX
      const dy = e.clientY - d.startMouseY
      const sf = d.startFrame

      let nf = { ...sf }

      if (d.mode === 'move') {
        nf.x = sf.x + dx
        nf.y = sf.y + dy
      } else if (lockAspect) {
        // Square lock: compute new size from drag, keep opposite corner/edge fixed
        let newSize = 0
        switch (d.mode) {
          case 'nw':
            newSize = Math.max(sf.w - dx, sf.h - dy)
            nf.w = newSize; nf.h = newSize
            nf.x = sf.x + sf.w - newSize
            nf.y = sf.y + sf.h - newSize
            break
          case 'ne':
            newSize = Math.max(sf.w + dx, sf.h - dy)
            nf.w = newSize; nf.h = newSize
            nf.x = sf.x
            nf.y = sf.y + sf.h - newSize
            break
          case 'sw':
            newSize = Math.max(sf.w - dx, sf.h + dy)
            nf.w = newSize; nf.h = newSize
            nf.x = sf.x + sf.w - newSize
            nf.y = sf.y
            break
          case 'se':
            newSize = Math.max(sf.w + dx, sf.h + dy)
            nf.w = newSize; nf.h = newSize
            nf.x = sf.x
            nf.y = sf.y
            break
          case 'n':
            newSize = Math.max(MIN_FRAME, sf.h - dy)
            nf.w = newSize; nf.h = newSize
            nf.x = sf.x + sf.w / 2 - newSize / 2
            nf.y = sf.y + sf.h - newSize
            break
          case 's':
            newSize = Math.max(MIN_FRAME, sf.h + dy)
            nf.w = newSize; nf.h = newSize
            nf.x = sf.x + sf.w / 2 - newSize / 2
            nf.y = sf.y
            break
          case 'w':
            newSize = Math.max(MIN_FRAME, sf.w - dx)
            nf.w = newSize; nf.h = newSize
            nf.x = sf.x + sf.w - newSize
            nf.y = sf.y + sf.h / 2 - newSize / 2
            break
          case 'e':
            newSize = Math.max(MIN_FRAME, sf.w + dx)
            nf.w = newSize; nf.h = newSize
            nf.x = sf.x
            nf.y = sf.y + sf.h / 2 - newSize / 2
            break
        }
      } else {
        // Free resize — no aspect lock
        switch (d.mode) {
          case 'n':
            nf.y = sf.y + dy; nf.h = sf.h - dy
            break
          case 's':
            nf.h = sf.h + dy
            break
          case 'e':
            nf.w = sf.w + dx
            break
          case 'w':
            nf.x = sf.x + dx; nf.w = sf.w - dx
            break
          case 'nw':
            nf.x = sf.x + dx; nf.y = sf.y + dy; nf.w = sf.w - dx; nf.h = sf.h - dy
            break
          case 'ne':
            nf.y = sf.y + dy; nf.w = sf.w + dx; nf.h = sf.h - dy
            break
          case 'sw':
            nf.x = sf.x + dx; nf.w = sf.w - dx; nf.h = sf.h + dy
            break
          case 'se':
            nf.w = sf.w + dx; nf.h = sf.h + dy
            break
        }
        // Enforce minimum size
        if (nf.w < MIN_FRAME) {
          if (d.mode!.includes('w')) nf.x = sf.x + sf.w - MIN_FRAME
          nf.w = MIN_FRAME
        }
        if (nf.h < MIN_FRAME) {
          if (d.mode!.includes('n')) nf.y = sf.y + sf.h - MIN_FRAME
          nf.h = MIN_FRAME
        }
      }

      setFrame(clampFrame(nf))
    }

    const onUp = () => {
      if (dragRef.current) {
        // Drag just ended — if the user released on the overlay,
        // a click event will follow. Suppress it so the modal stays open.
        suppressNextOverlayClickRef.current = true
        // Reset the flag after the click has had time to fire
        window.setTimeout(() => {
          suppressNextOverlayClickRef.current = false
        }, 0)
      }
      dragRef.current = null
    }

    window.addEventListener('mousemove', onMove)
    window.addEventListener('mouseup', onUp)
    return () => {
      window.removeEventListener('mousemove', onMove)
      window.removeEventListener('mouseup', onUp)
    }
  }, [displayW, displayH, lockAspect, clampFrame])

  // Render crop at native resolution of the source image region.
  // Cap at 4096 to avoid memory issues with extremely large images.
  const handleConfirm = () => {
    const img = imgRef.current
    if (!img || !natural || !displayW || !displayH) return

    // Convert display coords to natural image coords
    const scaleX = natural.w / displayW
    const scaleY = natural.h / displayH
    const sx = Math.round(frame.x * scaleX)
    const sy = Math.round(frame.y * scaleY)
    const sw = Math.round(frame.w * scaleX)
    const sh = Math.round(frame.h * scaleY)

    // Output at native resolution of the cropped region, with a safety cap.
    const maxDim = 4096
    const outScale = Math.min(1, maxDim / Math.max(sw, sh))
    const outW = Math.max(1, Math.round(sw * outScale))
    const outH = Math.max(1, Math.round(sh * outScale))

    const canvas = canvasRef.current || document.createElement('canvas')
    canvas.width = outW
    canvas.height = outH
    const ctx = canvas.getContext('2d')
    if (!ctx) return
    ctx.imageSmoothingEnabled = outScale < 1
    ctx.imageSmoothingQuality = 'high'
    ctx.drawImage(img, sx, sy, sw, sh, 0, 0, outW, outH)
    const dataUrl = canvas.toDataURL('image/jpeg', 0.92)
    onConfirm(dataUrl)
  }

  const onOverlayClick = (e: MouseEvent) => {
    if (e.target === e.currentTarget) {
      if (suppressNextOverlayClickRef.current) {
        suppressNextOverlayClickRef.current = false
        return
      }
      onCancel()
    }
  }

  const hasImg = !!natural && displayW > 0

  // Shade regions (4 rectangles around the frame)
  const shades = hasImg
    ? [
        // Top
        { left: 0, top: 0, width: displayW, height: frame.y },
        // Bottom
        { left: 0, top: frame.y + frame.h, width: displayW, height: displayH - frame.y - frame.h },
        // Left
        { left: 0, top: frame.y, width: frame.x, height: frame.h },
        // Right
        { left: frame.x + frame.w, top: frame.y, width: displayW - frame.x - frame.w, height: frame.h },
      ]
    : []

  return (
    <div className="crop-overlay" onClick={onOverlayClick}>
      <div className="crop-modal">
        <div className="crop-header">
          <span>{t('settings.appearance.cropHint', '拖动移动 · 拖动边缘/角落调整大小')}</span>
          <div className="crop-actions">
            <button className="btn-ghost" onClick={onCancel}>{t('common.cancel')}</button>
            <button className="btn btn-primary btn-sm" onClick={handleConfirm} disabled={!hasImg}>
              {t('common.apply')}
            </button>
          </div>
        </div>

        <div
          ref={containerRef}
          className="crop-container"
          style={{
            width: displayW ? `${displayW}px` : 'auto',
            height: displayH ? `${displayH}px` : 'auto',
            position: 'relative',
            userSelect: 'none',
            overflow: 'hidden',
            display: 'inline-block',
            lineHeight: 0,
            background: 'rgba(0,0,0,0.65)',
          }}
        >
          <img
            ref={imgRef}
            src={source}
            alt="Crop"
            onLoad={onImgLoad}
            draggable={false}
            style={{
              display: 'block',
              width: displayW ? `${displayW}px` : 'auto',
              height: displayH ? `${displayH}px` : 'auto',
              maxWidth: '80vw',
              maxHeight: '70vh',
              objectFit: 'contain',
            }}
          />

          {hasImg && (
            <>
              {/* Shade overlay */}
              {shades.map((s, i) => (
                <div
                  key={i}
                  className="crop-shade"
                  style={{
                    position: 'absolute',
                    left: `${s.left}px`,
                    top: `${s.top}px`,
                    width: `${s.width}px`,
                    height: `${s.height}px`,
                    background: 'rgba(0,0,0,0.55)',
                    pointerEvents: 'none',
                    zIndex: 1,
                  }}
                />
              ))}

              {/* Crop frame */}
              <div
                className="crop-frame"
                style={{
                  position: 'absolute',
                  left: `${frame.x}px`,
                  top: `${frame.y}px`,
                  width: `${frame.w}px`,
                  height: `${frame.h}px`,
                  border: '2px solid #fff',
                  boxShadow: '0 0 0 1px rgba(0,0,0,0.5)',
                  cursor: 'move',
                  zIndex: 2,
                  boxSizing: 'border-box',
                }}
                onMouseDown={(e) => startDrag(e, 'move')}
              >
                {/* Rule-of-thirds grid */}
                <div style={{ position: 'absolute', left: '33.33%', top: 0, bottom: 0, width: 1, background: 'rgba(255,255,255,0.35)', pointerEvents: 'none' }} />
                <div style={{ position: 'absolute', left: '66.66%', top: 0, bottom: 0, width: 1, background: 'rgba(255,255,255,0.35)', pointerEvents: 'none' }} />
                <div style={{ position: 'absolute', top: '33.33%', left: 0, right: 0, height: 1, background: 'rgba(255,255,255,0.35)', pointerEvents: 'none' }} />
                <div style={{ position: 'absolute', top: '66.66%', left: 0, right: 0, height: 1, background: 'rgba(255,255,255,0.35)', pointerEvents: 'none' }} />

                {/* Resize handles */}
                {(['nw', 'n', 'ne', 'e', 'se', 's', 'sw', 'w'] as const).map((h) => {
                  const pos: Record<string, CSSProperties> = {
                    nw: { top: -6, left: -6, cursor: 'nw-resize' },
                    n: { top: -6, left: '50%', marginLeft: -6, cursor: 'n-resize' },
                    ne: { top: -6, right: -6, cursor: 'ne-resize' },
                    e: { top: '50%', right: -6, marginTop: -6, cursor: 'e-resize' },
                    se: { bottom: -6, right: -6, cursor: 'se-resize' },
                    s: { bottom: -6, left: '50%', marginLeft: -6, cursor: 's-resize' },
                    sw: { bottom: -6, left: -6, cursor: 'sw-resize' },
                    w: { top: '50%', left: -6, marginTop: -6, cursor: 'w-resize' },
                  }
                  return (
                    <div
                      key={h}
                      className={`crop-handle crop-handle-${h}`}
                      style={{
                        position: 'absolute',
                        width: 12,
                        height: 12,
                        background: '#fff',
                        border: '1px solid rgba(0,0,0,0.4)',
                        borderRadius: 2,
                        zIndex: 3,
                        ...pos[h],
                      }}
                      onMouseDown={(e) => startDrag(e, h)}
                    />
                  )
                })}
              </div>
            </>
          )}
        </div>

        {/* Aspect ratio lock toggle */}
        <div className="crop-scale" style={{ display: 'flex', alignItems: 'center', gap: 12, padding: '8px 16px' }}>
          <label style={{ fontSize: 'var(--text-sm)', opacity: 0.7, display: 'flex', alignItems: 'center', gap: 6, cursor: 'pointer' }}>
            <input
              type="checkbox"
              checked={lockAspect}
              onChange={(e) => setLockAspect(e.target.checked)}
              style={{ cursor: 'pointer' }}
            />
            {t('settings.appearance.lockSquare', '锁定正方形')}
          </label>
        </div>

        <canvas ref={canvasRef} style={{ display: 'none' }} />
      </div>
    </div>
  )
}
