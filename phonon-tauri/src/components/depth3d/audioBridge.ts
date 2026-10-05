// 音频数据 → 3D 场景的桥接层。
// 订阅主窗口已广播的 CustomEvent('audio-features')，对外暴露
// 可订阅的 reactive refs（useAudioData hook），R3F 场景中任何组件都
// 可以订阅需要的字段，场景通过 rAF 自己做 lerp 插值到 60fps。
import { useEffect, useRef } from 'react'
import type { AudioFeatures } from '../VisualizerPage'

export interface SmoothedAudioData {
  spectrum64: Float32Array     // 上采样到 64 bins
  waveform128: Float32Array    // 下采样到 128
  rms: number                  // 响度（smoothed）
  peak: number                 // 峰值（smoothed）
  onset: number                // 瞬态触发值 0..1（decay 衰减）
  beat: boolean                // 是否是节拍点
  bpm: number
  chroma12: Float32Array       // 音高能量 12 音阶
  lowFreqAvg: number           // 低频 bins 平均（0-16 档）
  highFreqAvg: number          // 高频 bins 平均（50-64 档）
  centroidHz: number
}

const DEFAULT_SPEC = new Float32Array(64)
const DEFAULT_WF = new Float32Array(128)
const DEFAULT_CHR = new Float32Array(12)

export function createEmptyAudio(): SmoothedAudioData {
  return {
    spectrum64: DEFAULT_SPEC.slice(),
    waveform128: DEFAULT_WF.slice(),
    rms: 0,
    peak: 0,
    onset: 0,
    beat: false,
    bpm: 0,
    chroma12: DEFAULT_CHR.slice(),
    lowFreqAvg: 0,
    highFreqAvg: 0,
    centroidHz: 0,
  }
}

/** 把 features 里的 32 bin spectrum 上采样到 64 bins（线性插值） */
export function upsampleSpectrum64(src: number[]): Float32Array {
  const out = new Float32Array(64)
  if (!src || src.length === 0) return out
  if (src.length >= 64) {
    for (let i = 0; i < 64; i++) out[i] = src[i]
    return out
  }
  const step = (src.length - 1) / 63
  for (let i = 0; i < 64; i++) {
    const x = i * step
    const x0 = Math.floor(x)
    const x1 = Math.min(x0 + 1, src.length - 1)
    const t = x - x0
    const a = src[x0] ?? 0
    const b = src[x1] ?? 0
    out[i] = a + (b - a) * t
  }
  return out
}

function downsampleWaveform128(src: number[]): Float32Array {
  const out = new Float32Array(128)
  if (!src || src.length === 0) return out
  if (src.length <= 128) {
    for (let i = 0; i < src.length; i++) out[i] = src[i] ?? 0
    return out
  }
  const step = src.length / 128
  for (let i = 0; i < 128; i++) {
    const s = Math.floor(i * step)
    const e = Math.floor((i + 1) * step)
    let sum = 0
    let n = 0
    for (let k = s; k < e; k++) {
      sum += Math.abs(src[k] ?? 0)
      n++
    }
    out[i] = n > 0 ? sum / n : 0
  }
  return out
}

/** 安全数值：NaN/Infinity → 0，钳制到 [0, 1] */
function safeVal(v: number, min = 0, max = 1): number {
  if (typeof v !== 'number' || !isFinite(v)) return 0
  return Math.min(max, Math.max(min, v))
}

let lastFeatureAt = 0

/** 距离最后一次 audio-features 事件过去了多少毫秒（暂停/停止后不再推送，
 *  消费方可据此把视觉响应降为 0，避免能量值冻结导致动画永不停） */
export function featuresStaleMs(): number {
  return performance.now() - lastFeatureAt
}

/**
 * 单通道订阅：返回一个可变 ref，每当主窗口 push audio-features 事件时
 * ref 内容就地更新（不是返回新对象）。场景里每帧读 ref.current 做 lerp。
 */
export function useAudioDataRef() {
  const dataRef = useRef<SmoothedAudioData>(createEmptyAudio())
  const onsetAccumRef = useRef(0)

  useEffect(() => {
    const handler = (e: Event) => {
      lastFeatureAt = performance.now()
      const custom = e as CustomEvent<AudioFeatures>
      const f = custom.detail
      if (!f) return
      const prev = dataRef.current

      // spectrum upsampling
      const spec = upsampleSpectrum64(f.spectrum ?? [])
      for (let i = 0; i < 64; i++) {
        // 指数平滑 + NaN 保护 + 钳制
        const v = safeVal(spec[i])
        prev.spectrum64[i] = prev.spectrum64[i] * 0.55 + v * 0.45
      }

      const wf = downsampleWaveform128(f.waveform ?? [])
      for (let i = 0; i < 128; i++) {
        const v = safeVal(wf[i], -1, 1)
        prev.waveform128[i] = prev.waveform128[i] * 0.7 + v * 0.3
      }

      const chroma = f.chroma ?? []
      for (let i = 0; i < 12; i++) {
        const v = safeVal(typeof chroma[i] === 'number' ? chroma[i] : 0)
        prev.chroma12[i] = prev.chroma12[i] * 0.65 + v * 0.35
      }

      prev.rms = prev.rms * 0.7 + safeVal(f.rms ?? 0) * 0.3
      prev.peak = prev.peak * 0.85 + safeVal(f.peak ?? 0) * 0.15
      prev.centroidHz = prev.centroidHz * 0.7 + safeVal(f.spectral_centroid_hz ?? 0, 0, 20000) * 0.3

      // onset: accumulate 触发值，rAF 里每帧 decay 15%
      const on = safeVal(f.onset ?? 0)
      if (on > 0.02) onsetAccumRef.current = Math.max(onsetAccumRef.current, on)
      prev.onset = Math.max(prev.onset * 0.85, onsetAccumRef.current)
      onsetAccumRef.current *= 0.85

      prev.beat = !!f.beat
      prev.bpm = f.bpm ?? prev.bpm

      let lowSum = 0
      for (let i = 0; i < 16; i++) lowSum += prev.spectrum64[i]
      prev.lowFreqAvg = safeVal(lowSum / 16)

      let hiSum = 0
      for (let i = 50; i < 64; i++) hiSum += prev.spectrum64[i]
      prev.highFreqAvg = safeVal(hiSum / 14)
    }
    window.addEventListener('audio-features', handler as EventListener)
    return () => window.removeEventListener('audio-features', handler as EventListener)
  }, [])

  return dataRef
}

/** 每帧用 rAF 做 lerp 的通用工具函数 */
export function lerp(a: number, b: number, t: number) {
  return a + (b - a) * t
}
