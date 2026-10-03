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

  // ===== 3D 场景设置状态（跨进入持久化）=====
  // 每次进页都重置会让用户反复看 governor 重新降档的闪屏，
  // 也丢掉主题/画质选择——照 phonon-vis-mode 的模式存 localStorage。
  // 水合必须做白名单校验：脏数据（旧版本/手改）不能带崩页面。
  const [sceneSettings, setSceneSettings] = useState(() => {
    const defaults = {
      theme: 'nebula',           // 可视化主题
      postProcessing: true,      // 后处理效果
      quality: 'high',           // 质量预设：low / mid / high / ultra
    }
    try {
      const raw = localStorage.getItem('phonon-3d-settings')
      if (!raw) return defaults
      const p = JSON.parse(raw) as Record<string, unknown>
      return {
        theme: p.theme === 'rhythm' ? 'rhythm' : 'nebula',
        postProcessing: typeof p.postProcessing === 'boolean' ? p.postProcessing : true,
        quality: ['low', 'mid', 'high', 'ultra'].includes(p.quality as string)
          ? (p.quality as string)
          : 'high',
      }
    } catch { /* 损坏的 JSON 按默认值走 */ }
    return defaults
  })

  // 任何设置变化统一写回（含 HUD 里的 setSetting 全部入口）
  useEffect(() => {
    try { localStorage.setItem('phonon-3d-settings', JSON.stringify(sceneSettings)) } catch { /* ignore */ }
  }, [sceneSettings])

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

  // ── 帧预算自适应：统一降级阶梯（level 越大越省）──
  // 调节器是尖峰驱动的（见 FrameGovernor）：窗口内出现多次长帧 → 降
  // 一级；连续 5 个零尖峰窗口（≈5s）且距上次降档 8s 冷却后才升一级。
  // ultra 阶梯：满血(DoF+全分辨率) → 关DoF → 分辨率×0.8 → ×0.6 → ×0.45；
  // 其余档位只走分辨率阶梯。分辨率用固定值而不是 [min,max] 区间——
  // 区间会被设备 DPR 截断，产生"降了档但像素数没变"的空操作。
  const [level, setLevel] = useState(0)
  const levelRef = useRef(0)
  const lastDownRef = useRef(0)
  const ultra = sceneSettings.quality === 'ultra'
  const maxLevel = ultra ? 4 : 3

  const applyLevel = useCallback((next: number) => {
    levelRef.current = next
    setLevel(next)
  }, [])

  const handleFrameDown = useCallback(() => {
    let next = levelRef.current + 1
    // 非 ultra 跳过 DoF 档（它本来就是关的）
    if (!ultra && next === 1) next = 2
    if (next > maxLevel) return
    lastDownRef.current = performance.now()
    applyLevel(next)
  }, [ultra, maxLevel, applyLevel])

  const handleFrameUp = useCallback(() => {
    // 冷却：贴边配置会在"刚好能跑"与"尖峰"之间反复横跳——每次跳变
    // （重建 composer/重设画布）本身就是一次闪屏，必须让降档决定 sticky。
    if (performance.now() - lastDownRef.current < 8000) return
    let next = levelRef.current - 1
    if (!ultra && next === 1) next = 0
    if (next < 0) return
    applyLevel(next)
  }, [ultra, applyLevel])

  // 切换质量预设时重置，给新预设一个干净的起点
  useEffect(() => {
    applyLevel(0)
    lastDownRef.current = 0
  }, [sceneSettings.quality, applyLevel])

  const dofOn = ultra && level < 1
  const DPR_STEPS = [1, 0.8, 0.6, 0.45]
  const dprScale = ultra
    ? (level <= 1 ? 1 : DPR_STEPS[level - 1])
    : DPR_STEPS[level]

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
        dpr={Math.min(window.devicePixelRatio || 1, dprCap) * dprScale}
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

        {/* 帧预算调节器：尖峰驱动的自动降载/回升 */}
        <FrameGovernor onDown={handleFrameDown} onUp={handleFrameUp} />

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

        {/* 后期处理（按质量分级 + 帧预算自动降级）。
            景深（DoF）是全屏后处理里最贵的一项，只在 ultra 档保留。
            MSAA 固定关闭：后处理链（bloom 的 mipmap 模糊）本身就会
            平滑边缘，多重采样在 composer 之下几乎全是白付的显存与
            resolve 开销。ultra 超预算时 FrameGovernor 关 DoF、再逐级
            降分辨率。 */}
        {sceneSettings.postProcessing && (
          <EffectComposer multisampling={0} enableNormalPass={false}>
            {[
              ...(dofOn ? (
                [<DepthOfField key="dof" focusDistance={0.012} focalLength={0.02} bokehScale={1.5} />]
              ) : []),
              <Bloom
                key="bloom"
                intensity={sceneSettings.quality === 'low' ? 0.2 : sceneSettings.quality === 'mid' ? 0.28 : sceneSettings.quality === 'ultra' ? 0.45 : 0.32}
                luminanceThreshold={sceneSettings.quality === 'low' ? 0.85 : sceneSettings.quality === 'mid' ? 0.78 : sceneSettings.quality === 'ultra' ? 0.45 : 0.75}
                luminanceSmoothing={0.9}
                mipmapBlur
                radius={sceneSettings.quality === 'low' ? 0.35 : sceneSettings.quality === 'mid' ? 0.45 : sceneSettings.quality === 'ultra' ? 0.7 : 0.55}
              />,
              <Vignette key="vignette" eskil={false} offset={0.2} darkness={0.65} />,
              ...(sceneSettings.quality !== 'low' ? (
                [<Noise key="noise" blendFunction={BlendFunction.COLOR_DODGE} opacity={sceneSettings.quality === 'mid' ? 0.02 : 0.03} />]
              ) : []),
            ]}
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
        loadLevel={level}
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

/** 帧预算调节器（尖峰驱动）：统计窗口内的超时帧（帧间隔 > 24ms，
 *  即明显错过 60fps 节拍的长帧——正是"合成器读到空缓冲"的成因）。
 *  - 一个窗口（60 帧 ≈ 1s）内 ≥3 次尖峰 → onDown（降一级）
 *  - 连续 5 个零尖峰窗口（≈5s）→ onUp（父级有 8s 冷却）
 *  为什么不用平均帧时间：60fps 节流下帧间隔恒为 ~16.7ms（含节流空转），
 *  平均值无法区分"从容"与"贴边"——贴边配置会触发升降振荡，而每次
 *  跳变（重建 composer/重设画布）本身就是一次闪屏。 */
function FrameGovernor({ onDown, onUp }: { onDown: () => void; onUp: () => void }) {
  const stat = useRef({ last: 0, n: 0, spikes: 0, clean: 0 })
  useFrame(() => {
    const now = performance.now()
    const s = stat.current
    if (s.last > 0) {
      if (now - s.last > 24) s.spikes += 1
      s.n += 1
      if (s.n >= 60) {
        if (s.spikes >= 3) {
          s.clean = 0
          onDown()
        } else if (s.spikes === 0) {
          s.clean += 1
          if (s.clean >= 5) {
            s.clean = 0
            onUp()
          }
        } else {
          s.clean = 0
        }
        s.n = 0
        s.spikes = 0
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
  // 节拍推拉方向向量：useFrame 每帧跑，复用同一个实例避免 60fps 的 GC 压力
  const beatDirRef = useRef(new THREE.Vector3())
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
          beatDirRef.current.subVectors(camera.position, c.target).normalize()
          camera.position.addScaledVector(beatDirRef.current, -beatPush * 0.15)
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
