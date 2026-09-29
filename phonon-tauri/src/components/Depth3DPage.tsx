// 三维成空 - 沉浸式 3D 音乐可视化
// 场景五层结构：流体地面 + 频谱晶体环 + 能量核心 + 流场星尘 + 氛围雾
// HUD：隐藏式交互，鼠标移动浮现四角按钮，底部 hover 呼出播放栏
import { useRef, Component, Suspense, useState, useCallback, useEffect } from 'react'
import * as THREE from 'three'
import { Canvas, useFrame, useThree } from '@react-three/fiber'
import { OrbitControls, Environment } from '@react-three/drei'
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

// 独立错误边界：隔离 HDR 环境贴图加载失败，避免整棵 Canvas 崩溃
class EnvErrorBoundary extends Component<{ children: React.ReactNode }, { hasError: boolean }> {
  state = { hasError: false }
  static getDerivedStateFromError() { return { hasError: true } }
  componentDidCatch() { /* 静默降级：HDR 加载失败不渲染环境，用现有灯光体系兜底 */ }
  render() {
    if (this.state.hasError) return null
    return this.props.children
  }
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
        dpr={sceneSettings.quality === 'low' ? [1, 1] : sceneSettings.quality === 'mid' ? [1, 1.5] : sceneSettings.quality === 'ultra' ? [1, 2.5] : [1, 2]}
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

        <EnvErrorBoundary>
          <Suspense fallback={null}>
            <Environment
              preset="night"
              files={undefined as any}
              background={false}
              resolution={128}
            />
          </Suspense>
        </EnvErrorBoundary>

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

        {/* 后期处理（按质量分级） */}
        {sceneSettings.postProcessing && (
          <EffectComposer multisampling={sceneSettings.quality === 'low' ? 0 : sceneSettings.quality === 'mid' ? 0 : 2} enableNormalPass={false}>
            <DepthOfField
              focusDistance={0.012}
              focalLength={sceneSettings.quality === 'low' ? 0.01 : sceneSettings.quality === 'mid' ? 0.015 : 0.02}
              bokehScale={sceneSettings.quality === 'low' ? 0.5 : sceneSettings.quality === 'mid' ? 0.9 : sceneSettings.quality === 'ultra' ? 1.5 : 1.2}
            />
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
