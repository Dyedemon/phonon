// 三维成空 - 沉浸式 3D 音乐可视化
// 场景五层结构：流体地面 + 频谱晶体环 + 能量核心 + 流场星尘 + 氛围雾
// HUD：隐藏式交互，鼠标移动浮现四角按钮，底部 hover 呼出播放栏
import { useRef, useState, useCallback, useEffect } from 'react'
import * as THREE from 'three'
import { Canvas, useFrame, useThree } from '@react-three/fiber'
import { OrbitControls } from '@react-three/drei'
import { EffectComposer, Bloom, DepthOfField, Vignette, Noise } from '@react-three/postprocessing'
import { BlendFunction } from 'postprocessing'
import { useAudioDataRef, lerp } from './depth3d/audioBridge'
import type { SmoothedAudioData } from './depth3d/audioBridge'
import { NebulaCore } from './depth3d/NebulaCore'
import { StarField } from './depth3d/StarField'
import { RhythmVisualizer } from './depth3d/RhythmVisualizer'
import { HUD } from './depth3d/HUD'

interface Props {
  onClose?: () => void
  playback?: {
    state: string
    position_secs: number
    duration_secs: number | null
    current_track: string | null
  }
  volume?: number
  onVolumeChange?: (v: number) => void
  coverUrl?: string | null
  visActive?: boolean
  visMode?: string
  onToggleVisActive?: (on: boolean) => void
  onVisModeChange?: (mode: any) => void
  onEnterDimension?: (mode: any) => void
}

export default function Depth3DPage({
  onClose, playback, volume = 80, onVolumeChange, coverUrl = null,
  visActive = true, visMode = 'depth3d',
  onToggleVisActive, onVisModeChange, onEnterDimension,
}: Props) {
  // 未使用参数占位（未来扩展用）
  void visActive; void visMode; void onToggleVisActive; void onVisModeChange; void onEnterDimension
  void coverUrl
  const audioRef = useAudioDataRef()

  // ===== 3D 场景设置状态 =====
  const [sceneSettings, setSceneSettings] = useState({
    theme: 'nebula',           // 可视化主题
    postProcessing: true,      // 后处理效果
    quality: 'high',           // 质量预设：low / mid / high / ultra
  })

  const setSetting = useCallback((key: string, value: any) => {
    setSceneSettings((prev) => ({ ...prev, [key]: value }))
  }, [])

  // 主题切换转场：主题变化时触发闪黑渐出效果
  const [transitionKey, setTransitionKey] = useState(0)
  const prevThemeRef = useRef(sceneSettings.theme)

  useEffect(() => {
    if (prevThemeRef.current !== sceneSettings.theme) {
      prevThemeRef.current = sceneSettings.theme
      setTransitionKey((k) => k + 1)
    }
  }, [sceneSettings.theme])

  // 自适应渲染倍率：GPU 跟不上时（帧时间下降）自动降档，避免
  // 帧超时导致合成器读到空缓冲（表现为整窗闪烁）；恢复流畅后回升。
  const [dprScale, setDprScale] = useState(1)

  // 质量预设 -> 渲染倍率上限。星云着色器较重，4K 下 dpr 2 会把中端
  // GPU 压垮，因此 high 档从 2 降到 1.5（配合自适应仍可自动再降）。
  const dprCap =
    sceneSettings.quality === 'low' ? 1
    : sceneSettings.quality === 'mid' ? 1.25
    : sceneSettings.quality === 'ultra' ? 2
    : 1.5

  return (
    <div style={{
      position: 'fixed', inset: 0,
      width: '100vw', height: '100vh',
      background: '#03050f',
      overflow: 'hidden', zIndex: 9999,
    }}>
      {/* 3D 画布 */}
      <Canvas
        shadows
        dpr={[1, dprCap * dprScale]}
        frameloop="demand"
        gl={{ antialias: true, alpha: false, powerPreference: 'high-performance', stencil: false }}
        camera={{ position: [0, 1.5, 13], fov: 60, near: 0.1, far: 200 }}
        style={{ position: 'absolute', inset: 0, zIndex: 0 }}
      >
        {/* 背景色：深靛蓝紫 */}
        <color attach="background" args={['#050718']} />
        {/* 雾：营造空间深度 */}
        <fog attach="fog" args={['#050718', 28, 65]} />

        <CameraRig
          audioRef={audioRef}
          theme={sceneSettings.theme}
        />

        {/* 帧率上限：240Hz 屏幕上把渲染节流到 60fps（见组件注释） */}
        <FramerateCap fps={60} />

        {/* 帧时间调节器：GPU 跟不上时自动降低渲染倍率 */}
        <FrameGovernor
          onAdjust={(delta) =>
            setDprScale((s) => Math.min(1, Math.max(0.6, s + delta * 0.2)))
          }
        />

        {/* 灯光体系：整体偏暗，靠发光物体照亮场景 */}
        <ambientLight intensity={0.12} color="#4a5a8a" />
        {/* 主光：从斜上方打，偏冷蓝色 */}
        <directionalLight
          position={[8, 16, 10]}
          intensity={0.45}
          color={'#a8c0ff'}
          castShadow
          shadow-mapSize-width={1024}
          shadow-mapSize-height={1024}
          shadow-camera-near={0.5}
          shadow-camera-far={50}
        />
        {/* 补光：下方反射光，偏紫 */}
        <pointLight position={[0, -2, 2]} intensity={0.35} color={'#8a6aff'} distance={30} decay={2} />
        {/* 侧面轮廓光：偏青 + 偏粉 */}
        <pointLight position={[-10, 4, -8]} intensity={0.25} color={'#5ac8ff'} distance={40} decay={2} />
        <pointLight position={[10, 4, -8]} intensity={0.25} color={'#ff8ac0'} distance={40} decay={2} />

        {/* 注意：不要加 <Environment>。它会从 CDN 下载 HDR 环境贴图，
            离线/弱网时 fetch 失败并反复重试，导致整个 3D 页面重新挂载
            （表现为全窗闪烁）。场景的照明由上面的实体灯光承担。 */}

        {/* 场景内容：按主题切换 */}
        {sceneSettings.theme === 'nebula' && (
          <>
            <StarField audioRef={audioRef} quality={sceneSettings.quality} />
            <NebulaCore audioRef={audioRef} quality={sceneSettings.quality} />
          </>
        )}

        {sceneSettings.theme === 'rhythm' && (
          <>
            <RhythmVisualizer audioRef={audioRef} />
          </>
        )}

        {/* OrbitControls：自由视角（星云模式） */}
        {sceneSettings.theme === 'nebula' && (
          <OrbitControls
            enablePan={false}
            enableDamping
            dampingFactor={0.05}
            minDistance={5}
            maxDistance={45}
            maxPolarAngle={Math.PI / 2 - 0.05}
            minPolarAngle={0.25}
            target={[0, -0.5, 0]}
            makeDefault
          />
        )}

        {/* OrbitControls：节奏模式（固定俯视视角，限制移动） */}
        {sceneSettings.theme === 'rhythm' && (
          <OrbitControls
            enablePan={false}
            enableDamping
            dampingFactor={0.08}
            minDistance={5}
            maxDistance={7.5}
            maxPolarAngle={Math.PI / 2.1}
            minPolarAngle={Math.PI / 3}
            minAzimuthAngle={-0.2}
            maxAzimuthAngle={0.2}
            target={[0, 0.2, 0]}
            makeDefault
          />
        )}

        {/* 后期处理（按质量分级）。
            景深（DoF）是全屏后处理里最贵的一项，只在 ultra 档保留——
            中低档关掉它可显著降低帧时间（丢帧会表现为整窗闪烁）。
            MSAA（multisampling）同理，仅 ultra 开启。 */}
        {sceneSettings.postProcessing && (
          <EffectComposer multisampling={sceneSettings.quality === 'ultra' ? 2 : 0} enableNormalPass={false}>
            {sceneSettings.quality === 'ultra' && (
              <DepthOfField focusDistance={0.012} focalLength={0.02} bokehScale={1.5} />
            )}
            <Bloom
              intensity={sceneSettings.quality === 'low' ? 0.2 : sceneSettings.quality === 'mid' ? 0.28 : sceneSettings.quality === 'ultra' ? 0.45 : 0.32}
              luminanceThreshold={sceneSettings.quality === 'low' ? 0.85 : sceneSettings.quality === 'mid' ? 0.78 : 0.75}
              luminanceSmoothing={0.9}
              mipmapBlur
              radius={sceneSettings.quality === 'low' ? 0.35 : sceneSettings.quality === 'mid' ? 0.45 : sceneSettings.quality === 'ultra' ? 0.7 : 0.55}
            />
            <Vignette eskil={false} offset={0.2} darkness={0.65} />
            {sceneSettings.quality !== 'low' && (
              <Noise blendFunction={BlendFunction.COLOR_DODGE} opacity={sceneSettings.quality === 'mid' ? 0.02 : 0.03} />
            )}
          </EffectComposer>
        )}
      </Canvas>

      {/* 入场动画 + 主题切换转场遮罩 */}
      <div
        key={`entry-${transitionKey}`}
        style={{
          position: 'absolute',
          inset: 0,
          background: '#03050f',
          pointerEvents: 'none',
          zIndex: 10000,
          animation: transitionKey > 0
            ? 'themeFade 500ms ease-out forwards'
            : 'entryFade 1200ms ease-out forwards',
        }}
      />
      <style>{`
        @keyframes entryFade {
          0% { opacity: 1; }
          60% { opacity: 0.4; }
          100% { opacity: 0; }
        }
        @keyframes themeFade {
          0% { opacity: 0; }
          20% { opacity: 1; }
          100% { opacity: 0; }
        }
      `}</style>

      {/* HUD：隐藏式交互界面 */}
      <HUD
        playback={playback}
        volume={volume}
        onVolumeChange={onVolumeChange}
        onExit3D={onClose}
        sceneSettings={sceneSettings}
        onSettingChange={setSetting}
      />
    </div>
  )
}

// ========== 相机 rig：target 轻微浮动 ==========
/** 帧率上限：把渲染节流到指定帧率（默认 60）。
 *
 *  rAF 跟随显示器刷新率——在 240Hz 屏幕上 R3F 会尝试每秒渲染 240 帧，
 *  每帧预算只有 4.17ms，3D 场景（尤其星云主题）根本跑不进这个窗口，
 *  帧超时会让合成器读到空缓冲（整窗闪黑）。节流到 60fps 后每帧预算
 *  放大到 16.7ms，且对这种可视化场景观感无差别。
 *  配合 Canvas 的 frameloop="demand"：只有 invalidate() 时才渲染。 */
function FramerateCap({ fps = 60 }: { fps?: number }) {
  const invalidate = useThree((s) => s.invalidate)
  useEffect(() => {
    let raf = 0
    let last = 0
    const interval = 1000 / fps - 1
    const loop = (t: number) => {
      raf = requestAnimationFrame(loop)
      if (t - last >= interval) {
        last = t
        invalidate()
      }
    }
    raf = requestAnimationFrame(loop)
    return () => cancelAnimationFrame(raf)
  }, [invalidate, fps])
  return null
}

/** 帧时间调节器：滚动统计平均帧时间，超标则请求降档、流畅则请求升档。
 *  仅统计、不渲染；配合外层 dprScale 使用。 */
function FrameGovernor({ onAdjust }: { onAdjust: (delta: number) => void }) {
  const stat = useRef({ sum: 0, n: 0, last: 0 })
  useFrame(() => {
    const now = performance.now()
    const s = stat.current
    if (s.last > 0) {
      s.sum += now - s.last
      s.n += 1
      if (s.n >= 40) {
        const avg = s.sum / s.n
        s.sum = 0
        s.n = 0
        // 60Hz 一帧 16.7ms；超过 24ms（<42fps）说明开始丢帧，降档；
        // 低于 18ms 且仍有余量时尝试升档（配合 alternate 双向）
        if (avg > 24) onAdjust(-1)
        else if (avg < 18) onAdjust(1)
      }
    }
    s.last = now
  })
  return null
}

function CameraRig({ audioRef, theme }: {
  audioRef: React.MutableRefObject<SmoothedAudioData>
  theme: string
}) {
  const { controls, camera } = useThree()
  const isRhythm = theme === 'rhythm'
  const prevThemeRef = useRef(theme)
  const targetBaseRef = useRef(new THREE.Vector3(0, isRhythm ? 0.2 : -0.5, isRhythm ? 0 : 0))
  const targetShakeRef = useRef(new THREE.Vector3())
  const beatShakeRef = useRef(0)
  const lastBeatRef = useRef(false)

  // 主题切换时重置相机位置
  useEffect(() => {
    if (prevThemeRef.current !== theme) {
      prevThemeRef.current = theme
      if (theme === 'rhythm') {
        // 节奏模式：低角度俯视，聚焦判定线附近
        camera.position.set(0, 3.2, 6)
        targetBaseRef.current.set(0, 0.2, 0)
      } else {
        // 星云模式：默认视角
        camera.position.set(0, 1.5, 13)
        targetBaseRef.current.set(0, -0.5, 0)
      }
      if (controls) {
        const c = controls as any
        if (c.target) c.target.copy(targetBaseRef.current)
        if (typeof c.update === 'function') c.update()
      }
    }
  }, [theme, camera, controls])

  useFrame(({ clock }) => {
    const d = audioRef.current

    // beat 检测（用于节奏模式轻微抖动）
    if (d.beat && !lastBeatRef.current) beatShakeRef.current = 1
    beatShakeRef.current *= 0.85
    lastBeatRef.current = d.beat

    if (isRhythm) {
      // 节奏模式：OrbitControls 负责控制相机，这里只做极轻量的 beat 推拉
      // 不覆盖 target，避免用户无法拖动
      if (controls) {
        const c = controls as any
        const beatPush = beatShakeRef.current * 0.08
        // 只对相机位置做极微小的 beat 推拉（不影响 target，用户可自由拖动）
        if (camera && c.target) {
          const dir = new THREE.Vector3()
          dir.subVectors(camera.position, c.target).normalize()
          camera.position.addScaledVector(dir, -beatPush * 0.15)
        }
      }
    } else {
      // 星云模式：正常幅度的浮动，覆盖 target
      targetShakeRef.current.x = lerp(targetShakeRef.current.x, 0, 0.08)
      targetShakeRef.current.y = lerp(targetShakeRef.current.y, Math.sin(clock.elapsedTime * 0.25) * 0.05 - d.lowFreqAvg * 0.04, 0.08)
      targetShakeRef.current.z = lerp(targetShakeRef.current.z, -d.lowFreqAvg * 0.1, 0.08)

      if (controls) {
        const c = controls as any
        if (c.target) {
          c.target.copy(targetBaseRef.current).add(targetShakeRef.current)
          if (typeof c.update === 'function') c.update()
        }
      }
    }
  })
  return null
}
