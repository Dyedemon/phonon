import { useState, useEffect, useRef, useCallback } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { save, open } from '@tauri-apps/plugin-dialog'
import { writeFile } from '@tauri-apps/plugin-fs'
import { useI18n } from '../i18n'
import CropEditor from './CropEditor'
import type { Lang } from '../i18n'
import { readStorage, writeStorage } from '../api/storage'
import {
  getHotplugStatus,
  setOnDeviceRemoved,
  setOnNewDeviceInserted,
  debugInjectHotplug,
  getExclusiveMode,
  getPlatform,
  setExclusiveMode,
  type HotplugStatus,
} from '../api/devices'
import type {
  DeviceInsertedStrategy,
  DeviceRemovedStrategy,
} from '../api/settings'
import { setSpectrumSettings, setResamplerQuality, resetToDefaults } from '../api/settings'

// ── Types ──────────────────────────────────────────────────────

type RgMode = 'Off' | 'Track' | 'Album'
type VolMode = 'Hardware' | 'Software'
type Quality = 'Fast' | 'Balanced' | 'High'
type StartupBehavior = 'Remember' | 'Clear'
type ColorScheme = 'aurora' | 'warm' | 'cool' | 'mono' | 'custom'
type ThemePreset = 'dark' | 'light' | 'aurora' | 'warm' | 'pro-blue' | 'high-contrast' | ''

interface ShortcutBinding {
  keys: string
  action: string
}

interface SettingsProps {
  volumeMode: string
  onVolumeModeChange: (mode: string) => void
  addToast: (message: string, kind?: 'info' | 'warn' | 'error') => void
  onSpectrumColorSchemeChange: (scheme: string, customColors?: { bottom: string; top: string; peak: string; grid: string }) => void
  onSpectrumSettingsChange: (settings: { smoothing: number; decay: number; fps: number }) => void
  /** Navigate to other top-level tabs — used by the "管理媒体库 →"
   * shortcut to switch to LibraryPage. Optional: if absent, the button
   * is hidden. */
  onNavigate?: (tab: string) => void
}

// ── Theme Preset Definitions ───────────────────────────────────

interface PresetColors {
  accent: string
  accent2: string
  bg: string
  cardBg: string
  border: string
  text: string
  textDim: string
  textBright: string
  placeholder: string
  accentOpacity: number
  accent2Opacity?: number
  bgOpacity: number
  cardBgOpacity: number
  borderOpacity: number
  textDimOpacity: number
  placeholderOpacity: number
}

const THEME_PRESETS: Record<string, PresetColors> = {
  dark: {
    accent: '#06b6d4', accent2: '#6366f1', bg: '#0a0a14', cardBg: '#111122', border: '#1e1e32',
    text: '#c8c8d4', textDim: '#565d79', textBright: '#e8e8f0', placeholder: '#565d79',
    accentOpacity: 1, bgOpacity: 1, cardBgOpacity: 1, borderOpacity: 1, textDimOpacity: 1, placeholderOpacity: 0.5,
  },
  light: {
    accent: '#0891b2', accent2: '#4f46e5', bg: '#f5f5f7', cardBg: '#ffffff', border: '#e0e0e6',
    text: '#3a3a44', textDim: '#8a8a96', textBright: '#1a1a24', placeholder: '#8a8a96',
    accentOpacity: 1, bgOpacity: 1, cardBgOpacity: 1, borderOpacity: 1, textDimOpacity: 1, placeholderOpacity: 0.5,
  },
  aurora: {
    accent: '#10b981', accent2: '#8b5cf6', bg: '#0a0f14', cardBg: '#111a1f', border: '#1e2e32',
    text: '#c8d4d0', textDim: '#568079', textBright: '#e0f0e8', placeholder: '#568079',
    accentOpacity: 1, bgOpacity: 1, cardBgOpacity: 1, borderOpacity: 1, textDimOpacity: 1, placeholderOpacity: 0.5,
  },
  warm: {
    accent: '#f59e0b', accent2: '#ef4444', bg: '#1a0f0a', cardBg: '#241a14', border: '#3e2a1e',
    text: '#d4c8c0', textDim: '#796556', textBright: '#f0e8e0', placeholder: '#796556',
    accentOpacity: 1, bgOpacity: 1, cardBgOpacity: 1, borderOpacity: 1, textDimOpacity: 1, placeholderOpacity: 0.5,
  },
  'pro-blue': {
    accent: '#3b82f6', accent2: '#06b6d4', bg: '#0a0f1a', cardBg: '#111a28', border: '#1e2e3e',
    text: '#c0c8d4', textDim: '#566579', textBright: '#e0e8f0', placeholder: '#566579',
    accentOpacity: 1, bgOpacity: 1, cardBgOpacity: 1, borderOpacity: 1, textDimOpacity: 1, placeholderOpacity: 0.5,
  },
  'high-contrast': {
    accent: '#ffffff', accent2: '#f97316', bg: '#000000', cardBg: '#0a0a0a', border: '#333333',
    text: '#ffffff', textDim: '#aaaaaa', textBright: '#ffffff', placeholder: '#aaaaaa',
    accentOpacity: 1, bgOpacity: 1, cardBgOpacity: 1, borderOpacity: 1, textDimOpacity: 1, placeholderOpacity: 0.6,
  },
}

// ── Helpers ────────────────────────────────────────────────────

function hexToRgba(hex: string, opacity: number): string {
  if (!hex || typeof hex !== 'string') return `rgba(0, 0, 0, ${opacity})`
  const num = parseInt(hex.replace('#', ''), 16)
  if (isNaN(num)) return `rgba(0, 0, 0, ${opacity})`
  const r = (num >> 16) & 0xFF
  const g = (num >> 8) & 0xFF
  const b = num & 0xFF
  return `rgba(${r}, ${g}, ${b}, ${opacity})`
}

function lighten(hex: string, amount: number): string {
  const num = parseInt(hex.replace('#', ''), 16)
  const r = Math.min(255, (num >> 16) + Math.round(255 * amount))
  const g = Math.min(255, ((num >> 8) & 0x00FF) + Math.round(255 * amount))
  const b = Math.min(255, (num & 0x0000FF) + Math.round(255 * amount))
  return `#${(r << 16 | g << 8 | b).toString(16).padStart(6, '0')}`
}

function applyColorOverrides(colors: {
  accent: string; accentOpacity: number
  accent2: string; accent2Opacity: number
  bg: string; bgOpacity: number
  cardBg: string; cardBgOpacity: number
  border: string; borderOpacity: number
}) {
  const root = document.documentElement
  const parts: string[] = []
  root.style.setProperty('--accent', hexToRgba(colors.accent, colors.accentOpacity), 'important')
  root.style.setProperty('--accent-hover', hexToRgba(lighten(colors.accent, 0.15), colors.accentOpacity), 'important')
  root.style.setProperty('--accent-glow', hexToRgba(colors.accent, 0.2), 'important')
  root.style.setProperty('--accent2', hexToRgba(colors.accent2, colors.accent2Opacity), 'important')
  root.style.setProperty('--accent2-hover', hexToRgba(lighten(colors.accent2, 0.15), colors.accent2Opacity), 'important')
  parts.push(`--accent:${hexToRgba(colors.accent, colors.accentOpacity)}!important`)
  parts.push(`--accent-hover:${hexToRgba(lighten(colors.accent, 0.15), colors.accentOpacity)}!important`)
  parts.push(`--accent-glow:${hexToRgba(colors.accent, 0.2)}!important`)
  parts.push(`--accent2:${hexToRgba(colors.accent2, colors.accent2Opacity)}!important`)
  parts.push(`--accent2-hover:${hexToRgba(lighten(colors.accent2, 0.15), colors.accent2Opacity)}!important`)
  root.style.setProperty('--bg', hexToRgba(colors.bg, colors.bgOpacity), 'important')
  parts.push(`--bg:${hexToRgba(colors.bg, colors.bgOpacity)}!important`)
  root.style.setProperty('--bg-card', hexToRgba(colors.cardBg, colors.cardBgOpacity), 'important')
  parts.push(`--bg-card:${hexToRgba(colors.cardBg, colors.cardBgOpacity)}!important`)
  // Derive --bg-hover and --bg-active from card bg
  const hoverColor = lighten(colors.cardBg, 0.06)
  const activeColor = lighten(colors.cardBg, 0.12)
  root.style.setProperty('--bg-hover', hexToRgba(hoverColor, colors.cardBgOpacity), 'important')
  root.style.setProperty('--bg-active', hexToRgba(activeColor, colors.cardBgOpacity), 'important')
  parts.push(`--bg-hover:${hexToRgba(hoverColor, colors.cardBgOpacity)}!important`)
  parts.push(`--bg-active:${hexToRgba(activeColor, colors.cardBgOpacity)}!important`)
  root.style.setProperty('--border', hexToRgba(colors.border, colors.borderOpacity), 'important')
  // Derive --border-glow from border color
  const glowColor = lighten(colors.border, 0.15)
  root.style.setProperty('--border-glow', hexToRgba(glowColor, colors.borderOpacity), 'important')
  parts.push(`--border:${hexToRgba(colors.border, colors.borderOpacity)}!important`)
  parts.push(`--border-glow:${hexToRgba(glowColor, colors.borderOpacity)}!important`)
  // Reuse the style tag instead of recreating on every call
  let styleEl = document.getElementById('phonon-color-overrides') as HTMLStyleElement | null
  if (!styleEl) {
    styleEl = document.createElement('style')
    styleEl.id = 'phonon-color-overrides'
    document.head.appendChild(styleEl)
  }
  styleEl.textContent = `:root{${parts.join(';')}}`
}

function applyTextColors(
  textColor: string,
  textBrightColor: string,
  textDimColor: string,
  textDimOpacity: number,
  placeholderColor: string = textDimColor,
  placeholderOpacity: number = 0.5,
) {
  const root = document.documentElement
  root.style.setProperty('--text', textColor, 'important')
  root.style.setProperty('--text-bright', textBrightColor, 'important')
  root.style.setProperty('--text-dim', hexToRgba(textDimColor, textDimOpacity), 'important')
  root.style.setProperty('--placeholder-text', hexToRgba(placeholderColor, placeholderOpacity), 'important')
}

function applyFontSize(size: number) {
  document.documentElement.style.setProperty('--font-size', `${size}px`)
  document.body.style.fontSize = `${size}px`
}

function applyFontFamily(family: string) {
  if (family) {
    document.documentElement.style.setProperty('--sans', `${family}, sans-serif`)
  } else {
    document.documentElement.style.removeProperty('--sans')
  }
}

// ── ModalDialog Component ──────────────────────────────────────

function ModalDialog({
  type,
  title,
  message,
  defaultValue,
  onConfirm,
  onCancel,
}: {
  type: 'prompt' | 'confirm'
  title: string
  message: string
  defaultValue?: string
  onConfirm: (value?: string) => void
  onCancel: () => void
}) {
  const { t } = useI18n()
  const inputRef = useRef<HTMLInputElement>(null)

  useEffect(() => {
    if (type === 'prompt' && inputRef.current) {
      inputRef.current.focus()
      inputRef.current.select()
    }
  }, [type])

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter') {
      onConfirm(type === 'prompt' ? inputRef.current?.value : undefined)
    } else if (e.key === 'Escape') {
      onCancel()
    }
  }

  return (
    <div
      style={{
        position: 'fixed', inset: 0, zIndex: 10000,
        background: 'rgba(0,0,0,0.6)', backdropFilter: 'blur(4px)',
        display: 'flex', alignItems: 'center', justifyContent: 'center',
      }}
      onClick={onCancel}
    >
      <div
        style={{
          background: 'var(--bg-card)', border: '1px solid var(--border-glow)',
          borderRadius: 'var(--radius)', padding: '20px 24px',
          minWidth: 360, maxWidth: 480,
          boxShadow: '0 12px 40px rgba(0,0,0,0.5)',
        }}
        onClick={(e) => e.stopPropagation()}
        onKeyDown={handleKeyDown}
      >
        <h3 style={{ margin: '0 0 8px', fontSize: 'var(--text-lg)' }}>{title}</h3>
        {message && (
          <p style={{ margin: '0 0 16px', fontSize: 'var(--text-base)', color: 'var(--text-dim)', whiteSpace: 'pre-wrap', lineHeight: 1.6 }}>
            {message}
          </p>
        )}
        {type === 'prompt' && (
          <input
            ref={inputRef}
            type="text"
            defaultValue={defaultValue}
            style={{
              width: '100%', boxSizing: 'border-box',
              padding: '8px 12px', fontSize: 'var(--text-base)',
              background: 'var(--bg)', color: 'var(--text)',
              border: '1px solid var(--border)',
              borderRadius: 'var(--radius-sm)',
              outline: 'none',
              marginBottom: 16,
            }}
          />
        )}
        <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 8 }}>
          <button className="btn btn-outline" onClick={onCancel}>{t('common.cancel')}</button>
          <button className="btn btn-primary" onClick={() => onConfirm(type === 'prompt' ? inputRef.current?.value : 'confirmed')}>
            {t('common.ok')}
          </button>
        </div>
      </div>
    </div>
  )
}

// ── Main Settings Component ────────────────────────────────────

export default function Settings({ volumeMode, onVolumeModeChange, addToast, onSpectrumColorSchemeChange, onSpectrumSettingsChange }: SettingsProps) {
  const { t, lang, setLang } = useI18n()

  // ── Settings tabs ──────────────────────────────────────────
  const [settingsTab, setSettingsTab] = useState<string>('general')
  const settingsTabs = [
    { id: 'general', label: t('settings.tab.general') },
    { id: 'audio', label: t('settings.tab.audio') },
    { id: 'appearance', label: t('settings.tab.appearance') },
    { id: 'about', label: t('settings.tab.about') },
  ]

  // ── Theme state ────────────────────────────────────────────
  const [themePreset, setThemePreset] = useState<ThemePreset>(() => {
    const saved = localStorage.getItem('phonon-theme-preset')
    return (saved as ThemePreset) || 'dark'
  })
  const [accentColor, setAccentColor] = useState(() => localStorage.getItem('phonon-accent-color') || THEME_PRESETS[themePreset]?.accent || '#06b6d4')
  const [accent2Color, setAccent2Color] = useState(() => localStorage.getItem('phonon-accent2-color') || THEME_PRESETS[themePreset]?.accent2 || '#6366f1')
  const [bgColor, setBgColor] = useState(() => localStorage.getItem('phonon-bg-color') || THEME_PRESETS[themePreset]?.bg || '#0a0a14')
  const [cardBgColor, setCardBgColor] = useState(() => localStorage.getItem('phonon-card-bg-color') || THEME_PRESETS[themePreset]?.cardBg || '#111122')
  const [borderColor, setBorderColor] = useState(() => localStorage.getItem('phonon-border-color') || THEME_PRESETS[themePreset]?.border || '#1e1e32')
  const [accentOpacity, setAccentOpacity] = useState(() => {
    const v = localStorage.getItem('phonon-accent-opacity')
    return v !== null ? parseFloat(v) : THEME_PRESETS[themePreset]?.accentOpacity ?? 1
  })
  const [accent2Opacity, setAccent2Opacity] = useState(() => {
    const v = localStorage.getItem('phonon-accent2-opacity')
    return v !== null ? parseFloat(v) : THEME_PRESETS[themePreset]?.accent2Opacity ?? 1
  })
  const [bgOpacity, setBgOpacity] = useState(() => {
    const v = localStorage.getItem('phonon-bg-opacity')
    return v !== null ? parseFloat(v) : THEME_PRESETS[themePreset]?.bgOpacity ?? 1
  })
  const [cardBgOpacity, setCardBgOpacity] = useState(() => {
    const v = localStorage.getItem('phonon-cardbg-opacity')
    return v !== null ? parseFloat(v) : THEME_PRESETS[themePreset]?.cardBgOpacity ?? 1
  })
  const [borderOpacity, setBorderOpacity] = useState(() => {
    const v = localStorage.getItem('phonon-border-opacity')
    return v !== null ? parseFloat(v) : THEME_PRESETS[themePreset]?.borderOpacity ?? 1
  })
  const [textColor, setTextColor] = useState(() => localStorage.getItem('phonon-text-color') || THEME_PRESETS[themePreset]?.text || '#c8c8d4')
  const [textBrightColor, setTextBrightColor] = useState(() => localStorage.getItem('phonon-text-bright-color') || THEME_PRESETS[themePreset]?.textBright || '#e8e8f0')
  const [textDimColor, setTextDimColor] = useState(() => localStorage.getItem('phonon-text-dim-color') || THEME_PRESETS[themePreset]?.textDim || '#565d79')
  const [textDimOpacity, setTextDimOpacity] = useState(() => {
    const v = localStorage.getItem('phonon-text-dim-opacity')
    return v !== null ? parseFloat(v) : THEME_PRESETS[themePreset]?.textDimOpacity ?? 1
  })
  const [placeholderColor, setPlaceholderColor] = useState(() => localStorage.getItem('phonon-placeholder-color') || THEME_PRESETS[themePreset]?.placeholder || '#565d79')
  const [placeholderOpacity, setPlaceholderOpacity] = useState(() => {
    const v = localStorage.getItem('phonon-placeholder-opacity')
    return v !== null ? parseFloat(v) : THEME_PRESETS[themePreset]?.placeholderOpacity ?? 0.5
  })
  // Refs that hold the MOST RECENT values for color + opacity. The rAF
  // batch scheduler reads from these (not from React state closure) so
  // every drag tick sees the latest slider value even if the component
  // hasn't re-rendered yet. Kept in sync by handleColorChange /
  // handleOpacityChange immediately after setState.
  const colorValuesRef = useRef<Record<string, string>>({})
  const opacityValuesRef = useRef<Record<string, number>>({})
  colorValuesRef.current.accent = accentColor
  colorValuesRef.current.accent2 = accent2Color
  colorValuesRef.current.bg = bgColor
  colorValuesRef.current.cardBg = cardBgColor
  colorValuesRef.current.border = borderColor
  colorValuesRef.current.text = textColor
  colorValuesRef.current.textBright = textBrightColor
  colorValuesRef.current.textDim = textDimColor
  colorValuesRef.current.placeholder = placeholderColor
  opacityValuesRef.current.accent = accentOpacity
  opacityValuesRef.current.accent2 = accent2Opacity
  opacityValuesRef.current.bg = bgOpacity
  opacityValuesRef.current.cardBg = cardBgOpacity
  opacityValuesRef.current.border = borderOpacity
  opacityValuesRef.current.textDim = textDimOpacity
  opacityValuesRef.current.placeholder = placeholderOpacity
  const [fontSize, setFontSize] = useState(() => {
    const v = localStorage.getItem('phonon-font-size')
    return v !== null ? parseFloat(v) : 14.5
  })
  const [fontFamily, setFontFamily] = useState(() => localStorage.getItem('phonon-font-family') || '')
  const [albumArtRadius, setAlbumArtRadius] = useState(() => {
    const v = localStorage.getItem('phonon-album-art-radius')
    return v !== null ? parseInt(v) : 14
  })
  const [albumArtBlur, setAlbumArtBlur] = useState(() => localStorage.getItem('phonon-album-art-blur') === 'true')

  // Desktop lyrics settings (synced via Rust backend to both windows)
  const [desktopLyricsFontSize, setDesktopLyricsFontSize] = useState(22)
  const [desktopLyricsOpacity, setDesktopLyricsOpacity] = useState(1)
  const [desktopLyricsBgOpacity, setDesktopLyricsBgOpacity] = useState(0.45)
  const [desktopLyricsAlwaysOnTop, setDesktopLyricsAlwaysOnTop] = useState(true)
  // Track current locked state from backend without exposing a UI toggle.
  // The lock is controlled from the floating window's lock button instead.
  const desktopLyricsLockedRef = useRef(false)
  const [desktopLyricsPlayedColor, setDesktopLyricsPlayedColor] = useState('#ffffff')
  const [desktopLyricsUnplayedColor, setDesktopLyricsUnplayedColor] = useState('#888888')
  const [desktopLyricsBgMode, setDesktopLyricsBgMode] = useState('hidden')
  const [desktopLyricsBgPath, setDesktopLyricsBgPath] = useState('')
  const [desktopLyricsLineMode, setDesktopLyricsLineMode] = useState('single')
  const [desktopLyricsScrollFx, setDesktopLyricsScrollFx] = useState(true)
  const [desktopLyricsLineStyle, setDesktopLyricsLineStyle] = useState('scroll')

  // Custom color presets — use readStorage for validation + degradation (spec §12).
  const [customPresets, setCustomPresets] = useState<Record<string, PresetColors>>(() => {
    return readStorage('customPresets') as unknown as Record<string, PresetColors>
  })
  const [renamingPreset, setRenamingPreset] = useState<string | null>(null)

  // ── Custom CSS ─────────────────────────────────────────────
  const [customCss, setCustomCss] = useState(() => localStorage.getItem('phonon-custom-css') || '')
  const [cssEnabled, setCssEnabled] = useState(() => localStorage.getItem('phonon-css-enabled') !== 'false')
  const cssFileInputRef = useRef<HTMLInputElement>(null)
  const presetNameInputRef = useRef<HTMLInputElement>(null)

  // ── Background Image ───────────────────────────────────────
  const [bgImages, setBgImages] = useState<string[]>(() => {
    try {
      const saved = localStorage.getItem('phonon-bg-images')
      return saved ? JSON.parse(saved) : []
    } catch { return [] }
  })
  const [bgImageIndex, setBgImageIndex] = useState(() => Number(localStorage.getItem('phonon-bg-image-index') || '0'))
  const [bgEnabled, setBgEnabled] = useState(() => localStorage.getItem('phonon-bg-enabled') !== 'false')
  const [bgDeleteMode, setBgDeleteMode] = useState(false)
  const [cropVisible, setCropVisible] = useState(false)
  const [cropSource, setCropSource] = useState('')
  // Separate crop state for desktop lyrics background image
  const [dlCropVisible, setDlCropVisible] = useState(false)
  const [dlCropSource, setDlCropSource] = useState('')

  // ── General settings ───────────────────────────────────────
  const [startupBehavior, setStartupBehavior] = useState<StartupBehavior>('Remember')
  const [hotplugNotifications, setHotplugNotifications] = useState(true)
  const [closeToTray, setCloseToTray] = useState(true)
  const [debugLog, setDebugLog] = useState(false)
  const [lyricsApiUrl, setLyricsApiUrl] = useState('https://lrclib.net')
  // §14 合规：歌词 API 默认关闭（白名单置灰策略），用户主动启用后才能输入 URL
  const [lyricsApiEnabled, setLyricsApiEnabled] = useState<boolean>(() => readStorage('lyricsApiEnabled'))
  const [language, setLanguage] = useState<Lang>(lang)
  const [shortcuts, setShortcuts] = useState<ShortcutBinding[]>([])
  const [loading, setLoading] = useState(true)

  // ── Audio settings ─────────────────────────────────────────
  const [replayGainMode, setReplayGainMode] = useState<RgMode>(() => (localStorage.getItem('phonon-replay-gain') as RgMode) || 'Off')
  const [replayGainPreamp, setReplayGainPreamp] = useState(() => Number(localStorage.getItem('phonon-replay-gain-preamp') || '0.0'))
  const [bitDepth, setBitDepth] = useState(() => localStorage.getItem('phonon-bit-depth') || 'Float32')
  const [outputSampleRate, setOutputSampleRate] = useState(() => Number(localStorage.getItem('phonon-sample-rate') || '0'))
  const [dsdMode, setDsdMode] = useState(() => localStorage.getItem('phonon-dsd-mode') || 'Off')
  // WASAPI exclusive mode — actual backend state, not localStorage (the
  // backend owns persistence in settings.json and may revert on failure).
  const [exclusive, setExclusive] = useState(false)
  // Platform gating: exclusive mode, DSD and hardware volume are
  // Windows-only features (WASAPI).
  const [platform, setPlatform] = useState<'windows' | 'other' | ''>('')
  const [audioDiag, setAudioDiag] = useState<AudioDiagInfo | null>(null)
  // Build identity — proves which commit an installed app carries.
  const [buildInfo, setBuildInfo] = useState<{ version: string; build_sha: string } | null>(null)

  interface AudioDiagInfo {
    mode: string
    format: string
    sample_rate: number
    channels: number
    buffer_frames: number
    padding_frames: number
    ring_capacity_frames: number
    ring_available_frames: number
    underruns: number
    device_name: string | null
    output_running: boolean
  }
  const [quality, setQuality] = useState<Quality>(() => (localStorage.getItem('phonon-quality') as Quality) || 'Balanced')

  // ── Hotplug policy (Plan B) ──────────────────────────────
  const [hotplugStatus, setHotplugStatus] = useState<HotplugStatus | null>(null)

  // ── Spectrum settings ──────────────────────────────────────
  const [spectrumSmoothing, setSpectrumSmoothing] = useState(() => Number(localStorage.getItem('phonon-spectrum-smoothing') || '0.35'))
  const [spectrumDecay, setSpectrumDecay] = useState(() => Number(localStorage.getItem('phonon-spectrum-decay') || '0.90'))
  const [spectrumFps, setSpectrumFps] = useState(() => Number(localStorage.getItem('phonon-spectrum-fps') || '60'))
  const [spectrumColorScheme, setSpectrumColorScheme] = useState<ColorScheme>(() => (localStorage.getItem('phonon-spectrum-colorscheme') as ColorScheme) || 'aurora')
  const [customSpectrumColors, setCustomSpectrumColors] = useState(() => {
    const saved = localStorage.getItem('phonon-spectrum-custom-colors')
    if (saved) {
      try { return JSON.parse(saved) }
      catch { /* ignore */ }
    }
    return { bottom: '#003153', top: '#00BFFF', peak: '#00FFFF', grid: '#404040' }
  })

  // ── Modal state ────────────────────────────────────────────
  const [modal, setModal] = useState<{
    type: 'prompt' | 'confirm'
    title: string
    message: string
    defaultValue?: string
    resolve: (value?: string) => void
  } | null>(null)

  // ── Load settings from backend on mount ────────────────────
  useEffect(() => {
    const load = async () => {
      const ls = (k: string, def: string) => localStorage.getItem(k) || def
      try {
        const [
          sBehavior, hotplug, close, debug, rg, rgPreamp,
          bit, sampleRate, dsd, qual, sSmooth, sDecay, sFps, sScheme,
          lyricsUrl, dlSettings,
        ] = await Promise.all([
          invoke<string>('get_startup_behavior').catch(() => ls('phonon-startup-behavior', 'Remember')),
          invoke<boolean>('get_hotplug_notifications').catch(() => localStorage.getItem('phonon-hotplug-notifications') !== 'false'),
          invoke<boolean>('get_close_to_tray').then(async (rustVal) => {
            // 优先使用 localStorage 中用户明确保存的值（Rust 端没有持久化，
            // 每次启动都返回默认 true，会覆盖用户之前的『退出程序』选择）。
            const saved = localStorage.getItem('phonon-close-to-tray')
            if (saved !== null) {
              const savedBool = saved === 'true'
              // 同步回 Rust 端，保证本次会话内行为一致
              if (savedBool !== rustVal) {
                try { await invoke('set_close_to_tray', { enabled: savedBool }) } catch { /* ignore */ }
              }
              return savedBool
            }
            return rustVal
          }).catch(() => localStorage.getItem('phonon-close-to-tray') !== 'false'),
          invoke<boolean>('get_debug_log').catch(() => localStorage.getItem('phonon-debug-log') === 'true'),
          invoke<string>('get_replay_gain').catch(() => ls('phonon-replay-gain', 'Off')),
          invoke<number>('get_replay_gain_preamp').catch(() => Number(localStorage.getItem('phonon-replay-gain-preamp') || '0.0')),
          invoke<string>('get_bit_depth').catch(() => ls('phonon-bit-depth', 'Float32')),
          invoke<number>('get_output_sample_rate').catch(() => Number(localStorage.getItem('phonon-sample-rate') || '0')),
          invoke<string>('get_dsd_mode').catch(() => ls('phonon-dsd-mode', 'Off')),
          // NOTE: no backend getters exist for quality/spectrum smoothing/
          // decay/fps — these were previously invoked as phantom commands
          // that always rejected and fell back to localStorage. Read the
          // localStorage values directly (identical effective behavior).
          ls('phonon-quality', 'Balanced'),
          Number(localStorage.getItem('phonon-spectrum-smoothing') || '0.35'),
          Number(localStorage.getItem('phonon-spectrum-decay') || '0.90'),
          Number(localStorage.getItem('phonon-spectrum-fps') || '60'),
          invoke<string>('get_spectrum_colorscheme').catch(() => ls('phonon-spectrum-colorscheme', 'aurora')),
          invoke<string>('get_lyrics_api_url').catch(() => 'https://lrclib.net'),
          invoke<{
            font_size: number; opacity: number; text_color: string;
            locked: boolean; bg_opacity: number;
            always_on_top: boolean; played_color: string; unplayed_color: string;
            bg_mode: string; bg_path: string; line_mode: string;
            scroll_fx?: boolean; line_style?: string;
          }>('get_desktop_lyrics_settings').catch(() => ({
            font_size: 22, opacity: 1, text_color: '#ffffff',
            locked: false, bg_opacity: 0.45,
            always_on_top: true, played_color: '#ffffff', unplayed_color: '#888888',
            bg_mode: 'hidden', bg_path: '', line_mode: 'single',
            scroll_fx: true, line_style: 'scroll',
          })),
        ])
        setStartupBehavior(sBehavior as StartupBehavior)
        setHotplugNotifications(hotplug)
        setCloseToTray(close)
        setDebugLog(debug)
        setReplayGainMode(rg as RgMode)
        setReplayGainPreamp(rgPreamp)
        setBitDepth(bit || 'Float32')
        if (bit) localStorage.setItem('phonon-bit-depth', bit)
        setOutputSampleRate(sampleRate)
        if (sampleRate) localStorage.setItem('phonon-sample-rate', String(sampleRate))
        setDsdMode(dsd || 'Off')
        if (dsd) localStorage.setItem('phonon-dsd-mode', dsd)
        setQuality(qual as Quality)
        if (qual) localStorage.setItem('phonon-quality', qual)
        setSpectrumSmoothing(sSmooth)
        setSpectrumDecay(sDecay)
        setSpectrumFps(sFps)
        // Prefer localStorage value to avoid backend default overriding custom selection
        const savedScheme = localStorage.getItem('phonon-spectrum-colorscheme') as ColorScheme
        setSpectrumColorScheme(savedScheme || (sScheme as ColorScheme))
        setLyricsApiUrl(lyricsUrl as string)
        setDesktopLyricsFontSize(dlSettings.font_size)
        setDesktopLyricsOpacity(dlSettings.opacity)
        setDesktopLyricsBgOpacity(dlSettings.bg_opacity)
        setDesktopLyricsAlwaysOnTop(dlSettings.always_on_top)
        desktopLyricsLockedRef.current = dlSettings.locked
        setDesktopLyricsPlayedColor(dlSettings.played_color)
        setDesktopLyricsUnplayedColor(dlSettings.unplayed_color)
        setDesktopLyricsBgMode(dlSettings.bg_mode || 'hidden')
        setDesktopLyricsBgPath(dlSettings.bg_path || '')
        setDesktopLyricsLineMode(dlSettings.line_mode || 'single')
        setDesktopLyricsScrollFx(dlSettings.scroll_fx ?? true)
        setDesktopLyricsLineStyle(dlSettings.line_style ?? 'scroll')
        // If color scheme is 'custom', notify parent so it applies the custom colors
        if (savedScheme === 'custom' || sScheme === 'custom') {
          onSpectrumColorSchemeChange('custom', customSpectrumColors)
        }
      } catch {
        /* ignore — defaults are fine */
      } finally {
        setLoading(false)
        applyColorOverrides({
          accent: accentColor, accent2: accent2Color, accentOpacity, accent2Opacity,
          bg: bgColor, bgOpacity,
          cardBg: cardBgColor, cardBgOpacity,
          border: borderColor, borderOpacity,
        })
        applyTextColors(textColor, textBrightColor, textDimColor, textDimOpacity, placeholderColor, placeholderOpacity)
      }
    }
    load()
    loadShortcuts()
  }, [])

  // ── Shortcut management ────────────────────────────────────
  const loadShortcuts = () => {
    try {
      const raw = localStorage.getItem('phonon_shortcuts')
      if (raw) {
        const bindings = JSON.parse(raw)
        setShortcuts(bindings)
      }
    } catch { /* ignore */ }
  }

  const saveShortcuts = (bindings: ShortcutBinding[]) => {
    localStorage.setItem('phonon_shortcuts', JSON.stringify(bindings))
    setShortcuts(bindings)
    window.dispatchEvent(new CustomEvent('shortcuts-changed'))
  }

  const [recordingAction, setRecordingAction] = useState<string | null>(null)

  const handleShortcutRecord = (action: string) => {
    setRecordingAction(action)
  }

  useEffect(() => {
    if (!recordingAction) return
    const handleKeyDown = (e: KeyboardEvent) => {
      e.preventDefault()
      e.stopPropagation()
      if (e.key === 'Escape') {
        setRecordingAction(null)
        return
      }
      // Build shortcut string from e.code (compatible with Tauri global-shortcut)
      const parts: string[] = []
      if (e.ctrlKey) parts.push('Control')
      if (e.altKey) parts.push('Alt')
      if (e.shiftKey) parts.push('Shift')
      if (e.metaKey) parts.push('Super')
      // Skip pure modifier presses
      if (['ControlLeft', 'ControlRight', 'AltLeft', 'AltRight', 'ShiftLeft', 'ShiftRight', 'MetaLeft', 'MetaRight'].includes(e.code)) return
      parts.push(e.code)
      const shortcutStr = parts.join('+')
      const conflict = shortcuts.find((s) => s.action !== recordingAction && s.keys === shortcutStr)
      if (conflict) {
        addToast(t('settings.shortcuts.conflict'), 'warn')
        return
      }
      const updated = shortcuts.filter((s) => s.action !== recordingAction)
      updated.push({ keys: shortcutStr, action: recordingAction })
      saveShortcuts(updated)
      setRecordingAction(null)
    }
    window.addEventListener('keydown', handleKeyDown, true)
    return () => window.removeEventListener('keydown', handleKeyDown, true)
  }, [recordingAction, shortcuts, addToast, t])

  const handleRemoveShortcut = (action: string) => {
    const updated = shortcuts.filter((s) => s.action !== action)
    saveShortcuts(updated)
  }

  // ── Theme handlers ─────────────────────────────────────────
  const handleThemePreset = (preset: ThemePreset) => {
    setThemePreset(preset)
    localStorage.setItem('phonon-theme-preset', preset)
    document.documentElement.setAttribute('data-theme', preset)

    const colors = THEME_PRESETS[preset]
    if (colors) {
      setAccentColor(colors.accent)
      setAccent2Color(colors.accent2)
      setBgColor(colors.bg)
      setCardBgColor(colors.cardBg)
      setBorderColor(colors.border)
      setAccentOpacity(colors.accentOpacity)
      setAccent2Opacity(colors.accent2Opacity ?? colors.accentOpacity)
      setBgOpacity(colors.bgOpacity)
      setCardBgOpacity(colors.cardBgOpacity)
      setBorderOpacity(colors.borderOpacity)
      setTextColor(colors.text)
      setTextDimColor(colors.textDim)
      setTextBrightColor(colors.textBright)
      setTextDimOpacity(colors.textDimOpacity)
      setPlaceholderColor(colors.placeholder)
      setPlaceholderOpacity(colors.placeholderOpacity)

      localStorage.setItem('phonon-accent-color', colors.accent)
      localStorage.setItem('phonon-accent2-color', colors.accent2)
      localStorage.setItem('phonon-bg-color', colors.bg)
      localStorage.setItem('phonon-card-bg-color', colors.cardBg)
      localStorage.setItem('phonon-border-color', colors.border)
      localStorage.setItem('phonon-accent-opacity', String(colors.accentOpacity))
      localStorage.setItem('phonon-accent2-opacity', String(colors.accent2Opacity ?? colors.accentOpacity))
      localStorage.setItem('phonon-bg-opacity', String(colors.bgOpacity))
      localStorage.setItem('phonon-cardbg-opacity', String(colors.cardBgOpacity))
      localStorage.setItem('phonon-border-opacity', String(colors.borderOpacity))
      localStorage.setItem('phonon-text-color', colors.text)
      localStorage.setItem('phonon-text-bright-color', colors.textBright)
      localStorage.setItem('phonon-text-dim-color', colors.textDim)
      localStorage.setItem('phonon-text-dim-opacity', String(colors.textDimOpacity))
      localStorage.setItem('phonon-placeholder-color', colors.placeholder)
      localStorage.setItem('phonon-placeholder-opacity', String(colors.placeholderOpacity))

      applyColorOverrides({
        accent: colors.accent, accent2: colors.accent2, accentOpacity: colors.accentOpacity, accent2Opacity: colors.accent2Opacity ?? colors.accentOpacity,
        bg: colors.bg, bgOpacity: colors.bgOpacity,
        cardBg: colors.cardBg, cardBgOpacity: colors.cardBgOpacity,
        border: colors.border, borderOpacity: colors.borderOpacity,
      })
      applyTextColors(colors.text, colors.textBright, colors.textDim, colors.textDimOpacity, colors.placeholder, colors.placeholderOpacity)
    }
  }

  const saveCurrentColorsAsPreset = (name: string): boolean => {
    // Check if preset name already exists (including built-in presets)
    const builtInPresets = ['dark', 'light', 'aurora', 'warm', 'pro-blue', 'high-contrast']
    if (builtInPresets.includes(name) || customPresets[name]) {
      return false // Name already exists
    }
    const newPreset: PresetColors = {
      accent: accentColor, accent2: accent2Color, bg: bgColor, cardBg: cardBgColor, border: borderColor,
      text: textColor, textDim: textDimColor, textBright: textBrightColor, placeholder: placeholderColor,
      accentOpacity, bgOpacity, cardBgOpacity, borderOpacity, textDimOpacity, placeholderOpacity,
    }
    const updated = { ...customPresets, [name]: newPreset }
    setCustomPresets(updated)
    localStorage.setItem('phonon-custom-presets', JSON.stringify(updated))
    setThemePreset(name as ThemePreset)
    localStorage.setItem('phonon-theme-preset', name)
    return true
  }

  const exportPreset = async (name: string) => {
    const preset = customPresets[name]
    if (!preset) return

    try {
      const filePath = await save({
        defaultPath: `${name}.json`,
        filters: [{ name: 'JSON', extensions: ['json'] }],
      })
      if (!filePath) return

      const exportData = {
        name,
        colors: preset,
        exportedAt: new Date().toISOString(),
        phononVersion: '1.0.0',
      }

      const content = JSON.stringify(exportData, null, 2)
      const encoder = new TextEncoder()
      await writeFile(filePath, encoder.encode(content))
      addToast(t('settings.appearance.preset.exported', '预设已导出'))
    } catch (e) {
      console.error('Failed to export preset:', e)
      addToast(t('settings.appearance.preset.exportFailed', '导出失败'), 'error')
    }
  }

  const renamePreset = (oldName: string, newName: string) => {
    const trimmedName = newName.trim()
    if (!trimmedName || trimmedName === oldName) return false

    const builtInPresets = ['dark', 'light', 'aurora', 'warm', 'pro-blue', 'high-contrast']
    if (builtInPresets.includes(trimmedName) || customPresets[trimmedName]) {
      addToast(t('settings.appearance.preset.nameExists', '预设名称已存在，请使用其他名称'), 'warn')
      return false
    }

    const preset = customPresets[oldName]
    if (!preset) return false

    const updated = { ...customPresets }
    delete updated[oldName]
    updated[trimmedName] = preset
    setCustomPresets(updated)
    localStorage.setItem('phonon-custom-presets', JSON.stringify(updated))

    if (themePreset === oldName) {
      setThemePreset(trimmedName as ThemePreset)
      localStorage.setItem('phonon-theme-preset', trimmedName)
    }

    addToast(t('toast.renamedTo', '已重命名为') + ' ' + trimmedName)
    return true
  }

  const deleteCustomPreset = (name: string) => {
    setModal({
      type: 'confirm',
      title: t('common.confirm'),
      message: t('settings.appearance.preset.deleteConfirm', `确定删除预设 "${name}" 吗？\n当前颜色设置不会丢失。`),
      resolve: (val) => {
        if (val === undefined) return
        const wasActive = themePreset === name
        const updated = { ...customPresets }
        delete updated[name]
        setCustomPresets(updated)
        localStorage.setItem('phonon-custom-presets', JSON.stringify(updated))
        if (wasActive) {
          setThemePreset('')
          localStorage.removeItem('phonon-theme-preset')
        }
      },
    })
  }

  // ── Two-tier color pipeline for "real-time feel + no layout thrash" ──
  //
  // SYNC (every pixel tick):
  //   1. Update refs (cheap, same-thread).
  //   2. Call applyColorOverrides / applyTextColors SYNCHRONOUSLY with the
  //      new value. This gives "finger drag is screen change" responsiveness
  //      that React state / rAF-batched-apply simply cannot match.
  //
  // BATCHED (coalesced to avoid redundant work):
  //   3. setState via React → eventually updates color swatch previews +
  //      hex text inputs (low priority; user already sees live CSS change).
  //   4. localStorage save is debounced 500ms (the real hotspot for lag —
  //      sync IO in the hot path) via scheduleColorSave.
  //
  // Result: real-time visible feedback + zero re-render/IO lag.

  const handleColorChange = (key: string, value: string) => {
    const setters: Record<string, React.Dispatch<React.SetStateAction<string>>> = {
      accent: setAccentColor,
      accent2: setAccent2Color,
      bg: setBgColor,
      cardBg: setCardBgColor,
      border: setBorderColor,
      text: setTextColor,
      textBright: setTextBrightColor,
      textDim: setTextDimColor,
      placeholder: setPlaceholderColor,
    }
    const storageKeys: Record<string, string> = {
      accent: 'phonon-accent-color',
      accent2: 'phonon-accent2-color',
      bg: 'phonon-bg-color',
      cardBg: 'phonon-card-bg-color',
      border: 'phonon-border-color',
      text: 'phonon-text-color',
      textBright: 'phonon-text-bright-color',
      textDim: 'phonon-text-dim-color',
      placeholder: 'phonon-placeholder-color',
    }
    // 1. Ref is always up to date, even if setState is async
    colorValuesRef.current[key] = value
    // 2. rAF-batched DOM apply — coalesces all slider pixels within a
    //    single animation frame into ONE applyColorOverrides call.
    //    Visual feedback is still real-time (rAF fires before paint).
    scheduleDomUpdate()
    // 3. React state update — IMMEDIATE (not debounced).
    //    The slider's value={state} is a controlled input; if setState
    //    is delayed, the browser fights the user's drag and "snaps back".
    //    Performance is OK because rAF already batches the expensive
    //    DOM/CSS work; React's re-render is just a vdom diff.
    setters[key]?.(value)
    // 4. Debounced localStorage write — avoid 10+ sync IO ops/sec during drag
    scheduleColorSave(storageKeys[key], value)
  }

  // Debounced localStorage save. Coalesces rapid changes (e.g. slider drag
  // firing dozens of events per second) into a single write 500ms after the
  // last change. Without this, each input event blocks on a sync localStorage
  // write, contributing to slider lag.
  const pendingSavesRef = useRef<Record<string, string>>({})
  const colorSaveTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const scheduleColorSave = (key: string, value: string) => {
    pendingSavesRef.current[key] = value
    if (colorSaveTimerRef.current) clearTimeout(colorSaveTimerRef.current)
    colorSaveTimerRef.current = setTimeout(() => {
      colorSaveTimerRef.current = null
      for (const [k, v] of Object.entries(pendingSavesRef.current)) {
        localStorage.setItem(k, v)
      }
      pendingSavesRef.current = {}
    }, 500)
  }

  // ── Debounced React state commit ──
  // The #1 cause of slider lag: every input event calls setState →
  // React re-renders this entire 2700-line component. By deferring
  // the setState by 100ms, we coalesce dozens of drag ticks into a
  // single re-render. The DOM is already updated synchronously
  // (applyColorOverrides / applyFontSize), so the user sees instant
  // visual feedback; the React state (hex text preview, slider value
  // label) catches up 100ms after the user pauses.
  const stateCommitTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  // ── rAF-batched DOM update ──
  // Coalesces applyColorOverrides + applyTextColors calls across all
  // slider pixels within a single animation frame into ONE DOM update
  // per frame. This is the key to smooth slider dragging: instead of
  // 60 sync style recalculations per second (each blocking the main
  // thread), we do at most 1 per frame (~16ms), which the browser
  // coalesces with its own rendering pipeline. The user still sees
  // real-time color changes — rAF fires before paint, so the visual
  // feedback is in the same frame as the slider movement.
  const domUpdateRafRef = useRef<number | null>(null)
  const flushDomUpdate = useCallback(() => {
    domUpdateRafRef.current = null
    applyColorOverrides({
      accent: colorValuesRef.current.accent,
      accent2: colorValuesRef.current.accent2,
      accentOpacity: opacityValuesRef.current.accent,
      accent2Opacity: opacityValuesRef.current.accent2,
      bg: colorValuesRef.current.bg,
      bgOpacity: opacityValuesRef.current.bg,
      cardBg: colorValuesRef.current.cardBg,
      cardBgOpacity: opacityValuesRef.current.cardBg,
      border: colorValuesRef.current.border,
      borderOpacity: opacityValuesRef.current.border,
    })
    applyTextColors(
      colorValuesRef.current.text,
      colorValuesRef.current.textBright,
      colorValuesRef.current.textDim,
      opacityValuesRef.current.textDim,
      colorValuesRef.current.placeholder,
      opacityValuesRef.current.placeholder,
    )
  }, [])
  const scheduleDomUpdate = useCallback(() => {
    if (domUpdateRafRef.current !== null) return
    domUpdateRafRef.current = requestAnimationFrame(flushDomUpdate)
  }, [flushDomUpdate])

  // Cancel any pending rAF / timers on unmount to prevent stray
  // DOM updates after the component is gone (HMR reload scenario).
  useEffect(() => {
    return () => {
      if (domUpdateRafRef.current !== null) {
        cancelAnimationFrame(domUpdateRafRef.current)
        domUpdateRafRef.current = null
      }
      if (stateCommitTimerRef.current) {
        clearTimeout(stateCommitTimerRef.current)
        stateCommitTimerRef.current = null
      }
      if (colorSaveTimerRef.current) {
        clearTimeout(colorSaveTimerRef.current)
        colorSaveTimerRef.current = null
      }
    }
  }, [])

  const handleOpacityChange = (key: string, value: number) => {
    const setters: Record<string, React.Dispatch<React.SetStateAction<number>>> = {
      accent: setAccentOpacity,
      accent2: setAccent2Opacity,
      bg: setBgOpacity,
      cardBg: setCardBgOpacity,
      border: setBorderOpacity,
      textDim: setTextDimOpacity,
      placeholder: setPlaceholderOpacity,
    }
    const storageKeys: Record<string, string> = {
      accent: 'phonon-accent-opacity',
      accent2: 'phonon-accent2-opacity',
      bg: 'phonon-bg-opacity',
      cardBg: 'phonon-cardbg-opacity',
      border: 'phonon-border-opacity',
      textDim: 'phonon-text-dim-opacity',
      placeholder: 'phonon-placeholder-opacity',
    }
    opacityValuesRef.current[key] = value
    // rAF-batched DOM apply — same as handleColorChange
    scheduleDomUpdate()
    // Immediate setState — controlled slider needs sync state or it snaps back
    setters[key]?.(value)
    scheduleColorSave(storageKeys[key], String(value))
  }

  const resetTextColor = () => {
    // Fall back to 'dark' preset if user is on a customized unnamed preset (themePreset === '')
    const key = themePreset || 'dark'
    const preset = THEME_PRESETS[key]
    if (!preset) return
    setTextColor(preset.text)
    setTextBrightColor(preset.textBright)
    setTextDimColor(preset.textDim)
    setTextDimOpacity(preset.textDimOpacity)
    setPlaceholderColor(preset.placeholder)
    setPlaceholderOpacity(preset.placeholderOpacity)
    localStorage.setItem('phonon-text-color', preset.text)
    localStorage.setItem('phonon-text-bright-color', preset.textBright)
    localStorage.setItem('phonon-text-dim-color', preset.textDim)
    localStorage.setItem('phonon-text-dim-opacity', String(preset.textDimOpacity))
    localStorage.setItem('phonon-placeholder-color', preset.placeholder)
    localStorage.setItem('phonon-placeholder-opacity', String(preset.placeholderOpacity))
    applyTextColors(preset.text, preset.textBright, preset.textDim, preset.textDimOpacity, preset.placeholder, preset.placeholderOpacity)
  }
  // keep: reserved for future UI wiring (suppresses unused-variable TS error)
  void resetTextColor

  const handleFontSizeChange = (size: number) => {
    applyFontSize(size)
    setFontSize(size)
    scheduleColorSave('phonon-font-size', String(size))
  }

  const handleFontFamilyChange = (family: string) => {
    setFontFamily(family)
    localStorage.setItem('phonon-font-family', family)
    applyFontFamily(family)
  }

  const handleAlbumArtRadius = (radius: number) => {
    document.documentElement.style.setProperty('--album-art-radius', `${radius}px`)
    setAlbumArtRadius(radius)
    scheduleColorSave('phonon-album-art-radius', String(radius))
  }

  const handleAlbumArtBlur = (blur: boolean) => {
    setAlbumArtBlur(blur)
    localStorage.setItem('phonon-album-art-blur', String(blur))
    document.documentElement.style.setProperty('--album-art-blur', blur ? '1' : '0')
  }

  // ── Desktop lyrics settings ────────────────────────────────
  const pushDesktopLyricsSettings = useCallback(() => {
    const settings = {
      font_size: desktopLyricsFontSize,
      opacity: desktopLyricsOpacity,
      text_color: '#ffffff',
      locked: desktopLyricsLockedRef.current,
      bg_opacity: desktopLyricsBgOpacity,
      always_on_top: desktopLyricsAlwaysOnTop,
      played_color: desktopLyricsPlayedColor,
      unplayed_color: desktopLyricsUnplayedColor,
      bg_mode: desktopLyricsBgMode,
      bg_path: desktopLyricsBgPath,
      line_mode: desktopLyricsLineMode,
      scroll_fx: desktopLyricsScrollFx,
      equal_size: false,
      line_style: desktopLyricsLineStyle,
      shadow_enabled: true,
    }
    invoke('set_desktop_lyrics_settings', { settings }).catch(() => {})
  }, [desktopLyricsFontSize, desktopLyricsOpacity, desktopLyricsBgOpacity, desktopLyricsAlwaysOnTop, desktopLyricsPlayedColor, desktopLyricsUnplayedColor, desktopLyricsBgMode, desktopLyricsBgPath, desktopLyricsLineMode, desktopLyricsScrollFx, desktopLyricsLineStyle])

  const handleDesktopLyricsFontSize = (size: number) => {
    setDesktopLyricsFontSize(size)
  }

  const handleDesktopLyricsOpacity = (opacity: number) => {
    setDesktopLyricsOpacity(opacity)
  }

  const handleDesktopLyricsBgOpacity = (opacity: number) => {
    setDesktopLyricsBgOpacity(opacity)
  }

  const handleDesktopLyricsAlwaysOnTop = (enabled: boolean) => {
    setDesktopLyricsAlwaysOnTop(enabled)
  }

  const handleDesktopLyricsPlayedColor = (color: string) => {
    setDesktopLyricsPlayedColor(color)
  }

  const handleDesktopLyricsUnplayedColor = (color: string) => {
    setDesktopLyricsUnplayedColor(color)
  }

  const handleDesktopLyricsBgMode = (mode: string) => {
    setDesktopLyricsBgMode(mode)
  }

  const handleDesktopLyricsBgPath = (path: string) => {
    setDesktopLyricsBgPath(path)
  }

  const handleDesktopLyricsBgFile = async () => {
    try {
      const selected = await open({
        multiple: false,
        title: t('settings.desktopLyrics.bgFileSelect'),
        filters: [{ name: 'Video', extensions: ['mp4', 'webm', 'mkv', 'avi'] }],
      })
      if (selected && typeof selected === 'string') {
        setDesktopLyricsBgPath(selected)
        setDesktopLyricsBgMode('video')
      }
    } catch { /* user cancelled */ }
  }

  // Image upload → CropEditor → write cropped bytes to app data dir → use as bg_path
  const handleDesktopLyricsBgUpload = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0]
    if (!file) return
    const reader = new FileReader()
    reader.onload = () => {
      setDlCropSource(reader.result as string)
      setDlCropVisible(true)
    }
    reader.readAsDataURL(file)
    e.target.value = ''
  }

  const handleDlCropConfirm = (croppedDataUrl: string) => {
    console.log('[Settings] handleDlCropConfirm received dataUrl length:', croppedDataUrl.length)
    setDlCropVisible(false)
    setDlCropSource('')
    const img = new Image()
    img.onload = () => {
      console.log('[Settings] cropped image loaded, size:', img.width, 'x', img.height)
      const MAX_DIM = 1920
      let w = img.width, h = img.height
      if (w > MAX_DIM || h > MAX_DIM) {
        const ratio = Math.min(MAX_DIM / w, MAX_DIM / h)
        w = Math.round(w * ratio)
        h = Math.round(h * ratio)
      }
      const canvas = document.createElement('canvas')
      canvas.width = w
      canvas.height = h
      const ctx = canvas.getContext('2d')
      if (!ctx) return
      ctx.drawImage(img, 0, 0, w, h)
      const dataUrl = canvas.toDataURL('image/jpeg', 0.85)
      // Send cropped JPEG bytes directly to Rust via IPC — bypasses the
      // tauri-plugin-fs plugin (which is not registered in this build)
      // and its scope configuration. Rust writes the file and returns
      // the absolute path to use as bg_path.
      const payload = dataUrl.split(',')[1]
      const bytes = Uint8Array.from(atob(payload), c => c.charCodeAt(0))
      invoke<string>('save_desktop_lyrics_bg', { bytes: Array.from(bytes) })
        .then((path) => {
          console.log('[Settings] saved desktop lyrics bg to:', path)
          setDesktopLyricsBgPath(path)
          setDesktopLyricsBgMode('image')
        })
        .catch((err) => {
          console.error('[desktopLyrics] failed to save bg file:', err)
        })
    }
    img.src = croppedDataUrl
  }

  const handleDesktopLyricsReset = () => {
    setModal({
      type: 'confirm',
      title: t('common.confirm'),
      message: t('settings.desktopLyrics.resetConfirm'),
      resolve: async (val) => {
        if (val === undefined) return
        try {
          const s = await invoke<{
            font_size: number
            opacity: number
            bg_opacity: number
            always_on_top: boolean
            played_color: string
            unplayed_color: string
            bg_mode: string
            bg_path: string
            locked: boolean
            line_mode: string
            scroll_fx: boolean
            equal_size: boolean
            line_style: string
          }>('reset_desktop_lyrics_settings')
          setDesktopLyricsFontSize(s.font_size)
          setDesktopLyricsOpacity(s.opacity)
          setDesktopLyricsBgOpacity(s.bg_opacity)
          setDesktopLyricsAlwaysOnTop(s.always_on_top)
          setDesktopLyricsPlayedColor(s.played_color)
          setDesktopLyricsUnplayedColor(s.unplayed_color)
          setDesktopLyricsBgMode(s.bg_mode)
          setDesktopLyricsBgPath(s.bg_path)
          setDesktopLyricsLineMode(s.line_mode)
          setDesktopLyricsScrollFx(s.scroll_fx)
          setDesktopLyricsLineStyle(s.line_style || 'scroll')
          desktopLyricsLockedRef.current = s.locked
        } catch (err) {
          console.error('[desktopLyrics] reset failed:', err)
        }
      },
    })
  }

  const handleDlCropCancel = () => {
    setDlCropVisible(false)
    setDlCropSource('')
  }

  // Push desktop lyrics settings to backend when they change
  useEffect(() => {
    if (loading) return
    const timer = setTimeout(() => {
      pushDesktopLyricsSettings()
    }, 200)
    return () => clearTimeout(timer)
  }, [desktopLyricsFontSize, desktopLyricsOpacity, desktopLyricsBgOpacity, desktopLyricsAlwaysOnTop, desktopLyricsPlayedColor, desktopLyricsUnplayedColor, desktopLyricsBgMode, desktopLyricsBgPath, desktopLyricsLineMode, desktopLyricsScrollFx, desktopLyricsLineStyle, loading, pushDesktopLyricsSettings])

  // Keep desktop lyrics settings in sync when the floating window changes
  // them (e.g. via right-click menu), so Settings doesn't override with stale values.
  useEffect(() => {
    const unlisten = listen('desktop-lyrics-settings-updated', (ev: any) => {
      const p = ev.payload
      if (!p) return
      if (typeof p.locked === 'boolean') desktopLyricsLockedRef.current = p.locked
      if (typeof p.line_mode === 'string') setDesktopLyricsLineMode(p.line_mode)
      if (typeof p.bg_opacity === 'number') setDesktopLyricsBgOpacity(p.bg_opacity)
      if (typeof p.font_size === 'number') setDesktopLyricsFontSize(p.font_size)
      if (typeof p.opacity === 'number') setDesktopLyricsOpacity(p.opacity)
      if (typeof p.always_on_top === 'boolean') setDesktopLyricsAlwaysOnTop(p.always_on_top)
      if (typeof p.played_color === 'string') setDesktopLyricsPlayedColor(p.played_color)
      if (typeof p.unplayed_color === 'string') setDesktopLyricsUnplayedColor(p.unplayed_color)
      if (typeof p.bg_mode === 'string') setDesktopLyricsBgMode(p.bg_mode)
      if (typeof p.bg_path === 'string') setDesktopLyricsBgPath(p.bg_path)
      if (typeof p.scroll_fx === 'boolean') setDesktopLyricsScrollFx(p.scroll_fx)
      if (typeof p.line_style === 'string') setDesktopLyricsLineStyle(p.line_style)
    }).catch(() => {})
    return () => { unlisten.then(f => f?.()).catch(() => {}) }
  }, [])

  // ── Custom CSS handlers ────────────────────────────────────
  const injectCustomCss = (css: string) => {
    let el = document.getElementById('phonon-custom-css') as HTMLStyleElement | null
    if (!el) {
      el = document.createElement('style')
      el.id = 'phonon-custom-css'
      document.head.appendChild(el)
    }
    el.textContent = css
    if (el !== document.head.lastElementChild) {
      document.head.appendChild(el)
    }
  }

  const handleCustomCss = (css: string) => {
    setCustomCss(css)
    localStorage.setItem('phonon-custom-css', css)
    if (cssEnabled) injectCustomCss(css)
  }

  const toggleCssEnabled = () => {
    const next = !cssEnabled
    setCssEnabled(next)
    localStorage.setItem('phonon-css-enabled', String(next))
    if (next) {
      injectCustomCss(customCss)
    } else {
      const el = document.getElementById('phonon-custom-css')
      if (el) el.remove()
    }
  }

  const handleCustomCssFile = () => cssFileInputRef.current?.click()

  const handleCssFileChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0]
    if (!file) return
    const reader = new FileReader()
    reader.onload = () => handleCustomCss(reader.result as string)
    reader.readAsText(file)
    e.target.value = ''
  }

  const clearCustomCss = () => {
    setCustomCss('')
    localStorage.removeItem('phonon-custom-css')
    const el = document.getElementById('phonon-custom-css')
    if (el) el.remove()
  }

  // ── Background image handlers ──────────────────────────────
  const applyBgImage = (index: number, images: string[], enabled: boolean) => {
    const el = document.querySelector('.app') as HTMLElement | null
    if (!el) return
    if (!enabled || images.length === 0 || index < 0 || index >= images.length) {
      el.style.backgroundImage = ''
    } else {
      el.style.backgroundImage = `url("${images[index]}")`
    }
  }

  const handleBgImageUpload = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0]
    if (!file) return
    const reader = new FileReader()
    reader.onload = () => {
      setCropSource(reader.result as string)
      setCropVisible(true)
    }
    reader.readAsDataURL(file)
    e.target.value = ''
  }

  const handleCropConfirm = (croppedDataUrl: string) => {
    setCropVisible(false)
    setCropSource('')
    const img = new Image()
    img.onload = () => {
      const MAX_DIM = 1920
      let w = img.width, h = img.height
      if (w > MAX_DIM || h > MAX_DIM) {
        const ratio = Math.min(MAX_DIM / w, MAX_DIM / h)
        w = Math.round(w * ratio)
        h = Math.round(h * ratio)
      }
      const canvas = document.createElement('canvas')
      canvas.width = w
      canvas.height = h
      const ctx = canvas.getContext('2d')
      if (ctx) {
        ctx.drawImage(img, 0, 0, w, h)
        const dataUrl = canvas.toDataURL('image/jpeg', 0.7)
        const nextImages = [...bgImages, dataUrl]
        const nextIndex = nextImages.length - 1
        setBgImages(nextImages)
        setBgImageIndex(nextIndex)
        try { localStorage.setItem('phonon-bg-images', JSON.stringify(nextImages)) }
        catch { /* quota exceeded */ }
        localStorage.setItem('phonon-bg-image-index', String(nextIndex))
        applyBgImage(nextIndex, nextImages, bgEnabled)
      }
    }
    img.src = croppedDataUrl
  }

  const handleCropCancel = () => {
    setCropVisible(false)
    setCropSource('')
  }

  const handleBgEnabled = (enabled: boolean) => {
    setBgEnabled(enabled)
    localStorage.setItem('phonon-bg-enabled', String(enabled))
    applyBgImage(bgImageIndex, bgImages, enabled)
  }

  const handleSelectBgImage = (index: number) => {
    setBgImageIndex(index)
    localStorage.setItem('phonon-bg-image-index', String(index))
    applyBgImage(index, bgImages, bgEnabled)
  }

  const handleDeleteBgImage = (index: number) => {
    const nextImages = bgImages.filter((_, i) => i !== index)
    setBgImages(nextImages)
    try { localStorage.setItem('phonon-bg-images', JSON.stringify(nextImages)) }
    catch { /* ignore */ }
    let nextIndex = bgImageIndex
    if (nextIndex >= nextImages.length) nextIndex = Math.max(0, nextImages.length - 1)
    if (index < bgImageIndex) nextIndex = Math.max(0, bgImageIndex - 1)
    setBgImageIndex(nextIndex)
    localStorage.setItem('phonon-bg-image-index', String(nextIndex))
    applyBgImage(nextIndex, nextImages, bgEnabled)
  }

  // Apply saved background on mount
  useEffect(() => {
    applyBgImage(bgImageIndex, bgImages, bgEnabled)
  }, [])

  // ── General settings handlers ──────────────────────────────
  const handleDebugLog = async () => {
    const next = !debugLog
    setDebugLog(next)
    localStorage.setItem('phonon-debug-log', String(next))
    try { await invoke('set_debug_log', { enabled: next }) } catch { /* ignore */ }
  }

  const handleStartupBehavior = async (behavior: StartupBehavior) => {
    setStartupBehavior(behavior)
    localStorage.setItem('phonon-startup-behavior', behavior)
    try { await invoke('set_startup_behavior', { behavior }) } catch { /* ignore */ }
  }

  const handleHotplugNotifications = async () => {
    const next = !hotplugNotifications
    setHotplugNotifications(next)
    localStorage.setItem('phonon-hotplug-notifications', String(next))
    try { await invoke('set_hotplug_notifications', { enabled: next }) } catch { /* ignore */ }
  }

  const handleCloseToTray = async (enabled?: boolean) => {
    const next = enabled !== undefined ? enabled : !closeToTray
    setCloseToTray(next)
    localStorage.setItem('phonon-close-to-tray', String(next))
    try { await invoke('set_close_to_tray', { enabled: next }) } catch { /* ignore */ }
  }

  const handleLyricsApiUrl = async (url: string) => {
    setLyricsApiUrl(url)
    try { await invoke('set_lyrics_api_url', { url }) } catch { /* ignore */ }
  }

  const handleLanguage = (l: Lang) => {
    setLanguage(l)
    setLang(l)
    localStorage.setItem('phonon-language', l)
    invoke('set_language', { language: l }).catch(() => {})
  }

  // ── Audio settings handlers ────────────────────────────────
  const handleReplayGainMode = async (mode: RgMode) => {
    setReplayGainMode(mode)
    localStorage.setItem('phonon-replay-gain', mode)
    try { await invoke('set_replay_gain', { mode }) } catch { /* ignore */ }
  }

  const handleReplayGainPreamp = (db: number) => {
    setReplayGainPreamp(db)
    localStorage.setItem('phonon-replay-gain-preamp', String(db))
    // Batched rAF invoke to prevent callback-id flooding during slider drag
    pendingRgPreampRef.current = db
    if (rgPreampRafRef.current === null) {
      rgPreampRafRef.current = requestAnimationFrame(flushPendingRgPreamp)
    }
  }

  const handleBitDepth = async (depth: string) => {
    setBitDepth(depth)
    localStorage.setItem('phonon-bit-depth', depth)
    try { await invoke('set_bit_depth', { depth }) } catch { /* ignore */ }
  }

  const handleOutputSampleRate = async (rate: number) => {
    setOutputSampleRate(rate)
    localStorage.setItem('phonon-sample-rate', String(rate))
    try { await invoke('set_output_sample_rate', { rate }) } catch { /* ignore */ }
  }

  const handleDsdMode = async (mode: string) => {
    setDsdMode(mode)
    localStorage.setItem('phonon-dsd-mode', mode)
    try { await invoke('set_dsd_mode', { mode }) } catch { /* ignore */ }
  }

  const handleVolumeMode = async (mode: string) => {
    try {
      await invoke('set_volume_mode', { mode })
      onVolumeModeChange(mode)
    } catch (e) {
      console.error('Failed to set volume mode:', e)
    }
  }

  const handleExclusiveMode = async (enabled: boolean) => {
    if (enabled && !window.confirm(t('confirm.exclusiveMode'))) return
    setExclusive(enabled) // optimistic; backend may revert
    try {
      await setExclusiveMode(enabled)
      // Re-sync with reality: the backend falls back to shared mode when the
      // device rejects exclusive initialization, and restarts playback.
      const actual = await getExclusiveMode()
      setExclusive(actual)
      addToast(actual ? t('toast.exclusiveOn') : t('toast.exclusiveOff'), actual ? 'info' : 'warn')
    } catch (e) {
      setExclusive(false)
      addToast(t('toast.exclusiveFailed'), 'error')
      console.error('Failed to set exclusive mode:', e)
    }
  }

  const handleQuality = async (q: Quality) => {
    setQuality(q)
    localStorage.setItem('phonon-quality', q)
    try { await setResamplerQuality(q) } catch { /* ignore */ }
  }

  // ── Hotplug policy handlers (Plan B) ───────────────────────
  const refreshHotplugStatus = useCallback(async () => {
    try {
      const s = await getHotplugStatus()
      setHotplugStatus(s)
    } catch (e) {
      console.error('Failed to load hotplug status:', e)
    }
  }, [])

  useEffect(() => {
    if (settingsTab === 'about') {
      invoke<{ version: string; build_sha: string }>('get_build_info')
        .then(setBuildInfo)
        .catch(() => {})
    }
    if (settingsTab === 'audio') {
      void refreshHotplugStatus()
      // Platform for gating Windows-only audio cards.
      getPlatform()
        .then((p) => setPlatform(p === 'windows' ? 'windows' : 'other'))
        .catch(() => setPlatform('other'))
      // Sync the exclusive-mode toggle with the backend's actual state.
      getExclusiveMode().then(setExclusive).catch(() => {})
      // Poll audio diagnostics while the tab is open.
      const refresh = () =>
        invoke<AudioDiagInfo>('get_audio_diagnostics').then(setAudioDiag).catch(() => {})
      void refresh()
      const iv = setInterval(refresh, 1000)
      return () => clearInterval(iv)
    }
  }, [settingsTab, refreshHotplugStatus])

  const handleOnDeviceRemoved = async (v: DeviceRemovedStrategy) => {
    try {
      await setOnDeviceRemoved(v)
      await refreshHotplugStatus()
    } catch (e) {
      console.error('Failed to set on_device_removed:', e)
    }
  }

  const handleOnNewDeviceInserted = async (v: DeviceInsertedStrategy) => {
    try {
      await setOnNewDeviceInserted(v)
      await refreshHotplugStatus()
    } catch (e) {
      console.error('Failed to set on_new_device_inserted:', e)
    }
  }

  const handleDebugInject = async (kind: 'added' | 'removed' | 'default_changed') => {
    if (!hotplugStatus) return
    try {
      if (kind === 'removed') {
        const id = hotplugStatus.current_device_id ?? ''
        if (id) await debugInjectHotplug('removed', id)
      } else if (kind === 'added') {
        const dev = hotplugStatus.devices.find((d) => d.id !== hotplugStatus.current_device_id)
        if (dev) await debugInjectHotplug('added', dev.id)
      } else if (kind === 'default_changed') {
        const dev = hotplugStatus.devices.find((d) => d.is_default)
        if (dev) await debugInjectHotplug('default_changed', dev.id)
      }
      await refreshHotplugStatus()
    } catch (e) {
      console.error('Failed to inject hotplug event:', e)
    }
  }

  // ── Plugin limits ──────────────────────────────────────────
  // WASM 插件资源限制（内存/超时）卡片位于「扩展」页（ExtensionsPanel），
  // 与插件管理器同页；设置页不再重复展示，避免两处状态互相同步。

  // rAF-batched spectrum-settings + replayGain preamp scheduler.
  // Same reasoning as the EqPanel slider batching: these handlers are
  // attached to <input type="range"> sliders that fire 40-80 events/sec
  // during drag. Issuing an invoke for every pixel floods Tauri's
  // callback table and is the #1 trigger of
  // "[TAURI] Couldn't find callback id" on any reload. The rAF flush
  // below guarantees at most 1 invoke per frame per slider.
  const pendingSpectrumRef = useRef<{ smoothing?: number; decay?: number; fps?: number }>({})
  const spectrumRafRef = useRef<number | null>(null)
  const pendingRgPreampRef = useRef<number | null>(null)
  const rgPreampRafRef = useRef<number | null>(null)
  const flushPendingSpectrum = () => {
    spectrumRafRef.current = null
    const p = pendingSpectrumRef.current
    pendingSpectrumRef.current = {}
    // set_spectrum_settings takes all three params together
    setSpectrumSettings(
      p.smoothing ?? spectrumSmoothing,
      p.decay ?? spectrumDecay,
      p.fps ?? spectrumFps,
    ).catch(() => {})
  }
  const flushPendingRgPreamp = () => {
    rgPreampRafRef.current = null
    const v = pendingRgPreampRef.current
    pendingRgPreampRef.current = null
    if (v !== null) {
      invoke('set_replay_gain_preamp', { preampDb: v }).catch(() => {})
    }
  }

  // ── Spectrum handlers ──────────────────────────────────────
  const handleSpectrumSmoothing = (v: number) => {
    setSpectrumSmoothing(v)
    localStorage.setItem('phonon-spectrum-smoothing', String(v))
    onSpectrumSettingsChange({ smoothing: v, decay: spectrumDecay, fps: spectrumFps })
    pendingSpectrumRef.current.smoothing = v
    if (spectrumRafRef.current === null) {
      spectrumRafRef.current = requestAnimationFrame(flushPendingSpectrum)
    }
  }

  const handleSpectrumDecay = (v: number) => {
    setSpectrumDecay(v)
    localStorage.setItem('phonon-spectrum-decay', String(v))
    onSpectrumSettingsChange({ smoothing: spectrumSmoothing, decay: v, fps: spectrumFps })
    pendingSpectrumRef.current.decay = v
    if (spectrumRafRef.current === null) {
      spectrumRafRef.current = requestAnimationFrame(flushPendingSpectrum)
    }
  }

  const handleSpectrumFps = (v: number) => {
    setSpectrumFps(v)
    localStorage.setItem('phonon-spectrum-fps', String(v))
    onSpectrumSettingsChange({ smoothing: spectrumSmoothing, decay: spectrumDecay, fps: v })
    pendingSpectrumRef.current.fps = v
    if (spectrumRafRef.current === null) {
      spectrumRafRef.current = requestAnimationFrame(flushPendingSpectrum)
    }
  }

  const handleSpectrumColorScheme = (scheme: ColorScheme) => {
    setSpectrumColorScheme(scheme)
    localStorage.setItem('phonon-spectrum-colorscheme', scheme)
    onSpectrumColorSchemeChange(scheme, scheme === 'custom' ? customSpectrumColors : undefined)
    invoke('set_spectrum_colorscheme', { scheme }).catch(() => {})
    window.dispatchEvent(new CustomEvent('spectrum-colors-changed'))
  }

  const handleCustomSpectrumColor = (key: 'bottom' | 'top' | 'peak' | 'grid', value: string) => {
    const updated = { ...customSpectrumColors, [key]: value }
    setCustomSpectrumColors(updated)
    localStorage.setItem('phonon-spectrum-custom-colors', JSON.stringify(updated))
    if (spectrumColorScheme === 'custom') {
      onSpectrumColorSchemeChange('custom', updated)
    }
    window.dispatchEvent(new CustomEvent('spectrum-colors-changed'))
  }

  const resetSpectrumParams = () => {
    const defaultSmoothing = 0.35
    const defaultDecay = 0.90
    const defaultFps = 60
    setSpectrumSmoothing(defaultSmoothing)
    setSpectrumDecay(defaultDecay)
    setSpectrumFps(defaultFps)
    localStorage.setItem('phonon-spectrum-smoothing', String(defaultSmoothing))
    localStorage.setItem('phonon-spectrum-decay', String(defaultDecay))
    localStorage.setItem('phonon-spectrum-fps', String(defaultFps))
    onSpectrumSettingsChange({ smoothing: defaultSmoothing, decay: defaultDecay, fps: defaultFps })
    setSpectrumSettings(defaultSmoothing, defaultDecay, defaultFps).catch(() => {})
  }

  // ── Reset all settings ─────────────────────────────────────
  const handleResetSettings = () => {
    setModal({
      type: 'confirm',
      title: t('common.confirm'),
      message: t('confirm.resetSettings'),
      resolve: async (val) => {
        if (val === undefined) return
        try {
          await resetToDefaults()
          addToast(t('toast.settingsReset'))
          // Full reload is required: CSS variables, theme, and font-size
          // are scattered across DOM/localStorage and can't be reliably reset in-place.
          window.location.reload()
        } catch {
          addToast(t('toast.settingsResetFailed'), 'error')
        }
      },
    })
  }

  // ── Render ─────────────────────────────────────────────────
  if (loading) {
    return (
      <div className="settings-container" style={{ padding: 24 }}>
        <p style={{ color: 'var(--text-dim)', fontSize: 'var(--text-base)' }}>{t('common.loading')}</p>
      </div>
    )
  }

  return (
    <div className="settings-container" style={{ padding: '0 20px 20px', height: '100%', overflowY: 'auto', maxWidth: 860, margin: '0 auto' }}>
      {/* Tab bar */}
      <div className="settings-tabs" style={{
        display: 'flex', gap: 0, borderBottom: '1px solid var(--border)',
        marginBottom: 20, position: 'sticky', top: 0, zIndex: 10,
        background: 'var(--bg)', paddingTop: 8,
      }}>
        {settingsTabs.map((tab) => (
          <button
            key={tab.id}
            className={`settings-tab ${settingsTab === tab.id ? 'active' : ''}`}
            onClick={() => setSettingsTab(tab.id)}
            style={{
              padding: '10px 18px', fontSize: 'var(--text-base)', fontWeight: 500,
              border: 'none', background: 'none',
              color: settingsTab === tab.id ? 'var(--accent)' : 'var(--text-dim)',
              cursor: 'pointer',
              borderBottom: settingsTab === tab.id ? '2px solid var(--accent)' : '2px solid transparent',
              transition: 'color 0.15s, border-color 0.15s',
              whiteSpace: 'nowrap', flex: 1, textAlign: 'center',
            }}
          >
            {tab.label}
          </button>
        ))}
      </div>

      {/* General Tab */}
      {settingsTab === 'general' && (
        <div className="settings-tab-content">
          {/* Language */}
          <div className="card" style={{ marginBottom: 16 }}>
            <h3>{t('settings.language')}</h3>
            <p className="setting-desc">{t('settings.language.desc')}</p>
            <div className="setting-row" style={{ gap: 8, marginTop: 10 }}>
              {(['zh-CN', 'en'] as Lang[]).map((l) => (
                <button
                  key={l}
                  className={`btn ${language === l ? 'btn-primary' : 'btn-outline'} btn-sm`}
                  onClick={() => handleLanguage(l)}
                >
                  {t('settings.language.' + (l === 'zh-CN' ? 'zhCN' : 'en'))}
                </button>
              ))}
            </div>
          </div>

          {/* Startup Behavior */}
          <div className="card" style={{ marginBottom: 16 }}>
            <h3>{t('settings.startupBehavior')}</h3>
            <p className="setting-desc">{t('settings.startupBehavior.desc')}</p>
            <div className="setting-row" style={{ gap: 8, marginTop: 10 }}>
              {(['Remember', 'Clear'] as StartupBehavior[]).map((b) => (
                <button
                  key={b}
                  className={`btn ${startupBehavior === b ? 'btn-primary' : 'btn-outline'} btn-sm`}
                  onClick={() => handleStartupBehavior(b)}
                >
                  {t('settings.startupBehavior.' + (b === 'Remember' ? 'remember' : 'clear'))}
                </button>
              ))}
            </div>
          </div>

          {/* Close to Tray */}
          <div className="card" style={{ marginBottom: 16 }}>
            <h3>{t('settings.closeToTray')}</h3>
            <p className="setting-desc">{t('settings.closeToTray.desc')}</p>
            <div className="setting-row" style={{ gap: 8, marginTop: 10 }}>
              {[
                { value: true, label: t('settings.closeToTray.tray') },
                { value: false, label: t('settings.closeToTray.quit') },
              ].map((opt) => (
                <button
                  key={String(opt.value)}
                  className={`btn ${closeToTray === opt.value ? 'btn-primary' : 'btn-outline'} btn-sm`}
                  onClick={() => handleCloseToTray(opt.value)}
                >
                  {opt.label}
                </button>
              ))}
            </div>
          </div>

          {/* Lyrics API URL — §14 合规：白名单置灰策略 */}
          <div className="card" style={{ marginBottom: 16 }}>
            <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 4 }}>
              <h3 style={{ margin: 0 }}>{t('settings.lyricsApiUrl')}</h3>
              <label style={{ display: 'inline-flex', alignItems: 'center', gap: 6, cursor: 'pointer', fontSize: 'var(--text-sm)', color: 'var(--text-dim)' }}>
                <input
                  type="checkbox"
                  checked={lyricsApiEnabled}
                  onChange={(e) => {
                    const v = e.target.checked
                    setLyricsApiEnabled(v)
                    writeStorage('lyricsApiEnabled', v)
                  }}
                />
                {t('settings.lyricsApiUrl.enable')}
              </label>
            </div>
            <p className="setting-desc" style={{ marginTop: 4 }}>{t('settings.lyricsApiUrl.desc')}</p>
            {!lyricsApiEnabled && (
              <p style={{ margin: '6px 0 0', fontSize: 'var(--text-sm)', color: 'var(--accent)', lineHeight: 1.5 }}>
                {t('settings.lyricsApiUrl.whitelistHint')}
              </p>
            )}
            <div style={{ marginTop: 10, display: 'flex', gap: 8 }}>
              <input
                type="text"
                value={lyricsApiUrl}
                onChange={(e) => handleLyricsApiUrl(e.target.value)}
                placeholder="https://lrclib.net"
                disabled={!lyricsApiEnabled}
                style={{
                  flex: 1, padding: '6px 10px', fontSize: 'var(--text-sm)',
                  borderRadius: 4, border: '1px solid var(--border)',
                  background: lyricsApiEnabled ? 'var(--input-bg)' : 'var(--bg-dim)',
                  color: lyricsApiEnabled ? 'var(--text)' : 'var(--text-dim)',
                  opacity: lyricsApiEnabled ? 1 : 0.55,
                  cursor: lyricsApiEnabled ? 'text' : 'not-allowed',
                }}
              />
              <button
                className="btn btn-sm btn-outline"
                onClick={() => handleLyricsApiUrl('https://lrclib.net')}
                title={t('settings.lyricsApiUrl.reset')}
                disabled={!lyricsApiEnabled}
                style={
                  !lyricsApiEnabled
                    ? { opacity: 0.5, cursor: 'not-allowed' }
                    : undefined
                }
              >
                {t('settings.lyricsApiUrl.reset')}
              </button>
            </div>
          </div>

          {/* Global Shortcuts */}
          <div className="card" style={{ marginBottom: 16 }}>
            <h3>{t('settings.shortcuts')}</h3>
            <p className="setting-desc">{t('settings.shortcuts.desc')}</p>
            <div style={{ marginTop: 12, display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)', gap: 8 }}>
              {[
                { action: 'playPause', label: t('settings.shortcuts.playPause', '播放/暂停') },
                { action: 'stop', label: t('settings.shortcuts.stop', '停止') },
                { action: 'previous', label: t('settings.shortcuts.previous', '上一曲') },
                { action: 'next', label: t('settings.shortcuts.next', '下一曲') },
              ].map(({ action, label }) => {
                const binding = shortcuts.find((s) => s.action === action)
                const isRecording = recordingAction === action
                return (
                  <div
                    key={action}
                    onClick={() => handleShortcutRecord(action)}
                    style={{
                      padding: '8px 6px',
                      background: isRecording ? 'var(--accent)' : 'var(--card-bg)',
                      border: '1px solid ' + (isRecording ? 'var(--accent)' : 'var(--border)'),
                      borderRadius: 'var(--radius-sm)',
                      cursor: 'pointer',
                      display: 'flex',
                      flexDirection: 'column',
                      alignItems: 'center',
                      gap: 2,
                      position: 'relative',
                      transition: 'border-color 0.15s',
                    }}
                    onMouseEnter={(e) => { if (!isRecording) e.currentTarget.style.borderColor = 'var(--accent)' }}
                    onMouseLeave={(e) => { if (!isRecording) e.currentTarget.style.borderColor = 'var(--border)' }}
                  >
                    <span style={{ fontSize: 'var(--text-xs)', color: isRecording ? '#fff' : 'var(--text-dim)' }}>{label}</span>
                    <span style={{
                      fontSize: 'var(--text-xs)', fontFamily: 'var(--mono)',
                      color: isRecording ? '#fff' : (binding?.keys ? 'var(--accent)' : 'var(--text-dim)'),
                    }}>
                      {isRecording ? t('settings.shortcuts.pressEsc') : (binding?.keys || t('settings.shortcuts.notSet'))}
                    </span>
                    {binding && !isRecording && (
                      <button
                        onClick={(e) => { e.stopPropagation(); handleRemoveShortcut(action) }}
                        style={{
                          position: 'absolute', top: 2, right: 2, width: 12, height: 12, borderRadius: '50%',
                          background: 'transparent', border: 'none', color: 'var(--danger)', cursor: 'pointer',
                          fontSize: 'var(--text-xs)', lineHeight: 1, display: 'flex', alignItems: 'center', justifyContent: 'center',
                        }}
                      >×</button>
                    )}
                  </div>
                )
              })}
            </div>
          </div>

          {/* Hotplug Notifications */}
          <div className="card" style={{ marginBottom: 16 }}>
            <div className="setting-row" style={{ justifyContent: 'space-between' }}>
              <div>
                <span className="setting-label">{t('settings.hotplugNotifications')}</span>
                <p className="setting-desc" style={{ marginTop: 4 }}>{t('settings.hotplugNotifications.desc')}</p>
              </div>
              <div
                className={`toggle-switch ${hotplugNotifications ? 'on' : ''}`}
                onClick={handleHotplugNotifications}
                style={{ cursor: 'pointer' }}
              />
            </div>
          </div>

          {/* Debug Log */}
          <div className="card" style={{ marginBottom: 16 }}>
            <div className="setting-row" style={{ justifyContent: 'space-between' }}>
              <div>
                <span className="setting-label">{t('settings.debugLog.label')}</span>
                <p className="setting-desc" style={{ marginTop: 4 }}>{t('settings.debugLog.desc')}</p>
              </div>
              <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
                <button
                  className="btn btn-sm btn-outline"
                  onClick={async () => { try { await invoke('open_log_dir') } catch { /* ignore */ } }}
                  style={{ fontSize: 'var(--text-xs)' }}
                >
                  {t('settings.debugLog.openDir', '打开日志目录')}
                </button>
                <div
                  className={`toggle-switch ${debugLog ? 'on' : ''}`}
                  onClick={handleDebugLog}
                  style={{ cursor: 'pointer' }}
                />
              </div>
            </div>
          </div>

          {/* Reset */}
          <div className="card" style={{ marginBottom: 16 }}>
            <button className="btn btn-outline" onClick={handleResetSettings} style={{ color: 'var(--danger)', borderColor: 'var(--danger)' }}>
              {t('settings.resetButton')}
            </button>
          </div>

          {/* Exit Application */}
          <div className="card" style={{ marginBottom: 16 }}>
            <button
              className="btn btn-outline"
              onClick={() => getCurrentWindow().close()}
              style={{ color: 'var(--danger)', borderColor: 'var(--danger)' }}
            >
              {t('settings.exitApp')}
            </button>
          </div>
        </div>
      )}

      {/* Appearance Tab */}
      {settingsTab === 'appearance' && (
        <div className="settings-tab-content">
          {/* Theme Presets */}
          <div className="card" style={{ marginBottom: 16 }}>
            <h3>{t('settings.appearance.preset')}</h3>
            <p className="setting-desc">{t('settings.appearance.desc')}</p>
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(180px, 1fr))', gap: 10, marginTop: 10 }}>
              {/* ── Builtin presets ── */}
              {(['dark', 'light', 'aurora', 'warm', 'pro-blue', 'high-contrast'] as ThemePreset[]).map((preset) => {
                const c = THEME_PRESETS[preset]
                const isActive = themePreset === preset
                return (
                  <div
                    key={preset}
                    onClick={() => handleThemePreset(preset)}
                    className="theme-preset-card"
                    style={{
                      padding: '12px 14px', borderRadius: 'var(--radius)',
                      background: c.cardBg,
                      border: isActive ? '2px solid var(--accent)' : `1px solid ${c.border}`,
                      cursor: 'pointer',
                      transition: 'border-color 0.15s, transform 0.1s',
                      display: 'flex', flexDirection: 'column', gap: 10,
                    }}
                    onMouseDown={(e) => { e.currentTarget.style.transform = 'scale(0.97)' }}
                    onMouseUp={(e) => { e.currentTarget.style.transform = 'scale(1)' }}
                    onMouseLeave={(e) => { e.currentTarget.style.transform = 'scale(1)' }}
                  >
                    <div style={{ display: 'flex', gap: 5, alignItems: 'center' }}>
                      <div style={{ width: 12, height: 12, borderRadius: '50%', background: c.accent }} />
                      <div style={{ width: 12, height: 12, borderRadius: '50%', background: c.accent2 || c.accent }} />
                      <div style={{ width: 12, height: 12, borderRadius: '50%', background: c.bg, border: `1px solid ${c.border}` }} />
                      <div style={{ width: 12, height: 12, borderRadius: '50%', background: c.border }} />
                    </div>
                    <div style={{ fontSize: 'var(--text-sm)', fontWeight: 600, color: c.textBright }}>
                      {t('settings.appearance.preset.' + preset, preset)}
                    </div>
                    <div style={{
                      height: 3, borderRadius: 2,
                      background: `linear-gradient(90deg, ${c.accent} 0%, ${c.accent2 || c.accent} 50%, ${c.border} 100%)`,
                      marginTop: 'auto',
                    }} />
                  </div>
                )
              })}
              {/* ── Custom presets ── */}
              {Object.entries(customPresets).map(([name, c]) => {
                const isActive = themePreset === name
                return (
                  <div
                    key={name}
                    className="theme-preset-card custom-preset-card"
                    style={{
                      position: 'relative', padding: '12px 14px', borderRadius: 'var(--radius)',
                      background: c.cardBg,
                      border: isActive ? '2px solid var(--accent)' : `1px solid ${c.border}`,
                      cursor: 'pointer',
                      transition: 'border-color 0.15s, transform 0.1s',
                      display: 'flex', flexDirection: 'column', gap: 10,
                      minHeight: 78,
                    }}
                    onClick={() => {
                      if (renamingPreset === name) return
                      const a2 = c.accent2 || c.accent
                      setThemePreset(name as ThemePreset)
                      localStorage.setItem('phonon-theme-preset', name)
                      setAccentColor(c.accent); setAccent2Color(a2); setBgColor(c.bg); setCardBgColor(c.cardBg); setBorderColor(c.border)
                      setAccentOpacity(c.accentOpacity); setAccent2Opacity(c.accent2Opacity ?? c.accentOpacity); setBgOpacity(c.bgOpacity); setCardBgOpacity(c.cardBgOpacity); setBorderOpacity(c.borderOpacity)
                      setTextColor(c.text); setTextDimColor(c.textDim); setTextBrightColor(c.textBright); setTextDimOpacity(c.textDimOpacity)
                      setPlaceholderColor(c.placeholder); setPlaceholderOpacity(c.placeholderOpacity)
                      localStorage.setItem('phonon-accent-color', c.accent)
                      localStorage.setItem('phonon-accent2-color', a2)
                      localStorage.setItem('phonon-bg-color', c.bg)
                      localStorage.setItem('phonon-card-bg-color', c.cardBg)
                      localStorage.setItem('phonon-border-color', c.border)
                      localStorage.setItem('phonon-accent-opacity', String(c.accentOpacity))
                      localStorage.setItem('phonon-accent2-opacity', String(c.accent2Opacity ?? c.accentOpacity))
                      localStorage.setItem('phonon-bg-opacity', String(c.bgOpacity))
                      localStorage.setItem('phonon-cardbg-opacity', String(c.cardBgOpacity))
                      localStorage.setItem('phonon-border-opacity', String(c.borderOpacity))
                      localStorage.setItem('phonon-text-color', c.text)
                      localStorage.setItem('phonon-text-bright-color', c.textBright)
                      localStorage.setItem('phonon-text-dim-color', c.textDim)
                      localStorage.setItem('phonon-text-dim-opacity', String(c.textDimOpacity))
                      localStorage.setItem('phonon-placeholder-color', c.placeholder)
                      localStorage.setItem('phonon-placeholder-opacity', String(c.placeholderOpacity))
                      applyColorOverrides({ accent: c.accent, accent2: a2, accentOpacity: c.accentOpacity, accent2Opacity: c.accent2Opacity ?? c.accentOpacity, bg: c.bg, bgOpacity: c.bgOpacity, cardBg: c.cardBg, cardBgOpacity: c.cardBgOpacity, border: c.border, borderOpacity: c.borderOpacity })
                      applyTextColors(c.text, c.textBright, c.textDim, c.textDimOpacity, c.placeholder, c.placeholderOpacity)
                    }}
                    onMouseDown={(e) => { if (renamingPreset !== name) e.currentTarget.style.transform = 'scale(0.97)' }}
                    onMouseUp={(e) => { e.currentTarget.style.transform = 'scale(1)' }}
                    onMouseLeave={(e) => { e.currentTarget.style.transform = 'scale(1)' }}
                  >
                    <div style={{ display: 'flex', gap: 5, alignItems: 'center' }}>
                      <div style={{ width: 12, height: 12, borderRadius: '50%', background: c.accent }} />
                      <div style={{ width: 12, height: 12, borderRadius: '50%', background: c.accent2 || c.accent }} />
                      <div style={{ width: 12, height: 12, borderRadius: '50%', background: c.bg, border: `1px solid ${c.border}` }} />
                      <div style={{ width: 12, height: 12, borderRadius: '50%', background: c.border }} />
                    </div>
                    {renamingPreset === name ? (
                      <input
                        type="text"
                        defaultValue={name}
                        autoFocus
                        onClick={(e) => e.stopPropagation()}
                        onBlur={(e) => {
                          const newName = e.target.value.trim()
                          if (newName) renamePreset(name, newName)
                          setRenamingPreset(null)
                        }}
                        onKeyDown={(e) => {
                          if (e.key === 'Enter') {
                            const newName = (e.target as HTMLInputElement).value.trim()
                            if (newName) renamePreset(name, newName)
                            setRenamingPreset(null)
                          } else if (e.key === 'Escape') {
                            setRenamingPreset(null)
                          }
                        }}
                        style={{
                          fontSize: 'var(--text-sm)', fontWeight: 600, color: c.textBright,
                          width: '100%',
                          padding: '2px 4px', background: 'var(--bg)', border: '1px solid var(--accent)',
                          borderRadius: 4, outline: 'none', boxSizing: 'border-box',
                        }}
                      />
                    ) : (
                      <div style={{ fontSize: 'var(--text-sm)', fontWeight: 600, color: c.textBright, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                        {name}
                      </div>
                    )}
                    {/* Action buttons — hover only */}
                    <div className="preset-actions" style={{
                      position: 'absolute', top: 4, right: 6, display: 'flex', gap: 2,
                      opacity: 0, transition: 'opacity 0.15s',
                    }}>
                      <button
                        onClick={(e) => { e.stopPropagation(); setRenamingPreset(name) }}
                        title={t('settings.appearance.preset.rename', '重命名')}
                        style={{ width: 16, height: 16, borderRadius: '50%', background: 'rgba(0,0,0,0.5)', color: '#fff', border: 'none', cursor: 'pointer', fontSize: 'var(--text-xs)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}
                      >✎</button>
                      <button
                        onClick={(e) => { e.stopPropagation(); exportPreset(name) }}
                        title={t('settings.appearance.preset.export', '导出预设')}
                        style={{ width: 16, height: 16, borderRadius: '50%', background: 'rgba(0,0,0,0.5)', color: '#fff', border: 'none', cursor: 'pointer', fontSize: 'var(--text-xs)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}
                      >⬇</button>
                      <button
                        onClick={(e) => { e.stopPropagation(); deleteCustomPreset(name) }}
                        style={{ width: 16, height: 16, borderRadius: '50%', background: 'rgba(220,38,38,0.7)', color: '#fff', border: 'none', cursor: 'pointer', fontSize: 'var(--text-xs)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}
                      >×</button>
                    </div>
                    <div style={{
                      height: 3, borderRadius: 2,
                      background: `linear-gradient(90deg, ${c.accent} 0%, ${c.accent2 || c.accent} 50%, ${c.border} 100%)`,
                      marginTop: 'auto',
                    }} />
                  </div>
                )
              })}
            </div>
          </div>

          {/* Custom & Text Colors */}
          <div className="card" style={{ marginBottom: 16 }}>
            <h3>{t('settings.appearance.customColors')}</h3>
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(220px, 1fr))', gap: 12, marginTop: 12 }}>
              {[
                { key: 'accent', label: t('settings.appearance.accentColor'), state: accentColor, opacity: accentOpacity, opacityKey: 'accent', opacityLabel: t('settings.appearance.accentOpacity', '强调色透明度') },
                { key: 'accent2', label: t('settings.appearance.accent2Color', '渐变副色 (Vis 按钮)'), state: accent2Color, opacity: accent2Opacity, opacityKey: 'accent2', opacityLabel: t('settings.appearance.accent2Opacity', '副色透明度') },
                { key: 'bg', label: t('settings.appearance.bgColor'), state: bgColor, opacity: bgOpacity, opacityKey: 'bg', opacityLabel: t('settings.appearance.bgOpacity') },
                { key: 'cardBg', label: t('settings.appearance.cardBgColor'), state: cardBgColor, opacity: cardBgOpacity, opacityKey: 'cardBg', opacityLabel: t('settings.appearance.cardBgOpacity', '卡片透明度') },
                { key: 'border', label: t('settings.appearance.borderColor'), state: borderColor, opacity: borderOpacity, opacityKey: 'border', opacityLabel: t('settings.appearance.borderOpacity', '边框透明度') },
              ].map((item) => (
                <div key={item.key} style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
                  <span style={{ fontSize: 'var(--text-sm)', color: 'var(--text-dim)' }}>{item.label}</span>
                  <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
                    <input
                      type="color"
                      value={item.state}
                      onChange={(e) => handleColorChange(item.key, e.target.value)}
                      style={{ width: 32, height: 28, border: 'none', borderRadius: 4, cursor: 'pointer', padding: 0, background: 'none' }}
                    />
                    <input
                      type="text"
                      value={item.state}
                      onChange={(e) => handleColorChange(item.key, e.target.value)}
                      style={{
                        flex: 1, width: 70, fontSize: 'var(--text-xs)', fontFamily: 'var(--mono)',
                        padding: '4px 8px', background: 'var(--bg)', color: 'var(--text)',
                        border: '1px solid var(--border)', borderRadius: 'var(--radius-sm)',
                        outline: 'none',
                      }}
                    />
                  </div>
                  <div style={{ display: 'flex', gap: 6, alignItems: 'center' }}>
                    <span style={{ fontSize: 'var(--text-xs)', color: 'var(--text-dim)', minWidth: 36 }}>{item.opacityLabel}</span>
                    <input
                      type="range"
                      min="0.1" max="1" step="0.05"
                      value={item.opacity}
                      onChange={(e) => handleOpacityChange(item.opacityKey, parseFloat(e.target.value))}
                      className="eq-gain-slider"
                      style={{ flex: 1, height: 4 }}
                    />
                    <span style={{ fontSize: 'var(--text-xs)', fontFamily: 'var(--mono)', color: 'var(--text-dim)', minWidth: 28, textAlign: 'right' }}>
                      {item.opacity.toFixed(2)}
                    </span>
                  </div>
                </div>
              ))}
            </div>
            <hr style={{ border: 'none', borderTop: '1px solid var(--border)', margin: '16px 0' }} />
            <h3 style={{ marginBottom: 10 }}>{t('settings.appearance.textColors')}</h3>
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(220px, 1fr))', gap: 12 }}>
              {[
                { key: 'text', label: t('settings.appearance.textColor'), state: textColor },
                { key: 'textBright', label: t('settings.appearance.textBrightColor'), state: textBrightColor },
                { key: 'textDim', label: t('settings.appearance.textDimColor'), state: textDimColor, opacity: textDimOpacity, opacityKey: 'textDim', opacityLabel: t('settings.appearance.textDimOpacity', '辅助文字透明度') },
                { key: 'placeholder', label: t('settings.appearance.placeholderColor', '占位符颜色'), state: placeholderColor, opacity: placeholderOpacity, opacityKey: 'placeholder', opacityLabel: t('settings.appearance.placeholderOpacity', '占位符透明度') },
              ].map((item) => (
                <div key={item.key} style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
                  <span style={{ fontSize: 'var(--text-sm)', color: 'var(--text-dim)' }}>{item.label}</span>
                  <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
                    <input
                      type="color"
                      value={item.state}
                      onChange={(e) => handleColorChange(item.key, e.target.value)}
                      style={{ width: 32, height: 28, border: 'none', borderRadius: 4, cursor: 'pointer', padding: 0, background: 'none' }}
                    />
                    <input
                      type="text"
                      value={item.state}
                      onChange={(e) => handleColorChange(item.key, e.target.value)}
                      style={{
                        flex: 1, width: 70, fontSize: 'var(--text-xs)', fontFamily: 'var(--mono)',
                        padding: '4px 8px', background: 'var(--bg)', color: 'var(--text)',
                        border: '1px solid var(--border)', borderRadius: 'var(--radius-sm)',
                        outline: 'none',
                      }}
                    />
                  </div>
                  {item.opacityKey && (
                    <div style={{ display: 'flex', gap: 6, alignItems: 'center' }}>
                      <span style={{ fontSize: 'var(--text-xs)', color: 'var(--text-dim)', minWidth: 36 }}>{item.opacityLabel}</span>
                      <input
                        type="range"
                        min="0.1" max="1" step="0.05"
                        value={item.opacity!}
                        onChange={(e) => handleOpacityChange(item.opacityKey!, parseFloat(e.target.value))}
                        className="eq-gain-slider"
                        style={{ flex: 1, height: 4 }}
                      />
                      <span style={{ fontSize: 'var(--text-xs)', fontFamily: 'var(--mono)', color: 'var(--text-dim)', minWidth: 28, textAlign: 'right' }}>
                        {(item.opacity ?? 1).toFixed(2)}
                      </span>
                    </div>
                  )}
                </div>
              ))}
            </div>
            <hr style={{ border: 'none', borderTop: '1px solid var(--border)', margin: '16px 0' }} />
            <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
              <input
                ref={presetNameInputRef}
                type="text"
                placeholder={t('settings.appearance.preset.namePlaceholder', '预设名称')}
                style={{ flex: 1, maxWidth: 200, fontSize: 'var(--text-sm)', padding: '6px 10px', background: 'var(--bg)', color: 'var(--text)', border: '1px solid var(--border)', borderRadius: 'var(--radius-sm)' }}
              />
              <button
                className="btn btn-primary btn-sm"
                onClick={() => {
                  const name = presetNameInputRef.current?.value?.trim()
                  if (name) {
                    if (saveCurrentColorsAsPreset(name)) {
                      presetNameInputRef.current!.value = ''
                      addToast(t('settings.appearance.preset.saved', '预设已保存'))
                    } else {
                      addToast(t('settings.appearance.preset.nameExists', '预设名称已存在，请使用其他名称'), 'warn')
                    }
                  }
                }}
              >
                {t('settings.appearance.preset.saveAs', '保存为预设')}
              </button>
            </div>
          </div>

          {/* Font */}
          <div className="card" style={{ marginBottom: 16 }}>
            <h3>{t('settings.font')}</h3>
            <p className="setting-desc">{t('settings.font.desc')}</p>
            <div style={{ marginTop: 12, display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 20 }}>
              {/* Left: Font Family */}
              <div>
                <div style={{ fontSize: 'var(--text-sm)', color: 'var(--text-dim)', marginBottom: 6 }}>{t('settings.appearance.fontFamily')}</div>
                <select
                  value={fontFamily}
                  onChange={(e) => handleFontFamilyChange(e.target.value)}
                  style={{
                    width: '100%', fontSize: 'var(--text-sm)',
                    padding: '6px 10px', background: 'var(--bg)', color: 'var(--text)',
                    border: '1px solid var(--border)', borderRadius: 'var(--radius-sm)',
                    outline: 'none', cursor: 'pointer',
                  }}
                >
                  <option value="">{t('settings.font.default', '默认')}</option>
                  <option value="system-ui">{t('settings.font.system-ui', '系统UI')}</option>
                  <option value="Inter">{t('settings.font.inter', 'Inter')}</option>
                  <option value="sans-serif">{t('settings.font.sans-serif', '无衬线体')}</option>
                  <option value="serif">{t('settings.font.serif', '衬线体')}</option>
                  <option value="monospace">{t('settings.font.monospace', '等宽字体')}</option>
                  <option value="Arial">{t('settings.font.arial', 'Arial')}</option>
                  <option value="Helvetica">{t('settings.font.helvetica', 'Helvetica')}</option>
                  <option value="Georgia">{t('settings.font.georgia', 'Georgia')}</option>
                  <option value="Times New Roman">{t('settings.font.times-new-roman', 'Times New Roman')}</option>
                  <option value="Courier New">{t('settings.font.courier-new', 'Courier New')}</option>
                  <option value="Verdana">{t('settings.font.verdana', 'Verdana')}</option>
                  <option value="Tahoma">{t('settings.font.tahoma', 'Tahoma')}</option>
                  <option value="Trebuchet MS">{t('settings.font.trebuchet-ms', 'Trebuchet MS')}</option>
                  <option value="'Microsoft YaHei', '微软雅黑'">{t('settings.font.microsoft-yahei', '微软雅黑')}</option>
                  <option value="'PingFang SC', '苹方'">{t('settings.font.pingfang', '苹方')}</option>
                  <option value="'Noto Sans SC', '思源黑体'">{t('settings.font.noto-sans-sc', '思源黑体')}</option>
                  <option value="'Noto Serif SC', '思源宋体'">{t('settings.font.noto-serif-sc', '思源宋体')}</option>
                  <option value="'SimHei', '黑体'">{t('settings.font.simhei', '黑体')}</option>
                  <option value="'SimSun', '宋体'">{t('settings.font.simsun', '宋体')}</option>
                  <option value="'KaiTi', '楷体'">{t('settings.font.kaiti', '楷体')}</option>
                  <option value="'FangSong', '仿宋'">{t('settings.font.fangsong', '仿宋')}</option>
                </select>
              </div>
              {/* Right: Font Size */}
              <div>
                <div style={{ fontSize: 'var(--text-sm)', color: 'var(--text-dim)', marginBottom: 6 }}>{t('settings.appearance.fontSize')}</div>
                <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                  <input
                    type="range"
                    min="12" max="20" step="0.5"
                    value={fontSize}
                    onChange={(e) => handleFontSizeChange(parseFloat(e.target.value))}
                    className="eq-gain-slider"
                    style={{ flex: 1 }}
                  />
                  <span style={{ fontSize: 'var(--text-sm)', fontFamily: 'var(--mono)', color: 'var(--text)', minWidth: 36, textAlign: 'right' }}>
                    {fontSize}px
                  </span>
                </div>
              </div>
            </div>
          </div>

          {/* Desktop Lyrics */}
          <div className="card" style={{ marginBottom: 16 }}>
            <div className="setting-row" style={{ justifyContent: 'space-between', marginBottom: 10 }}>
              <h3 style={{ margin: 0 }}>{t('settings.desktopLyrics')}</h3>
              <button className="btn btn-outline btn-sm" onClick={handleDesktopLyricsReset}>
                {t('settings.desktopLyrics.reset')}
              </button>
            </div>
            <p className="setting-desc">{t('settings.desktopLyrics.desc')}</p>
            <div style={{ marginTop: 12, display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(280px, 1fr))', gap: '14px 20px' }}>
              <div className="setting-row" style={{ gap: 12, alignItems: 'center' }}>
                <span style={{ fontSize: 'var(--text-sm)', color: 'var(--text-dim)', minWidth: 80 }}>{t('settings.desktopLyrics.fontSize')}</span>
                <input
                  type="range"
                  min="12" max="60" step="1"
                  value={desktopLyricsFontSize}
                  onChange={(e) => handleDesktopLyricsFontSize(parseInt(e.target.value))}
                  className="eq-gain-slider"
                  style={{ flex: 1 }}
                />
                <span style={{ fontSize: 'var(--text-sm)', fontFamily: 'var(--mono)', color: 'var(--text)', minWidth: 32, textAlign: 'right' }}>
                  {desktopLyricsFontSize}px
                </span>
              </div>
              <div className="setting-row" style={{ gap: 12, alignItems: 'center' }}>
                <span style={{ fontSize: 'var(--text-sm)', color: 'var(--text-dim)', minWidth: 80 }}>{t('settings.desktopLyrics.opacity')}</span>
                <input
                  type="range"
                  min="0.1" max="1" step="0.05"
                  value={desktopLyricsOpacity}
                  onChange={(e) => handleDesktopLyricsOpacity(parseFloat(e.target.value))}
                  className="eq-gain-slider"
                  style={{ flex: 1 }}
                />
                <span style={{ fontSize: 'var(--text-sm)', fontFamily: 'var(--mono)', color: 'var(--text)', minWidth: 32, textAlign: 'right' }}>
                  {Math.round(desktopLyricsOpacity * 100)}%
                </span>
              </div>
              <div className="setting-row" style={{ gap: 12, alignItems: 'center', flexWrap: 'wrap' }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 12, flex: '1 1 180px', minWidth: 160 }}>
                  <span style={{ fontSize: 'var(--text-sm)', color: 'var(--text-dim)', minWidth: 80 }}>{t('settings.desktopLyrics.playedColor')}</span>
                  <input
                    type="color"
                    value={desktopLyricsPlayedColor}
                    onChange={(e) => handleDesktopLyricsPlayedColor(e.target.value)}
                    style={{
                      width: 44, height: 28, padding: 0, border: '1px solid var(--border)',
                      borderRadius: 'var(--radius-sm)', background: 'transparent', cursor: 'pointer',
                    }}
                  />
                </div>
                <div style={{ display: 'flex', alignItems: 'center', gap: 12, flex: '1 1 180px', minWidth: 160 }}>
                  <span style={{ fontSize: 'var(--text-sm)', color: 'var(--text-dim)', minWidth: 80 }}>{t('settings.desktopLyrics.unplayedColor')}</span>
                  <input
                    type="color"
                    value={desktopLyricsUnplayedColor}
                    onChange={(e) => handleDesktopLyricsUnplayedColor(e.target.value)}
                    style={{
                      width: 44, height: 28, padding: 0, border: '1px solid var(--border)',
                      borderRadius: 'var(--radius-sm)', background: 'transparent', cursor: 'pointer',
                    }}
                  />
                </div>
              </div>
              {/* Row 3: Background Mode */}
              <div className="setting-row" style={{ gap: 12, alignItems: 'center' }}>
                <span style={{ fontSize: 'var(--text-sm)', color: 'var(--text-dim)', minWidth: 88 }}>{t('settings.desktopLyrics.bgMode')}</span>
                <select
                  value={desktopLyricsBgMode}
                  onChange={(e) => handleDesktopLyricsBgMode(e.target.value)}
                  style={{
                    flex: 1, padding: '4px 8px', borderRadius: 'var(--radius-sm)',
                    border: '1px solid var(--border)', background: 'var(--card-bg)',
                    color: 'var(--text)', fontSize: 'var(--text-sm)', cursor: 'pointer',
                  }}
                >
                  <option value="hidden">{t('settings.desktopLyrics.bgModeHidden')}</option>
                  <option value="image">{t('settings.desktopLyrics.bgModeImage')}</option>
                  <option value="video">{t('settings.desktopLyrics.bgModeVideo')}</option>
                </select>
              </div>

              {/* Row 4: Background File (immediately after Background Mode) */}
              {(desktopLyricsBgMode === 'image' || desktopLyricsBgMode === 'video') && (
                <div className="setting-row" style={{ gap: 8, alignItems: 'center' }}>
                  <span style={{ fontSize: 'var(--text-sm)', color: 'var(--text-dim)', minWidth: 88 }}>{t('settings.desktopLyrics.bgFile')}</span>
                  {desktopLyricsBgMode === 'image' ? (
                    <>
                      <input
                        type="file"
                        accept="image/*"
                        onChange={handleDesktopLyricsBgUpload}
                        style={{ display: 'none' }}
                        id="desktop-lyrics-bg-upload"
                      />
                      <label
                        htmlFor="desktop-lyrics-bg-upload"
                        className="btn-ghost"
                        style={{ fontSize: 'var(--text-sm)', cursor: 'pointer', whiteSpace: 'nowrap' }}
                      >
                        {desktopLyricsBgPath ? t('settings.desktopLyrics.bgFileChange') : t('settings.desktopLyrics.bgFileSelect')}
                      </label>
                    </>
                  ) : (
                    <button className="btn-ghost" onClick={handleDesktopLyricsBgFile} style={{ fontSize: 'var(--text-sm)', cursor: 'pointer', whiteSpace: 'nowrap' }}>
                      {desktopLyricsBgPath ? t('settings.desktopLyrics.bgFileChange') : t('settings.desktopLyrics.bgFileSelect')}
                    </button>
                  )}
                  {desktopLyricsBgPath && (
                    <button className="btn-ghost" onClick={() => handleDesktopLyricsBgPath('')} style={{ fontSize: 'var(--text-sm)' }}>
                      {t('settings.desktopLyrics.bgFileClear')}
                    </button>
                  )}
                </div>
              )}

              {/* Row 5: Background Opacity (grouped with background settings) */}
              {(desktopLyricsBgMode === 'image' || desktopLyricsBgMode === 'video') && (
                <div className="setting-row" style={{ gap: 12, alignItems: 'center' }}>
                  <span style={{ fontSize: 'var(--text-sm)', color: 'var(--text-dim)', minWidth: 88 }}>
                    {t('settings.desktopLyrics.bgOpacityCustom')}
                  </span>
                  <input
                    type="range"
                    min="0" max="1" step="0.05"
                    value={desktopLyricsBgOpacity}
                    onChange={(e) => handleDesktopLyricsBgOpacity(parseFloat(e.target.value))}
                    className="eq-gain-slider"
                    style={{ flex: 1 }}
                  />
                  <span style={{ fontSize: 'var(--text-sm)', fontFamily: 'var(--mono)', color: 'var(--text)', minWidth: 32, textAlign: 'right' }}>
                    {Math.round(desktopLyricsBgOpacity * 100)}%
                  </span>
                </div>
              )}

              {/* Row 6: Line Mode */}
              <div className="setting-row" style={{ gap: 12, alignItems: 'center' }}>
                <span style={{ fontSize: 'var(--text-sm)', color: 'var(--text-dim)', minWidth: 88 }}>{t('settings.desktopLyrics.lineMode')}</span>
                <select
                  value={desktopLyricsLineMode}
                  onChange={(e) => setDesktopLyricsLineMode(e.target.value)}
                  style={{
                    flex: 1, padding: '4px 8px', borderRadius: 'var(--radius-sm)',
                    border: '1px solid var(--border)', background: 'var(--card-bg)',
                    color: 'var(--text)', fontSize: 'var(--text-sm)', cursor: 'pointer',
                  }}
                >
                  <option value="single">{t('settings.desktopLyrics.lineModeSingle')}</option>
                  <option value="double">{t('settings.desktopLyrics.lineModeDouble')}</option>
                </select>
              </div>

              {/* Row 7: Double-line Style (only when double mode) */}
              {desktopLyricsLineMode === 'double' && (
                <div className="setting-row" style={{ gap: 12, alignItems: 'center' }}>
                  <span style={{ fontSize: 'var(--text-sm)', color: 'var(--text-dim)', minWidth: 88 }}>双行样式</span>
                  <select
                    value={desktopLyricsLineStyle}
                    onChange={(e) => setDesktopLyricsLineStyle(e.target.value)}
                    style={{
                      flex: 1, padding: '4px 8px', borderRadius: 'var(--radius-sm)',
                      border: '1px solid var(--border)', background: 'var(--card-bg)',
                      color: 'var(--text)', fontSize: 'var(--text-sm)', cursor: 'pointer',
                    }}
                  >
                    <option value="scroll">滚动切换</option>
                    <option value="centered">居中</option>
                    <option value="split">左右分离</option>
                  </select>
                </div>
              )}
              <div className="setting-row" style={{ gap: 12, alignItems: 'center' }}>
                <span style={{ fontSize: 'var(--text-sm)', color: 'var(--text-dim)', minWidth: 88 }}>{t('settings.desktopLyrics.alwaysOnTop')}</span>
                <button
                  className={`toggle-btn ${desktopLyricsAlwaysOnTop ? 'on' : ''}`}
                  onClick={() => handleDesktopLyricsAlwaysOnTop(!desktopLyricsAlwaysOnTop)}
                  style={{
                    padding: '2px 10px', borderRadius: 12,
                    fontSize: 'var(--text-xs)', fontWeight: 600, border: 'none', cursor: 'pointer',
                    background: desktopLyricsAlwaysOnTop ? 'var(--accent)' : 'var(--bg-hover)',
                    color: desktopLyricsAlwaysOnTop ? '#fff' : 'var(--text-dim)',
                  }}
                >
                  {desktopLyricsAlwaysOnTop ? t('common.on') : t('common.off')}
                </button>
              </div>
            </div>
          </div>

          {/* Background Image */}
          <div className="card" style={{ marginBottom: 16 }}>
            <div className="setting-row" style={{ justifyContent: 'space-between', marginBottom: 6 }}>
              <h3 style={{ margin: 0 }}>{t('settings.appearance.bgImage')}</h3>
              <button
                className={`toggle-btn ${bgEnabled ? 'on' : ''}`}
                onClick={() => handleBgEnabled(!bgEnabled)}
                style={{
                  padding: '2px 10px', borderRadius: 12,
                  fontSize: 'var(--text-xs)', fontWeight: 600, border: 'none', cursor: 'pointer',
                  background: bgEnabled ? 'var(--accent)' : 'var(--bg-hover)',
                  color: bgEnabled ? '#fff' : 'var(--text-dim)',
                }}
              >
                {bgEnabled ? t('common.on') : t('common.off')}
              </button>
            </div>
            <div style={{ marginTop: 12 }}>
              <div className="setting-row" style={{ gap: 8, marginBottom: 8 }}>
                <label className="btn-ghost" style={{ cursor: 'pointer', whiteSpace: 'nowrap' }}>
                  {t('settings.appearance.bgImageUpload')}
                  <input
                    type="file"
                    accept="image/*"
                    style={{ display: 'none' }}
                    onChange={handleBgImageUpload}
                  />
                </label>
                {bgImages.length > 0 && (
                  <button
                    className={`btn-ghost ${bgDeleteMode ? 'btn-danger' : ''}`}
                    onClick={() => setBgDeleteMode(!bgDeleteMode)}
                    style={bgDeleteMode ? { color: '#f87171', borderColor: 'rgba(248,113,113,0.3)' } : {}}
                  >
                    {bgDeleteMode ? t('common.done') : t('common.delete')}
                  </button>
                )}
              </div>
              {bgImages.length > 0 && (
                <div style={{ display: 'flex', flexWrap: 'wrap', gap: 8, marginTop: 8 }}>
                  {bgImages.map((img, i) => (
                    <div
                      key={i}
                      style={{
                        position: 'relative', width: 80, height: 50,
                        borderRadius: 6, overflow: 'hidden', cursor: 'pointer',
                        border: i === bgImageIndex ? '2px solid var(--accent)' : '2px solid var(--border)',
                        transition: 'border-color 0.15s ease',
                      }}
                      onClick={() => {
                        if (bgDeleteMode) {
                          handleDeleteBgImage(i)
                        } else {
                          handleSelectBgImage(i)
                        }
                      }}
                    >
                      <img src={img} style={{ width: '100%', height: '100%', objectFit: 'cover' }} />
                      {bgDeleteMode && (
                        <button
                          onClick={(e) => { e.stopPropagation(); handleDeleteBgImage(i) }}
                          style={{
                            position: 'absolute', top: 2, right: 2,
                            width: 18, height: 18, borderRadius: '50%',
                            background: 'rgba(220,38,38,0.9)', color: '#fff',
                            border: 'none', cursor: 'pointer', fontSize: 'var(--text-xs)',
                            display: 'flex', alignItems: 'center', justifyContent: 'center',
                            lineHeight: 1, padding: 0,
                          }}
                        >
                          ✕
                        </button>
                      )}
                    </div>
                  ))}
                </div>
              )}
            </div>
          </div>

          {/* Album Art */}
          <div className="card" style={{ marginBottom: 16 }}>
            <h3>{t('settings.appearance.albumArt')}</h3>
            <div style={{ marginTop: 12 }}>
              <div className="setting-row" style={{ gap: 12, alignItems: 'center', marginBottom: 10 }}>
                <span style={{ fontSize: 'var(--text-sm)', color: 'var(--text-dim)', minWidth: 80 }}>{t('settings.appearance.albumArtRadius')}</span>
                <input
                  type="range"
                  min="0" max="30" step="1"
                  value={albumArtRadius}
                  onChange={(e) => handleAlbumArtRadius(parseInt(e.target.value))}
                  className="eq-gain-slider"
                  style={{ flex: 1 }}
                />
                <span style={{ fontSize: 'var(--text-sm)', fontFamily: 'var(--mono)', color: 'var(--text)', minWidth: 28, textAlign: 'right' }}>
                  {albumArtRadius}px
                </span>
              </div>
              <div className="setting-row" style={{ justifyContent: 'space-between', alignItems: 'center' }}>
                <div>
                  <span className="setting-label">{t('settings.appearance.albumArtBlur')}</span>
                  <p className="setting-desc" style={{ marginTop: 4 }}>{t('settings.appearance.albumArtBlur.desc')}</p>
                </div>
                <div
                  className={`toggle-switch ${albumArtBlur ? 'on' : ''}`}
                  onClick={() => handleAlbumArtBlur(!albumArtBlur)}
                  style={{ cursor: 'pointer' }}
                />
              </div>
            </div>
          </div>

          {/* Spectrum Settings */}
          <div className="card" style={{ marginBottom: 16 }}>
            <h3>{t('settings.spectrum.colorScheme')}</h3>
            <p className="setting-desc">{t('settings.spectrum.desc')}</p>
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(110px, 1fr))', gap: 10, marginTop: 12 }}>
              {(['aurora', 'warm', 'cool', 'mono', 'custom'] as ColorScheme[]).map((scheme) => {
                const isActive = spectrumColorScheme === scheme
                const gradientMap: Record<string, string> = {
                  aurora: 'linear-gradient(135deg, #003153 0%, #00BFFF 100%)',
                  warm: 'linear-gradient(135deg, #4a1500 0%, #FF5500 100%)',
                  cool: 'linear-gradient(135deg, #1a0050 0%, #7744FF 100%)',
                  mono: 'linear-gradient(135deg, #1a1a1a 0%, #CCCCCC 100%)',
                  custom: 'linear-gradient(135deg, #06b6d4 0%, #8b5cf6 50%, #ec4899 100%)',
                }
                return (
                  <div
                    key={scheme}
                    onClick={() => handleSpectrumColorScheme(scheme)}
                    style={{
                      borderRadius: 'var(--radius)',
                      padding: 2,
                      cursor: 'pointer',
                      border: isActive ? '2px solid var(--accent)' : '2px solid transparent',
                      transition: 'border-color 0.15s, transform 0.1s',
                    }}
                    onMouseDown={(e) => { e.currentTarget.style.transform = 'scale(0.96)' }}
                    onMouseUp={(e) => { e.currentTarget.style.transform = 'scale(1)' }}
                    onMouseLeave={(e) => { e.currentTarget.style.transform = 'scale(1)' }}
                  >
                    <div style={{
                      height: 36,
                      borderRadius: 'calc(var(--radius) - 2px)',
                      background: gradientMap[scheme],
                      marginBottom: 6,
                      position: 'relative',
                      overflow: 'hidden',
                    }}>
                      <div style={{
                        position: 'absolute', bottom: 0, left: 0, right: 0, height: 8,
                        background: 'rgba(0,0,0,0.3)',
                      }} />
                    </div>
                    <div style={{
                      fontSize: 'var(--text-xs)',
                      textAlign: 'center',
                      color: isActive ? 'var(--accent)' : 'var(--text)',
                      fontWeight: isActive ? 600 : 400,
                    }}>
                      {t('settings.colors.' + scheme)}
                    </div>
                  </div>
                )
              })}
            </div>

            {/* Custom Spectrum Colors - nested inside the same card when selected */}
            {spectrumColorScheme === 'custom' && (
              <div style={{ marginTop: 16, paddingTop: 12, borderTop: '1px solid var(--border)' }}>
                <h4 style={{ margin: '0 0 8px', fontSize: 'var(--text-sm)', color: 'var(--text-dim)' }}>{t('settings.spectrum.customColors')}</h4>
                <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(160px, 1fr))', gap: 8, minWidth: 0 }}>
                  {[
                    { key: 'bottom' as const, label: t('settings.spectrum.bottomColor') },
                    { key: 'top' as const, label: t('settings.spectrum.topColor') },
                    { key: 'peak' as const, label: t('settings.spectrum.peakColor') },
                    { key: 'grid' as const, label: t('settings.spectrum.gridColor') },
                  ].map((item) => (
                    <div key={item.key} style={{ display: 'flex', gap: 6, alignItems: 'center', minWidth: 0 }}>
                      <span style={{ fontSize: 'var(--text-xs)', color: 'var(--text-dim)', minWidth: 40, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>{item.label}</span>
                      <input
                        type="color"
                        value={customSpectrumColors[item.key]}
                        onChange={(e) => handleCustomSpectrumColor(item.key, e.target.value)}
                        style={{ width: 24, height: 24, border: 'none', borderRadius: 4, cursor: 'pointer', padding: 0, background: 'none', flexShrink: 0 }}
                      />
                      <input
                        type="text"
                        value={customSpectrumColors[item.key]}
                        onChange={(e) => handleCustomSpectrumColor(item.key, e.target.value)}
                        style={{
                          flex: 1, fontSize: 'var(--text-xs)', fontFamily: 'var(--mono)',
                          padding: '3px 6px', background: 'var(--bg)', color: 'var(--text)',
                          border: '1px solid var(--border)', borderRadius: 'var(--radius-sm)',
                          outline: 'none', minWidth: 0,
                        }}
                      />
                    </div>
                  ))}
                </div>
              </div>
            )}
          </div>

          {/* Spectrum Parameters */}
          <div className="card" style={{ marginBottom: 16 }}>
            <div className="setting-row" style={{ justifyContent: 'space-between', marginBottom: 10 }}>
              <h3 style={{ margin: 0 }}>{t('settings.spectrum.params')}</h3>
              <button className="btn btn-outline btn-sm" onClick={resetSpectrumParams}>
                {t('settings.spectrum.resetParams')}
              </button>
            </div>

            {/* Smoothing */}
            <div className="setting-row" style={{ gap: 12, alignItems: 'center', marginBottom: 8 }}>
              <span style={{ fontSize: 'var(--text-sm)', color: 'var(--text-dim)', minWidth: 80 }}>{t('settings.spectrum.smoothing')}</span>
              <input
                type="range"
                min="0" max="0.5" step="0.01"
                value={spectrumSmoothing}
                onChange={(e) => handleSpectrumSmoothing(parseFloat(e.target.value))}
                className="eq-gain-slider"
                style={{ flex: 1 }}
              />
              <span style={{ fontSize: 'var(--text-sm)', fontFamily: 'var(--mono)', color: 'var(--text)', minWidth: 36, textAlign: 'right' }}>
                {spectrumSmoothing.toFixed(2)}
              </span>
            </div>

            {/* Decay */}
            <div className="setting-row" style={{ gap: 12, alignItems: 'center', marginBottom: 8 }}>
              <span style={{ fontSize: 'var(--text-sm)', color: 'var(--text-dim)', minWidth: 80 }}>{t('settings.spectrum.decay')}</span>
              <input
                type="range"
                min="0.5" max="0.99" step="0.01"
                value={spectrumDecay}
                onChange={(e) => handleSpectrumDecay(parseFloat(e.target.value))}
                className="eq-gain-slider"
                style={{ flex: 1 }}
              />
              <span style={{ fontSize: 'var(--text-sm)', fontFamily: 'var(--mono)', color: 'var(--text)', minWidth: 36, textAlign: 'right' }}>
                {spectrumDecay.toFixed(2)}
              </span>
            </div>

            {/* FPS */}
            <div className="setting-row" style={{ gap: 12, alignItems: 'center', marginBottom: 12 }}>
              <span style={{ fontSize: 'var(--text-sm)', color: 'var(--text-dim)', minWidth: 80 }}>{t('settings.spectrum.fps')}</span>
              <input
                type="range"
                min="10" max="60" step="5"
                value={spectrumFps}
                onChange={(e) => handleSpectrumFps(parseInt(e.target.value))}
                className="eq-gain-slider"
                style={{ flex: 1 }}
              />
              <span style={{ fontSize: 'var(--text-sm)', fontFamily: 'var(--mono)', color: 'var(--text)', minWidth: 28, textAlign: 'right' }}>
                {spectrumFps}
              </span>
            </div>
          </div>

          {/* Custom CSS */}
          <div className="card" style={{ marginBottom: 16 }}>
            <div className="setting-row" style={{ justifyContent: 'space-between', marginBottom: 6 }}>
              <span className="setting-label">{t('settings.appearance.customCss')}</span>
              <button
                className={`toggle-btn ${cssEnabled ? 'on' : ''}`}
                onClick={toggleCssEnabled}
                style={{
                  padding: '2px 10px', borderRadius: 12,
                  fontSize: 'var(--text-xs)', fontWeight: 600, border: 'none', cursor: 'pointer',
                  background: cssEnabled ? 'var(--accent)' : 'var(--bg-hover)',
                  color: cssEnabled ? '#fff' : 'var(--text-dim)',
                }}
              >
                {cssEnabled ? t('common.on') : t('common.off')}
              </button>
            </div>
            <p className="setting-hint" style={{ marginTop: 4 }}>
              {t('settings.appearance.customCss.desc')}
            </p>
            <div style={{ marginTop: 8 }}>
              <textarea
                className="custom-css-editor"
                value={customCss}
                onChange={(e) => handleCustomCss(e.target.value)}
                placeholder={t('settings.appearance.customCss.placeholder')}
                spellCheck={false}
                style={{ maxHeight: 200, overflowY: 'auto' }}
              />
            </div>
            <div className="setting-row" style={{ marginTop: 8, gap: 8 }}>
              <button className="btn-ghost" onClick={handleCustomCssFile}>
                {t('settings.appearance.customCss.load')}
              </button>
              <input
                ref={cssFileInputRef}
                type="file"
                accept=".css,.txt"
                style={{ display: 'none' }}
                onChange={handleCssFileChange}
              />
              {customCss && (
                <button className="btn-ghost" onClick={clearCustomCss}>
                  {t('settings.appearance.customCss.clear')}
                </button>
              )}
            </div>
          </div>
        </div>
      )}

      {/* Audio Tab */}
      {settingsTab === 'audio' && (
        <div className="settings-tab-content">
          {/* Volume Mode + Exclusive Mode + DSD are Windows-only (WASAPI) */}
          {platform === 'windows' && (
          <>
          {/* Volume Mode */}
          <div className="card" style={{ marginBottom: 16 }}>
            <h3>{t('settings.volumeControl')}</h3>
            <p className="setting-desc">{t('settings.volumeControl.desc')}</p>
            <div className="setting-row" style={{ gap: 8, marginTop: 10 }}>
              {(['Hardware', 'Software'] as VolMode[]).map((mode) => (
                <button
                  key={mode}
                  className={`btn ${volumeMode === mode ? 'btn-primary' : 'btn-outline'} btn-sm`}
                  onClick={() => handleVolumeMode(mode)}
                >
                  {t('settings.volumeControl.' + (mode === 'Hardware' ? 'hw' : 'sw'))}
                </button>
              ))}
            </div>
          </div>

          {/* Exclusive Mode (WASAPI) — bypasses the Windows mixer for
              bit-perfect output. Button label doubles as status display. */}
          <div className="card" style={{ marginBottom: 16 }}>
            <h3>{t('settings.exclusiveMode')}</h3>
            <p className="setting-desc">{t('settings.exclusiveMode.desc')}</p>
            <div className="setting-row" style={{ gap: 8, marginTop: 10 }}>
              <button
                className={`btn ${exclusive ? 'btn-primary' : 'btn-outline'} btn-sm`}
                onClick={() => handleExclusiveMode(!exclusive)}
              >
                {exclusive ? t('settings.exclusiveMode.on') : t('settings.exclusiveMode.off')}
              </button>
            </div>
          </div>
          </>
          )}

          {/* Bit Depth */}
          <div className="card" style={{ marginBottom: 16 }}>
            <h3>{t('settings.bitDepth')}</h3>
            <p className="setting-desc">{t('settings.bitDepth.desc')}</p>
            <div className="setting-row" style={{ gap: 8, marginTop: 10 }}>
              {[
                { value: 'Float32', label: t('settings.bitDepth.float32'), desc: t('settings.bitDepth.float32.desc') },
                { value: 'I16', label: t('settings.bitDepth.16bit'), desc: t('settings.bitDepth.16bit.desc') },
                { value: 'I24', label: t('settings.bitDepth.24bit'), desc: t('settings.bitDepth.24bit.desc') },
              ].map((opt) => (
                <button
                  key={opt.value}
                  className={`btn ${bitDepth === opt.value ? 'btn-primary' : 'btn-outline'} btn-sm`}
                  onClick={() => handleBitDepth(opt.value)}
                  title={opt.desc}
                >
                  {opt.label}
                </button>
              ))}
            </div>
          </div>

          {/* ReplayGain */}
          <div className="card" style={{ marginBottom: 16 }}>
            <h3>{t('settings.replayGain')}</h3>
            <p className="setting-desc">{t('settings.replayGain.desc')}</p>
            <div className="setting-row" style={{ gap: 8, marginTop: 10, flexWrap: 'wrap' }}>
              {(['Off', 'Track', 'Album'] as RgMode[]).map((mode) => (
                <button
                  key={mode}
                  className={`btn ${replayGainMode === mode ? 'btn-primary' : 'btn-outline'} btn-sm`}
                  onClick={() => handleReplayGainMode(mode)}
                >
                  {t('settings.replayGain.' + mode.toLowerCase())}
                </button>
              ))}
            </div>
            <div className="setting-row" style={{ gap: 12, alignItems: 'center', marginTop: 12 }}>
              <span style={{ fontSize: 'var(--text-sm)', color: 'var(--text-dim)', minWidth: 80 }}>{t('settings.replayGain.preamp')}</span>
              <input
                type="range"
                min="-12" max="12" step="0.5"
                value={replayGainPreamp}
                onChange={(e) => handleReplayGainPreamp(parseFloat(e.target.value))}
                onMouseUp={() => handleReplayGainPreamp(replayGainPreamp)}
                className="eq-gain-slider"
                style={{ flex: 1 }}
              />
              <span style={{ fontSize: 'var(--text-sm)', fontFamily: 'var(--mono)', color: 'var(--text)', minWidth: 44, textAlign: 'right' }}>
                {replayGainPreamp > 0 ? '+' : ''}{replayGainPreamp.toFixed(1)} dB
              </span>
            </div>
          </div>

          {/* Quality */}
          <div className="card" style={{ marginBottom: 16 }}>
            <h3>{t('settings.resamplerQuality')}</h3>
            <div className="setting-row" style={{ gap: 8, marginTop: 10 }}>
              {(['Fast', 'Balanced', 'High'] as Quality[]).map((q) => (
                <button
                  key={q}
                  className={`btn ${quality === q ? 'btn-primary' : 'btn-outline'} btn-sm`}
                  onClick={() => handleQuality(q)}
                >
                  {t('settings.resamplerQuality.' + q.toLowerCase() + '.label')}
                </button>
              ))}
            </div>
          </div>

          {/* Output Sample Rate */}
          <div className="card" style={{ marginBottom: 16 }}>
            <h3>{t('settings.outputSampleRate')}</h3>
            <p className="setting-desc">{t('settings.outputSampleRate.desc')}</p>
            <div className="setting-row" style={{ gap: 8, marginTop: 10, flexWrap: 'wrap' }}>
              {[
                { value: 0, label: t('settings.outputSampleRate.auto') },
                { value: 44100, label: '44.1 kHz' },
                { value: 48000, label: '48 kHz' },
                { value: 88200, label: '88.2 kHz' },
                { value: 96000, label: '96 kHz' },
                { value: 176400, label: '176.4 kHz' },
                { value: 192000, label: '192 kHz' },
                { value: 352800, label: '352.8 kHz' },
                { value: 384000, label: '384 kHz' },
              ].map((opt) => (
                <button
                  key={opt.value}
                  className={`btn ${outputSampleRate === opt.value ? 'btn-primary' : 'btn-outline'} btn-sm`}
                  onClick={() => handleOutputSampleRate(opt.value)}
                >
                  {opt.label}
                </button>
              ))}
            </div>
          </div>

          {/* DSD Mode (DoP requires WASAPI exclusive — Windows only) */}
          {platform === 'windows' && (
          <div className="card" style={{ marginBottom: 16 }}>
            <h3>{t('settings.dsdMode')}</h3>
            <p className="setting-desc">{t('settings.dsdMode.desc')}</p>
            <div className="setting-row" style={{ gap: 8, marginTop: 10, flexWrap: 'wrap' }}>
              {[
                { value: 'Off', label: t('settings.dsdMode.off'), desc: t('settings.dsdMode.off.desc') },
                { value: 'DoP_Dsd64', label: t('settings.dsdMode.dopDsd64'), desc: t('settings.dsdMode.dopDsd64.desc') },
                { value: 'DoP_Dsd128', label: t('settings.dsdMode.dopDsd128'), desc: t('settings.dsdMode.dopDsd128.desc') },
                { value: 'DoP_Dsd256', label: t('settings.dsdMode.dopDsd256'), desc: t('settings.dsdMode.dopDsd256.desc') },
              ].map((opt) => (
                <button
                  key={opt.value}
                  className={`btn ${dsdMode === opt.value ? 'btn-primary' : 'btn-outline'} btn-sm`}
                  onClick={() => handleDsdMode(opt.value)}
                  title={opt.desc}
                >
                  {opt.label}
                </button>
              ))}
            </div>
            {dsdMode !== 'Off' && !exclusive && (
              <p
                className="setting-desc"
                style={{ color: 'var(--warning, #ffb84d)', marginTop: 8 }}
              >
                {t('settings.dsdMode.needsExclusive')}
              </p>
            )}
          </div>
          )}

          {/* Output Device Policies (Plan B) */}
          <div className="card" style={{ marginBottom: 16 }}>
            <h3>{t('settings.hotplugPolicy')}</h3>
            <p className="setting-desc">{t('settings.hotplugPolicy.desc')}</p>

            {/* On removed strategy */}
            <div className="setting-row" style={{ gap: 8, marginTop: 12, flexWrap: 'wrap', alignItems: 'center' }}>
              <span className="setting-label" style={{ minWidth: 140 }}>{t('settings.hotplugPolicy.onRemoved')}</span>
              {([
                { value: 'pause' as DeviceRemovedStrategy,            label: t('settings.hotplugPolicy.pause'),            hint: t('settings.hotplugPolicy.pause.hint') },
                { value: 'switch_default' as DeviceRemovedStrategy,    label: t('settings.hotplugPolicy.switchDefault'),    hint: t('settings.hotplugPolicy.switchDefault.hint') },
                { value: 'switch_last_used' as DeviceRemovedStrategy,  label: t('settings.hotplugPolicy.switchLastUsed'),  hint: t('settings.hotplugPolicy.switchLastUsed.hint') },
              ]).map((opt) => {
                const active = hotplugStatus?.strategy.on_device_removed === opt.value
                return (
                  <button
                    key={opt.value}
                    className={`btn ${active ? 'btn-primary' : 'btn-outline'} btn-sm`}
                    onClick={() => handleOnDeviceRemoved(opt.value)}
                    title={opt.hint}
                  >
                    {opt.label}
                  </button>
                )
              })}
            </div>

            {/* On inserted strategy */}
            <div className="setting-row" style={{ gap: 8, marginTop: 12, flexWrap: 'wrap', alignItems: 'center' }}>
              <span className="setting-label" style={{ minWidth: 140 }}>{t('settings.hotplugPolicy.onInserted')}</span>
              {([
                { value: 'ignore' as DeviceInsertedStrategy,      label: t('settings.hotplugPolicy.ignore'),      hint: t('settings.hotplugPolicy.ignore.hint') },
                { value: 'auto_switch' as DeviceInsertedStrategy,  label: t('settings.hotplugPolicy.autoSwitch'),  hint: t('settings.hotplugPolicy.autoSwitch.hint') },
              ]).map((opt) => {
                const active = hotplugStatus?.strategy.on_new_device_inserted === opt.value
                return (
                  <button
                    key={opt.value}
                    className={`btn ${active ? 'btn-primary' : 'btn-outline'} btn-sm`}
                    onClick={() => handleOnNewDeviceInserted(opt.value)}
                    title={opt.hint}
                  >
                    {opt.label}
                  </button>
                )
              })}
            </div>

            {/* Last used device */}
            <div className="setting-row" style={{ gap: 8, marginTop: 12, alignItems: 'center' }}>
              <span className="setting-label" style={{ minWidth: 140 }}>{t('settings.hotplugPolicy.lastUsed')}</span>
              <span style={{ fontSize: 'var(--text-sm)', color: 'var(--text-dim)', fontFamily: 'var(--mono)' }}>
                {hotplugStatus?.last_used_device_id ?? t('settings.hotplugPolicy.lastUsed.none')}
              </span>
            </div>

            {/* Hidden QA harness — show only when localStorage.phonon_devtools === '1' */}
            {typeof window !== 'undefined' && window.localStorage.getItem('phonon_devtools') === '1' && (
              <div className="setting-row" style={{ gap: 8, marginTop: 12, flexWrap: 'wrap', alignItems: 'center' }}>
                <span className="setting-label" style={{ minWidth: 140 }}>{t('settings.hotplugPolicy.devtools')}</span>
                <button className="btn btn-outline btn-sm" onClick={() => handleDebugInject('removed')}>
                  {t('settings.hotplugPolicy.devtools.removeCurrent')}
                </button>
                <button className="btn btn-outline btn-sm" onClick={() => handleDebugInject('added')}>
                  {t('settings.hotplugPolicy.devtools.insertNonDefault')}
                </button>
                <button className="btn btn-outline btn-sm" onClick={() => handleDebugInject('default_changed')}>
                  {t('settings.hotplugPolicy.devtools.defaultChanged')}
                </button>
                <button className="btn btn-outline btn-sm" onClick={refreshHotplugStatus}>
                  {t('settings.hotplugPolicy.devtools.refresh')}
                </button>
              </div>
            )}
          </div>

          {/* Audio Diagnostics */}
          <div className="card" style={{ marginBottom: 16 }}>
            <h3>{t('settings.audioDiagnostics')}</h3>
            <p className="setting-desc">{t('settings.audioDiagnostics.desc')}</p>
            {audioDiag ? (
              <div
                style={{
                  marginTop: 10,
                  display: 'grid',
                  gridTemplateColumns: 'auto 1fr',
                  gap: '4px 16px',
                  fontSize: 'var(--text-sm)',
                  fontFamily: 'var(--mono)',
                }}
              >
                <span style={{ color: 'var(--text-dim)' }}>{t('diag.mode')}</span>
                <span>{audioDiag.mode}</span>
                <span style={{ color: 'var(--text-dim)' }}>{t('diag.device')}</span>
                <span>{audioDiag.device_name ?? '--'}</span>
                <span style={{ color: 'var(--text-dim)' }}>{t('diag.format')}</span>
                <span>{audioDiag.format || '--'}</span>
                <span style={{ color: 'var(--text-dim)' }}>{t('diag.sampleRate')}</span>
                <span>{audioDiag.sample_rate} Hz</span>
                <span style={{ color: 'var(--text-dim)' }}>{t('diag.channels')}</span>
                <span>{audioDiag.channels}</span>
                {audioDiag.buffer_frames > 0 && (
                  <>
                    <span style={{ color: 'var(--text-dim)' }}>{t('diag.buffer')}</span>
                    <span>{audioDiag.buffer_frames}</span>
                    <span style={{ color: 'var(--text-dim)' }}>{t('diag.padding')}</span>
                    <span>{audioDiag.padding_frames}</span>
                  </>
                )}
                <span style={{ color: 'var(--text-dim)' }}>{t('diag.ring')}</span>
                <span>
                  {audioDiag.ring_available_frames} / {audioDiag.ring_capacity_frames}
                </span>
                <span style={{ color: 'var(--text-dim)' }}>{t('diag.underruns')}</span>
                <span
                  style={{ color: audioDiag.underruns > 0 ? 'var(--danger, #ff5066)' : undefined }}
                >
                  {audioDiag.underruns}
                </span>
              </div>
            ) : (
              <p className="setting-desc" style={{ marginTop: 10 }}>…</p>
            )}
            <div style={{ marginTop: 10 }}>
              <button
                className="btn btn-outline btn-sm"
                onClick={() => {
                  if (!audioDiag) return
                  navigator.clipboard
                    .writeText(JSON.stringify(audioDiag, null, 2))
                    .then(() => addToast(t('diag.copied'), 'info'))
                    .catch(() => {})
                }}
              >
                {t('diag.copy')}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* About Tab */}
      {settingsTab === 'about' && (
        <div className="settings-tab-content">
          <div className="card" style={{ marginBottom: 16, textAlign: 'center', padding: '32px 24px' }}>
            <h2 style={{ margin: '0 0 4px', fontSize: '26px', fontWeight: 700, color: 'var(--accent)' }}>Phonon</h2>
            <p style={{ margin: '0 0 20px', fontSize: 'var(--text-base)', color: 'var(--text-dim)' }}>
              {t('settings.audioOutput')} — High-Fidelity Audio Player
            </p>
            <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: '8px 16px', maxWidth: 320, margin: '0 auto', fontSize: 'var(--text-base)' }}>
              <span style={{ color: 'var(--text-dim)', textAlign: 'right' }}>{t('settings.about.version')}</span>
              <span style={{ color: 'var(--text)', fontFamily: 'var(--mono)', textAlign: 'left' }}>
                {buildInfo ? `${buildInfo.version} (${buildInfo.build_sha || 'dev'})` : '1.0.0'}
              </span>
              <span style={{ color: 'var(--text-dim)', textAlign: 'right' }}>{t('settings.about.engine')}</span>
              <span style={{ color: 'var(--text)', fontFamily: 'var(--mono)', textAlign: 'left' }}>Symphonia + CPAL</span>
              <span style={{ color: 'var(--text-dim)', textAlign: 'right' }}>{t('settings.about.platform')}</span>
              <span style={{ color: 'var(--text)', fontFamily: 'var(--mono)', textAlign: 'left' }}>Tauri v2 + Rust</span>
            </div>
          </div>

          <div className="card" style={{ marginBottom: 16 }}>
            <h3 style={{ margin: '0 0 12px', fontSize: 'var(--text-lg)', color: 'var(--accent)' }}>
              {t('settings.about.license.title')}
            </h3>
            <ul style={{ margin: 0, paddingLeft: 20, display: 'grid', gap: 6, fontSize: 'var(--text-sm)', color: 'var(--text-dim)', lineHeight: 1.6 }}>
              <li>{t('settings.about.license.pho')}</li>
              <li>{t('settings.about.license.codec')}</li>
              <li>{t('settings.about.license.third')}</li>
            </ul>
          </div>

          <div className="card" style={{ borderColor: 'color-mix(in srgb, var(--accent) 40%, var(--border))' }}>
            <h3 style={{ margin: '0 0 8px', fontSize: 'var(--text-lg)', color: 'var(--accent)' }}>
              {t('settings.about.eula.title')}
            </h3>
            <p style={{ margin: 0, fontSize: 'var(--text-sm)', color: 'var(--text-dim)', lineHeight: 1.7 }}>
              {t('settings.about.eula.text')}
            </p>
          </div>

        </div>
      )}

      {/* Crop Modal */}
      {cropVisible && (
        <CropEditor source={cropSource} onConfirm={handleCropConfirm} onCancel={handleCropCancel} />
      )}

      {/* Desktop lyrics background crop modal */}
      {dlCropVisible && (
        <CropEditor source={dlCropSource} onConfirm={handleDlCropConfirm} onCancel={handleDlCropCancel} />
      )}

      {/* Modal Dialog */}
      {modal && (
        <ModalDialog
          type={modal.type}
          title={modal.title}
          message={modal.message}
          defaultValue={modal.defaultValue}
          onConfirm={(val) => {
            modal.resolve(val)
            setModal(null)
          }}
          onCancel={() => {
            modal.resolve(undefined)
            setModal(null)
          }}
        />
      )}
    </div>
  )
}
