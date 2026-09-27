import { useState, useEffect, useRef } from 'react'

export type VisMode = 'none' | 'quality' | 'depth3d' | 'depth4d' | 'depth5d'

interface Props {
  enabled: boolean          // 插件总开关（插件管理里的启用）
  active: boolean           // 本按钮的本地开关（on/off）
  mode: VisMode             // 当前选择的视觉模式
  onToggle: (on: boolean) => void
  onModeChange: (mode: VisMode) => void
  /** 进入「X 维」按钮的回调（跳转到独立 3D/4D/5D 页），留空则禁用进入按钮 */
  onEnterDimension?: (mode: Exclude<VisMode, 'none' | 'quality'>) => void
}

const MODE_OPTIONS: { k: VisMode; label: string; icon: string; disabled?: boolean; badge?: string }[] = [
  { k: 'none',    label: '恢复原生界面', icon: 'M18 6 6 18M6 6l12 12' },
  { k: 'quality', label: '质感提升',     icon: 'M12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2Z' },
  { k: 'depth3d', label: '三维承空',     icon: 'M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16Z M3.3 7 12 12l8.7-5 M12 22V12', badge: '正在开发' },
  { k: 'depth4d', label: '四维流时',     icon: 'M12 6v6l4 2 M12 2a10 10 0 1 0 10 10', disabled: true, badge: '敬请期待' },
  { k: 'depth5d', label: '五维衍世',     icon: 'M3 3h18v18H3z M3 12h18 M12 3v18', disabled: true, badge: '遥遥无期' },
]

/**
 * 视觉增强按钮（长椭圆形 pill）。
 *
 * 行为：
 *   点击按钮直接弹出设置面板（打开/关闭面板），不依赖 active 状态。
 *   面板内含：
 *     - 右上角 iOS 风格 toggle 开关（控制 on/off）
 *     - 两种模式：恢复原生界面 / 提升质感
 */
export default function VisModeButton({ enabled, active, mode, onToggle, onModeChange, onEnterDimension }: Props) {
  const [panelOpen, setPanelOpen] = useState(false)
  const [hovered, setHovered] = useState(false)
  const panelRef = useRef<HTMLDivElement>(null)
  const btnRef = useRef<HTMLButtonElement>(null)

  useEffect(() => {
    if (!panelOpen) return
    const onDoc = (e: MouseEvent) => {
      if (!panelRef.current || !btnRef.current) return
      const t = e.target as Node
      if (panelRef.current.contains(t) || btnRef.current.contains(t)) return
      setPanelOpen(false)
    }
    document.addEventListener('mousedown', onDoc)
    return () => document.removeEventListener('mousedown', onDoc)
  }, [panelOpen])

  if (!enabled) return null

  const handleClick = () => {
    setPanelOpen(v => !v)
  }

  const label = '视觉升华'

  return (
    <div className="vis-mode-wrap" style={{ position: 'relative', marginLeft: 'auto', display: 'flex', alignItems: 'center' }}>
      <button
        ref={btnRef}
        type="button"
        className={`vis-mode-btn${active ? ' active' : ''}${hovered ? ' hovered' : ''}`}
        onClick={handleClick}
        onMouseEnter={() => setHovered(true)}
        onMouseLeave={() => setHovered(false)}
        aria-label="视觉升华设置"
      >
        {/* 内部径向光泽 */}
        <span className="vis-mode-gloss" aria-hidden="true" />

        {/* 漂浮粒子（开态显示） */}
        <span className="vis-mode-particles" aria-hidden="true">
          {Array.from({ length: 12 }).map((_, i) => (
            <span key={i} style={{ '--i': i, '--n': 12 } as React.CSSProperties} />
          ))}
        </span>

        {/* 均衡器 */}
        <span className={`vis-mode-eq${active ? ' on' : ''}`} aria-hidden="true">
          <span /><span /><span /><span /><span />
        </span>

        <span className="vis-mode-label">{label}</span>

        {/* 开态显示设置齿轮 */}
        {active && (
          <svg
            className="vis-mode-gear"
            width="13"
            height="13"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
          >
            <path d="M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6Z" />
            <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1Z" />
          </svg>
        )}
      </button>

      {panelOpen && (
        <div ref={panelRef} className="vis-mode-panel" role="menu">
          <div className="vis-mode-panel-glow" aria-hidden="true" />
          {/* 顶部标题 + 开启/关闭开关 */}
          <div className="vis-mode-panel-title">
            <div className="vis-mode-panel-head">
              <div className="vis-mode-panel-head-text">
                <span className="vis-mode-panel-name">视觉升华</span>
              </div>
              <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                {/* 进入维度按钮：仅当视觉增强总开关开启 + 选中三维承空时才显示 */}
                {active && mode === 'depth3d' && !!onEnterDimension && (
                  <button
                    type="button"
                    title="进入三维承空"
                    onClick={() => onEnterDimension('depth3d')}
                    style={{
                      padding: '4px 10px',
                      borderRadius: 8,
                      border: '1px solid rgba(120,160,255,0.35)',
                      background: 'linear-gradient(135deg, rgba(99,102,241,0.22), rgba(6,182,212,0.16))',
                      color: 'rgba(220,235,255,0.95)',
                      fontSize: 'var(--text-xs)',
                      fontWeight: 600,
                      cursor: 'pointer',
                      letterSpacing: 0.5,
                    }}
                  >
                    进入维度
                  </button>
                )}
                <button
                  type="button"
                  className={`vis-mode-toggle${active ? ' on' : ''}`}
                  onClick={() => {
                    if (active) {
                      onToggle(false)
                    } else {
                      onToggle(true)
                    }
                  }}
                  aria-label={active ? '关闭视觉升华' : '开启视觉升华'}
                  title={active ? '关闭视觉升华' : '开启视觉升华'}
                >
                  <span className="vis-mode-toggle-knob" />
                </button>
              </div>
            </div>
          </div>
          <div className="vis-mode-panel-list">
            {MODE_OPTIONS.map((opt) => {
              const sel = mode === opt.k
              return (
                <div
                  key={opt.k}
                  className={`vis-mode-opt${sel ? ' sel' : ''}${opt.disabled ? ' vis-mode-opt-disabled' : ''}`}
                  style={{ position: 'relative' }}
                >
                  <button
                    type="button"
                    style={{
                      width: '100%', display: 'flex', alignItems: 'center', gap: 10,
                      padding: '10px 12px', background: 'transparent', border: 'none',
                      cursor: opt.disabled ? 'not-allowed' : 'pointer',
                      opacity: opt.disabled ? 0.55 : 1,
                      color: 'inherit', font: 'inherit', textAlign: 'left',
                    }}
                    onClick={() => !opt.disabled && onModeChange(opt.k)}
                    disabled={opt.disabled}
                  >
                    <span className="vis-mode-opt-radio" data-sel={sel} />
                    <span className="vis-mode-opt-body">
                      <span style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                        <span className="vis-mode-opt-label">{opt.label}</span>
                        {opt.badge && (
                          <span style={{
                            fontSize: 'var(--text-xs)', padding: '1px 6px', borderRadius: 999,
                            background: 'rgba(255,255,255,0.06)',
                            border: '1px solid rgba(120,160,255,0.22)',
                            color: opt.k === 'depth5d' ? 'rgba(255,180,180,0.8)' : 'rgba(160,200,255,0.8)',
                          }}>{opt.badge}</span>
                        )}
                      </span>
                    </span>
                    <svg
                      className="vis-mode-opt-icon"
                      width="16" height="16" viewBox="0 0 24 24" fill="none"
                      stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"
                    >
                      <path d={opt.icon} />
                    </svg>
                  </button>
                </div>
              )
            })}
          </div>
        </div>
      )}
    </div>
  )
}
