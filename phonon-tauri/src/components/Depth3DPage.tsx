// 三维承空 - 全屏 3D 可视化主页面（整合播放器界面 + 维度控制台浮窗）
// 包含：Canvas（R3F）+ OrbitControls（含仰视视角）+ 后处理（Bloom / DOF / Vignette / Noise）
// 精简装配：中心播放器切片 + 频谱柱切片 + 歌词切片 + 地面 + 粒子
// HTML 浮窗：维度控制台（可隐藏、tab 切换：播放概览 / 输出设备 / 播放队列 / DSP）
import { useMemo, useRef, Component, Suspense, useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import * as THREE from 'three'
import { Canvas, useFrame, useThree } from '@react-three/fiber'
import { OrbitControls, Stars, Environment } from '@react-three/drei'
import { EffectComposer, Bloom, DepthOfField, Vignette, Noise } from '@react-three/postprocessing'
import { BlendFunction } from 'postprocessing'
import { useAudioDataRef, lerp } from './depth3d/audioBridge'
import type { SmoothedAudioData } from './depth3d/audioBridge'
import { useProgress, getProgress } from '../api/progressBus'
import { ParticleField, FloorGrid } from './depth3d/ParticleField'

interface Props {
  onClose?: () => void
  playback?: {
    state: string
    position_secs: number
    duration_secs: number | null
    current_track: string | null
  }
  volume?: number
  coverUrl?: string | null
  // 视觉增强联动：来自 App 的维度控制接口
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

type DimTab = 'vitals' | 'overview' | 'lyrics' | 'spectrum' | 'device' | 'queue' | 'dsp'
void (0 as unknown as DimTab)

// ================== 嵌入场景小窗口：统一系统窗口风格（顶部titlebar + 红黄绿圆点 + 玻璃面板 + 标题） ==================
interface SceneWindowProps {
  title: string
  accent?: string
  width?: number
  height?: number
  position: [number, number, number]
  rotation?: [number, number, number]
  children?: React.ReactNode
  showToolbar?: boolean
}

function makeTitleTexture(title: string, accent = '#8ab4ff') {
  const W = 512, H = 58
  const canvas = document.createElement('canvas')
  canvas.width = W; canvas.height = H
  const ctx = canvas.getContext('2d')!
  ctx.clearRect(0, 0, W, H)
  // 渐变底
  const g = ctx.createLinearGradient(0, 0, W, 0)
  g.addColorStop(0, 'rgba(14,18,48,0.9)')
  g.addColorStop(1, 'rgba(24,18,56,0.9)')
  ctx.fillStyle = g
  ctx.fillRect(0, 0, W, H)
  // 分隔阴影线
  ctx.fillStyle = 'rgba(255,255,255,0.06)'
  ctx.fillRect(0, H - 1, W, 1)
  // traffic light 圆点
  const lights = [
    { x: 24, c: '#ff5f57' },
    { x: 52, c: '#ffbd2e' },
    { x: 80, c: '#28c840' },
  ]
  for (const l of lights) {
    ctx.beginPath()
    ctx.arc(l.x, 29, 9, 0, Math.PI * 2)
    ctx.fillStyle = l.c
    ctx.shadowColor = l.c; ctx.shadowBlur = 10
    ctx.fill()
    ctx.shadowBlur = 0
  }
  // 标题
  ctx.font = '700 24px "PingFang SC", "Microsoft YaHei", system-ui'
  ctx.textAlign = 'center'
  ctx.textBaseline = 'middle'
  const titleGrad = ctx.createLinearGradient(0, 0, W, 0)
  titleGrad.addColorStop(0, accent)
  titleGrad.addColorStop(1, '#d7c8ff')
  ctx.fillStyle = titleGrad
  ctx.fillText(title, W / 2, 30)
  const t = new THREE.CanvasTexture(canvas)
  t.anisotropy = 8
  t.needsUpdate = true
  return t
}

function SceneWindow({
  title, accent = '#8ab4ff', width = 4.2, height = 2.6,
  position, rotation = [0, 0, 0], children, showToolbar = true,
}: SceneWindowProps) {
  const titleTex = useMemo(() => makeTitleTexture(title, accent), [title, accent])
  const titleH = 0.36
  const bodyY = -titleH / 2

  return (
    <group position={position} rotation={rotation}>
      {/* 整面板玻璃 */}
      <mesh castShadow receiveShadow>
        <boxGeometry args={[width, height, 0.25]} />
        <meshPhysicalMaterial
          color="#f5f8ff"
          transparent
          opacity={0.72}
          transmission={0.78}
          thickness={0.5}
          roughness={0.16}
          metalness={0.08}
          ior={1.5}
          clearcoat={1}
          clearcoatRoughness={0.06}
          side={THREE.DoubleSide}
          emissive={accent}
          emissiveIntensity={0.06}
        />
      </mesh>
      <lineSegments>
        <edgesGeometry args={[new THREE.BoxGeometry(width, height, 0.28), 20]} />
        <lineBasicMaterial color="#b4d2ff" transparent opacity={0.9} />
      </lineSegments>

      {showToolbar && (
        <>
          {/* titlebar 背景（稍微凸起的深色条） */}
          <mesh position={[0, height / 2 - titleH / 2, 0.14]}>
            <boxGeometry args={[width - 0.08, titleH, 0.025]} />
            <meshStandardMaterial
              map={titleTex}
              transparent
              depthWrite={false}
            />
          </mesh>
          {/* 内容区下方细分割 */}
          <mesh position={[0, height / 2 - titleH - 0.01, 0.15]}>
            <boxGeometry args={[width - 0.24, 0.01, 0.02]} />
            <meshBasicMaterial color={accent} transparent opacity={0.85} />
          </mesh>
        </>
      )}

      {/* children 整体平移到 body 区 */}
      <group position={[0, bodyY, 0.16]}>
        {children}
      </group>
    </group>
  )
}
// 以上组件目前暂不挂在 3D 场景里，保留作为后续扩展锚点
void SceneWindow;

// ================== 嵌入小窗 1-3 已全部迁移至控制台 HTML Tab，以下占位保留以便后续扩展 ==================
function _VitalsWindow(_: any) { return null }
void _VitalsWindow;
function _DeviceWindow(_: any) { return null }
void _DeviceWindow;
function _QueueWindow(_: any) { return null }
void _QueueWindow;

// ================== canvas 2d 小工具（保留作为后续扩展锚点） ==================
function roundRect(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, r: number) {
  const rr = Math.min(r, w / 2, h / 2)
  ctx.beginPath()
  ctx.moveTo(x + rr, y)
  ctx.arcTo(x + w, y, x + w, y + h, rr)
  ctx.arcTo(x + w, y + h, x, y + h, rr)
  ctx.arcTo(x, y + h, x, y, rr)
  ctx.arcTo(x, y, x + w, y, rr)
  ctx.closePath()
}
void roundRect;
function wrapText(ctx: CanvasRenderingContext2D, text: string, x: number, y: number, maxW: number, lineH: number) {
  const chars = text.split(''); let line = ''
  for (let i = 0; i < chars.length; i++) {
    const test = line + chars[i]
    if (ctx.measureText(test).width > maxW && line) {
      ctx.fillText(line, x, y); line = chars[i]; y += lineH
    } else line = test
  }
  if (line) ctx.fillText(line, x, y)
}
void wrapText;

// 通用顶部系统 TitleBar（红黄绿 traffic-light + 标题渐变字，可挂在任意 group 坐标）
function TitleBar({ title, accent = '#8ab4ff', width = 4, y = 0, z = 0 }:
  { title: string; accent?: string; width?: number; y?: number; z?: number }) {
  const tex = useMemo(() => makeTitleTexture(title, accent), [title, accent])
  return (
    <group position={[0, y, z]}>
      <mesh position={[0, 0, 0]}>
        <boxGeometry args={[width, 0.32, 0.025]} />
        <meshStandardMaterial map={tex} transparent depthWrite={false} />
      </mesh>
      <mesh position={[0, -0.17, 0.006]}>
        <boxGeometry args={[width - 0.18, 0.01, 0.02]} />
        <meshBasicMaterial color={accent} transparent opacity={0.9} />
      </mesh>
    </group>
  )
}
// TitleBar 目前保留为扩展锚点
void TitleBar;

// ========== 纯可视化组件（从原切片迁移后，场景内只保留纯视觉，不再渲染任何文字/按钮/控件）==========
// 1) 中心低频能量光晕环：两层 torusKnot + 低频驱动 scale / emissive
function AudioGlowRing({ audioRef }: { audioRef: React.MutableRefObject<SmoothedAudioData> }) {
  const ringRef = useRef<THREE.Mesh>(null)
  const coreRef = useRef<THREE.Mesh>(null)
  const scaleRef = useRef(1)
  useFrame(({ clock }) => {
    const d = audioRef.current
    const t = clock.elapsedTime
    const target = 1 + d.lowFreqAvg * 2.2 + d.onset * 0.6
    scaleRef.current = lerp(scaleRef.current, target, 0.14)
    if (ringRef.current) {
      const s = scaleRef.current
      ringRef.current.scale.setScalar(s)
      ringRef.current.rotation.z = t * 0.35
      ringRef.current.rotation.x = Math.sin(t * 0.2) * 0.3
    }
    if (coreRef.current) {
      const s = 0.6 + lerp(0.6, 1.2, d.rms)
      coreRef.current.scale.setScalar(s)
      const mat = coreRef.current.material as THREE.MeshPhysicalMaterial
      mat.emissiveIntensity = 0.4 + d.rms * 2.4 + d.onset * 0.8
    }
  })
  return (
    <group position={[0, 0.8, 0]}>
      <mesh ref={ringRef} castShadow receiveShadow>
        <torusKnotGeometry args={[1.6, 0.18, 180, 24, 2, 3]} />
        <meshPhysicalMaterial
          color="#eaf0ff"
          transparent
          opacity={0.55}
          transmission={0.66}
          thickness={0.8}
          roughness={0.14}
          metalness={0.12}
          clearcoat={1}
          clearcoatRoughness={0.05}
          emissive="#7aa7ff"
          emissiveIntensity={0.35}
          side={THREE.DoubleSide}
        />
      </mesh>
      <mesh ref={coreRef} castShadow receiveShadow>
        <icosahedronGeometry args={[0.9, 2]} />
        <meshPhysicalMaterial
          color="#f3f7ff"
          transparent
          opacity={0.5}
          transmission={0.85}
          thickness={1.4}
          roughness={0.12}
          metalness={0.08}
          clearcoat={1}
          clearcoatRoughness={0.04}
          emissive="#8ab4ff"
          emissiveIntensity={0.4}
          side={THREE.DoubleSide}
        />
      </mesh>
    </group>
  )
}

// 2) 环绕频谱球：48 颗卫星小球围绕中心公转 + 能量驱动 scale/color，不显示任何文字/控件
function OrbitingSpectrum({ audioRef }: { audioRef: React.MutableRefObject<SmoothedAudioData> }) {
  const groupRef = useRef<THREE.Group>(null)
  const bars = 48
  const refArr = useRef<(THREE.Mesh | null)[]>([])
  useFrame(({ clock }) => {
    const t = clock.elapsedTime
    const d = audioRef.current
    const sp = d.spectrum64
    if (groupRef.current) groupRef.current.rotation.y = t * 0.08
    for (let i = 0; i < bars; i++) {
      const m = refArr.current[i]
      if (!m) continue
      const bin = Math.floor((i / bars) * Math.max(1, sp.length))
      const v = Math.max(0, Math.min(1, sp[bin] ?? 0))
      const r = 3.6 + v * 1.6
      const a = (i / bars) * Math.PI * 2 + t * 0.3
      m.position.set(
        Math.cos(a) * r,
        Math.sin((i / 5 + t * 0.5) * 0.6) * (0.6 + v * 1.2),
        Math.sin(a) * r,
      )
      const s = 0.14 + v * 0.7
      m.scale.setScalar(s)
      const mat = m.material as THREE.MeshStandardMaterial
      const hue = 200 + (i / bars) * 120 + d.lowFreqAvg * 80
      mat.color.setHSL((hue % 360) / 360, 0.72, 0.62)
      mat.emissiveIntensity = 0.25 + v * 1.6
      mat.emissive.setHSL((hue % 360) / 360, 0.85, 0.55)
    }
  })
  const meshes = useMemo(() => {
    const arr: React.ReactElement[] = []
    for (let i = 0; i < bars; i++) {
      const geo = new THREE.IcosahedronGeometry(0.22, 0)
      const hue = 200 + (i / bars) * 120
      arr.push(
        <mesh
          key={i}
          ref={(el) => (refArr.current[i] = el)}
          geometry={geo}
          castShadow
          receiveShadow
        >
          <meshStandardMaterial
            color={`hsl(${hue}, 72%, 62%)`}
            metalness={0.22}
            roughness={0.32}
            emissive={`hsl(${hue}, 85%, 55%)`}
            emissiveIntensity={0.3}
            transparent
            opacity={0.92}
          />
        </mesh>,
      )
    }
    return arr
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])
  return (
    <group position={[0, 0.6, 0]}>
      <group ref={groupRef}>{meshes}</group>
      {/* 外层玻璃球壳（虚）——纯视觉 */}
      <mesh position={[0, 0, 0]}>
        <sphereGeometry args={[5.6, 64, 64]} />
        <meshPhysicalMaterial
          color="#ffffff"
          transparent
          opacity={0.035}
          transmission={0.92}
          thickness={1}
          roughness={0.2}
          metalness={0}
          side={THREE.BackSide}
          emissive="#8ab4ff"
          emissiveIntensity={0.03}
        />
      </mesh>
    </group>
  )
}

export default function Depth3DPage({
  onClose, playback, volume = 0.8, coverUrl = null,
  visActive = true, visMode = 'depth3d',
  onToggleVisActive, onVisModeChange, onEnterDimension,
}: Props) {
  // 保留未使用参数：其他组件目前不需要这些参数，Depth3D 自行管理状态即可。
  void visActive; void visMode; void onToggleVisActive; void onVisModeChange; void onEnterDimension
  const audioRef = useAudioDataRef()
  // 维度控制台：显示/隐藏
  const [panelVisible, setPanelVisible] = useState(true)

  const deviceItems = useMemo(() => ([
    { title: '扬声器 (Realtek Audio)', sub: '2ch · 48kHz · 独占', active: true, accent: '#7fe9a3' },
    { title: '扬声器 (网易虚拟音频设备)', sub: '2ch · 默认采样率', active: false },
    { title: 'USB Audio DAC · 独占', sub: '2ch · 32bit · 192kHz', active: false, accent: '#82cfff' },
    { title: 'AirPods Pro (蓝牙)', sub: '2ch · 44.1kHz', active: false },
  ]), [])

  const playlistItems = useMemo(() => ([
    { title: '01 · Opening Theme', sub: '久石让 · Best Live', active: true, accent: '#ffe08a' },
    { title: '02 · Summer (Piano Solo)', sub: '久石让 — 钢琴故事', active: false },
    { title: '03 · 千と千尋の神隠し', sub: '久石让 · 电影原声', active: false },
    { title: '04 · Merry-Go-Round', sub: 'Joe Hisaishi · DSD 256', active: false },
  ]), [])

  // 从后端获取实际播放队列（用户自己的歌单）
  const [realQueue, setRealQueue] = useState<DimItem[]>([])
  useEffect(() => {
    let disposed = false
    const fetchQueue = async () => {
      try {
        const queue = await invoke<any[]>('get_queue')
        if (disposed || !Array.isArray(queue)) return
        const items: DimItem[] = queue.map((track, i) => {
          const name = typeof track === 'string'
            ? track.replace(/^.*[\\/]/, '').replace(/\.[^/.]+$/, '')
            : (track.title || track.path || `曲目 ${i + 1}`)
          const sub = typeof track === 'object'
            ? (track.artist || track.album || '')
            : ''
          return {
            title: `${String(i + 1).padStart(2, '0')} · ${name}`,
            sub: sub || undefined,
            active: i === 0,
            accent: '#ffe08a',
          }
        })
        if (!disposed && items.length > 0) setRealQueue(items)
      } catch { /* 后端不可用时回退到占位数据 */ }
    }
    fetchQueue()
    const timer = setInterval(fetchQueue, 3000)
    return () => { disposed = true; clearInterval(timer) }
  }, [])

  // DSP 链仅保留实际可用的效果；如后端返回更多可动态扩展
  const dspItems = useMemo(() => ([
    { title: '✦ Parametric EQ', sub: '5 段 · 曲线 A', active: true, accent: '#c79eff' },
  ]), [])

  const currentTrack = playback?.current_track
    ? playback.current_track.replace(/^.*[\\/]/, '').replace(/\.[^.]+$/, '')
    : '未在播放'

  return (
    <div style={{
      position: 'fixed', inset: 0,
      width: '100vw', height: '100vh',
      background: '#000000',
      overflow: 'hidden', zIndex: 9999,
    }}>
      {/* 顶部栏 + 返回按钮 + 维度控制台显示/隐藏切换 */}
      <div style={{
        position: 'absolute', top: 0, left: 0, right: 0, zIndex: 10,
        padding: '14px 22px',
        display: 'flex', justifyContent: 'space-between', alignItems: 'center',
        background: 'linear-gradient(180deg, rgba(5,8,25,0.62), transparent)',
        pointerEvents: 'none',
      }}>
        <div style={{
          pointerEvents: 'auto',
          display: 'flex', alignItems: 'center', gap: 12,
        }}>
          <button
            onClick={onClose}
            title="退出维度"
            style={{
              width: 40, height: 40, borderRadius: 12,
              border: '1px solid rgba(160,200,255,0.28)',
              background: 'rgba(15,18,45,0.55)',
              color: '#e5efff',
              fontSize: 'var(--text-xl)', cursor: 'pointer',
              backdropFilter: 'blur(8px)',
            }}
          >←</button>
        </div>
        <div style={{ pointerEvents: 'auto', display: 'flex', gap: 8, alignItems: 'center' }}>
          <button
            title={panelVisible ? '隐藏维度控制台' : '显示维度控制台'}
            onClick={() => setPanelVisible(v => !v)}
            style={{
              padding: '6px 12px', borderRadius: 10,
              border: '1px solid rgba(120,160,255,0.28)',
              background: panelVisible
                ? 'linear-gradient(135deg, rgba(99,102,241,0.28), rgba(6,182,212,0.2))'
                : 'rgba(15,18,45,0.55)',
              color: '#e5efff',
              fontSize: 'var(--text-xs)', cursor: 'pointer',
              backdropFilter: 'blur(8px)',
              font: '600 11px "PingFang SC", system-ui',
              letterSpacing: 0.5,
            }}
          >{panelVisible ? '✦ 控制台 · 开' : '✦ 控制台 · 关'}</button>
        </div>
      </div>

      {/* 画布：纯黑宇宙 · 更大视域 · 完整 360° 球形空间 */}
      <Canvas
        shadows
        dpr={[1, 2.5]}
        gl={{ antialias: true, alpha: false, powerPreference: 'high-performance', stencil: false }}
        camera={{ position: [0, 0, 14], fov: 55, near: 0.08, far: 3000 }}
        style={{ position: 'absolute', inset: 0, zIndex: 0 }}
      >
        <color attach="background" args={['#000000']} />
        <fog attach="fog" args={['#000000', 150, 2000]} />

        <CameraRig audioRef={audioRef} />

        {/* 灯光：宇宙低调发光体系，仅照射主体，远处全黑 */}
        <ambientLight intensity={0.22} />
        <directionalLight
          position={[14, 22, 14]}
          intensity={0.82}
          color={'#e6efff'}
          castShadow
          shadow-mapSize-width={2048}
          shadow-mapSize-height={2048}
          shadow-camera-near={0.1}
          shadow-camera-far={80}
        />
        <pointLight position={[0, 6, 4]} intensity={0.95} color={'#7a9dff'} distance={80} decay={2} />
        <pointLight position={[-14, -2, -6]} intensity={0.42} color={'#b992ff'} distance={120} decay={2} />
        <pointLight position={[14, 2, -4]} intensity={0.42} color={'#56c8ff'} distance={120} decay={2} />
        <spotLight
          position={[0, 22, 0]}
          angle={0.9}
          penumbra={0.9}
          intensity={0.75}
          color={'#eef3ff'}
          castShadow
          shadow-mapSize-width={2048}
          shadow-mapSize-height={2048}
        />

        <Stars radius={350} depth={220} count={14000} factor={5} saturation={0.55} fade speed={0.18} />
        <EnvErrorBoundary>
          <Suspense fallback={null}>
            {/* HDR 环境贴图：本地缺文件时 preset 降级，避免 404 与未捕获 onError 抛错 */}
            <Environment
              preset="night"
              files={undefined as any}
              background={false}
              resolution={256}
            />
          </Suspense>
        </EnvErrorBoundary>

        {/* 场景内只保留纯可视化效果：切片内容统一迁移到右侧 HTML 维度控制台，避免场景杂乱 */}
        {/* 音频驱动能量可视化：中心低频光晕环 */}
        <AudioGlowRing audioRef={audioRef} />
        {/* 环绕频谱球（纯可视化，纯形状/颜色，不显示文字/控件） */}
        <OrbitingSpectrum audioRef={audioRef} />

        <FloorGrid audioRef={audioRef} />
        <ParticleField audioRef={audioRef} />

        {/* OrbitControls：完整球形空间，0.01~(PI-0.08) 可几乎正上仰视 & 正下俯视，距离范围很大 */}
        <OrbitControls
          enablePan={false}
          enableDamping
          dampingFactor={0.06}
          minDistance={2}
          maxDistance={160}
          maxPolarAngle={Math.PI - 0.08}
          minPolarAngle={0.01}
          makeDefault
        />

        {/* 后期链：关闭 multisampling 避免 glBlitFramebuffer Read/Write 同源 depth-stencil 冲突；DOF bokehScale 略降 */}
        <EffectComposer multisampling={0} enableNormalPass={false}>
          <DepthOfField
            focusDistance={0.0008}
            focalLength={0.012}
            bokehScale={1.2}
          />
          <Bloom
            intensity={0.55}
            luminanceThreshold={0.32}
            luminanceSmoothing={0.9}
            mipmapBlur
            radius={0.6}
          />
          <Vignette eskil={false} offset={0.28} darkness={0.75} />
          <Noise blendFunction={BlendFunction.COLOR_DODGE} opacity={0.04} />
        </EffectComposer>
      </Canvas>

      {/* 维度控制台浮窗（可隐藏）：整合原场景内 6 处切片全部迁入 + 模块式 Windows 四格首页 */}
      {panelVisible && (
        <DimensionConsole
          onClose={() => setPanelVisible(false)}
          onExit3D={onClose}
          playback={playback}
          volume={volume}
          coverUrl={coverUrl}
          audioRef={audioRef}
          deviceItems={deviceItems}
          playlistItems={realQueue.length > 0 ? realQueue : playlistItems}
          realQueueCount={realQueue.length}
          dspItems={dspItems}
          currentTrack={currentTrack}
        />
      )}
    </div>
  )
}

// ========== 相机 rig（不再覆盖 OrbitControls 俯仰，仅做镜头 shake + FOV 呼吸 + target 轻微浮动）==========
function CameraRig({ audioRef }: { audioRef: React.MutableRefObject<SmoothedAudioData> }) {
  const { camera, controls } = useThree()
  const fovRef = useRef(55)
  const shakeRef = useRef({ x: 0, y: 0, z: 0 })
  const targetBaseRef = useRef(new THREE.Vector3(0, 1, 0))
  const targetShakeRef = useRef(new THREE.Vector3())

  useFrame(({ clock }) => {
    const d = audioRef.current
    // FOV 呼吸：低频推 fov 稍大，但不超过 60° 避免太鱼眼
    const target = 55 + d.rms * 10
    fovRef.current = lerp(fovRef.current, target, 0.12)
    const persp = camera as THREE.PerspectiveCamera
    if (typeof persp.fov === 'number') {
      persp.fov = fovRef.current
      persp.updateProjectionMatrix()
    }
    // onset 镜头微抖（只叠加，不重置 position，避免覆盖 OrbitControls 的俯仰距离）
    const shakeMag = d.onset * 0.08
    shakeRef.current.x = lerp(shakeRef.current.x, (Math.random() - 0.5) * shakeMag, 0.28)
    shakeRef.current.y = lerp(shakeRef.current.y, (Math.random() - 0.5) * shakeMag, 0.28)
    camera.position.x += shakeRef.current.x
    camera.position.y += shakeRef.current.y
    // orbit target 轻微呼吸（低频整体轻微前移/下沉，增强代入感）
    targetShakeRef.current.x = lerp(targetShakeRef.current.x, 0, 0.08)
    targetShakeRef.current.y = lerp(targetShakeRef.current.y, Math.sin(clock.elapsedTime * 0.25) * 0.06 - d.lowFreqAvg * 0.05, 0.08)
    targetShakeRef.current.z = lerp(targetShakeRef.current.z, -d.lowFreqAvg * 0.12, 0.08)
    if (controls) {
      const c = controls as any
      if (c.target) {
        c.target.copy(targetBaseRef.current).add(targetShakeRef.current)
        if (typeof c.update === 'function') c.update()
      }
    }
  })
  return null
}

// ========== 维度控制台浮窗（整合原场景内全部切片：监控/播放/歌词/频谱/设备/队列/DSP 7 Tab）==========
interface DimItem { title: string; sub?: string; active?: boolean; accent?: string }
function DimensionConsole({
  onClose, onExit3D,
  playback, volume, currentTrack, coverUrl,
  audioRef,
  deviceItems, playlistItems, dspItems,
  realQueueCount,
}: {
  onClose: () => void
  onExit3D?: () => void
  playback?: Props['playback']
  volume: number
  currentTrack: string
  coverUrl?: string | null
  audioRef: React.MutableRefObject<SmoothedAudioData>
  deviceItems: DimItem[]
  playlistItems: DimItem[]
  dspItems: DimItem[]
  realQueueCount: number
}) {
  const cur = playback
  // Position streams through the progress bus — self-subscribe so the
  // console keeps updating even though App state no longer carries it at
  // ~20 Hz.
  const liveProgress = useProgress()
  const pos = liveProgress.position
  const dur = liveProgress.duration ?? cur?.duration_secs ?? null
  const progress = dur ? Math.max(0, Math.min(1, pos / dur)) : 0
  const fmt = (s: number | null | undefined) => {
    if (s == null) return '00:00'
    const m = Math.floor(s / 60)
    const sec = Math.floor(s % 60)
    return `${String(m).padStart(2, '0')}:${String(sec).padStart(2, '0')}`
  }

  // 四格模块：home = Windows 四格首页；其他 = 进入单模块详情
  type ModuleKey = 'home' | 'playback' | 'device' | 'extensions' | 'visualization'
  const [module, setModule] = useState<ModuleKey>('home')
  const [showPlaylist, setShowPlaylist] = useState(false)
  const [showVolumeSlider, setShowVolumeSlider] = useState(false)
  const [volumeLevel, setVolumeLevel] = useState(volume)
  // 音量滑块：拖动支持
  const volDragRef = useRef<HTMLDivElement | null>(null)
  const volDraggingRef = useRef(false)
  const syncVolumeFromClientY = (clientY: number) => {
    const el = volDragRef.current
    if (!el) return
    const rect = el.getBoundingClientRect()
    const ratio = 1 - Math.max(0, Math.min(1, (clientY - rect.top) / rect.height))
    const v = Math.round(ratio * 100)
    setVolumeLevel(v)
    invoke('set_volume', { level: v / 100 }).catch(() => {})
  }
  useEffect(() => {
    const onMove = (e: MouseEvent) => {
      if (!volDraggingRef.current) return
      syncVolumeFromClientY(e.clientY)
    }
    const onUp = () => { volDraggingRef.current = false }
    document.addEventListener('mousemove', onMove)
    document.addEventListener('mouseup', onUp)
    return () => {
      document.removeEventListener('mousemove', onMove)
      document.removeEventListener('mouseup', onUp)
    }
  }, [])
  // 设备模块子导航：list = 双栏列表；device-detail / dsp-detail = 选中项详情
  const [deviceSub, setDeviceSub] = useState<'list' | 'device-detail' | 'dsp-detail'>('list')
  const [selectedDevice, setSelectedDevice] = useState<number>(-1)
  const [selectedDsp, setSelectedDsp] = useState<number>(-1)

  const PlaybackIcon = (
    <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><polygon points="6 4 20 12 6 20 6 4" /></svg>
  )
  const DeviceIcon = (
    <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d="M12 1v22 M5 5H4a2 2 0 0 0-2 2v10a2 2 0 0 0 2 2h1 M20 5h1a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2h-1 M8 9v6 M16 9v6 M11 18a1 1 0 1 0 2 0 1 1 0 0 0-2 0Z" /></svg>
  )
  const SettingsIcon = (
    <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d="M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6Z" /><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1Z" /></svg>
  )
  const VisualIcon = (
    <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d="M3 18V12 M7 18V7 M11 18V4 M15 18V9 M19 18V14" /></svg>
  )
  const BackIcon = (
    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round"><path d="M15 18l-6-6 6-6" /></svg>
  )
  const QueueIcon = (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><line x1="8" y1="6" x2="21" y2="6" /><line x1="8" y1="12" x2="21" y2="12" /><line x1="8" y1="18" x2="21" y2="18" /><line x1="3" y1="6" x2="3.01" y2="6" /><line x1="3" y1="12" x2="3.01" y2="12" /><line x1="3" y1="18" x2="3.01" y2="18" /></svg>
  )

  return (
    <div style={{
      position: 'absolute',
      top: 78,
      right: 22,
      zIndex: 20,
      color: '#dce8ff',
      pointerEvents: 'auto',
      display: 'flex',
      alignItems: 'flex-start',
      gap: 10,
    }}>
      {/* 左侧外侧：隐藏式播放队列按钮 + 向右自适应展开浮窗 */}
      <div style={{
        position: 'relative',
        marginTop: 8,
        display: 'flex',
        alignItems: 'flex-start',
      }}>
        <button
          onClick={() => { setShowPlaylist(v => !v); setShowVolumeSlider(false) }}
          title={showPlaylist ? `收起播放队列（${realQueueCount || 0} 首）` : `打开播放队列（${realQueueCount || 0} 首）`}
          style={{
            width: 40, height: 40, borderRadius: 14,
            border: showPlaylist
              ? '1px solid rgba(255,224,138,0.55)'
              : '1px solid rgba(120,160,255,0.28)',
            background: showPlaylist
              ? 'linear-gradient(135deg, rgba(255,224,138,0.24), rgba(251,191,36,0.14))'
              : 'rgba(12,14,36,0.55)',
            color: showPlaylist ? '#ffe08a' : '#c6d4ff',
            cursor: 'pointer',
            display: 'inline-flex', alignItems: 'center', justifyContent: 'center',
            position: 'relative',
            backdropFilter: 'blur(10px) saturate(150%)',
            boxShadow: showPlaylist ? '0 6px 18px rgba(0,0,0,0.45)' : '0 6px 16px rgba(0,0,0,0.4)',
          }}
        >
          {QueueIcon}
          {realQueueCount > 0 && (
            <span style={{
              position: 'absolute',
              top: -4, right: -4,
              minWidth: 16, height: 16, padding: '0 4px',
              borderRadius: 999,
              background: 'linear-gradient(135deg, #f59e0b, #ef4444)',
              color: '#fff',
              font: '700 9px ui-monospace, monospace',
              display: 'inline-flex', alignItems: 'center', justifyContent: 'center',
              border: '1px solid rgba(0,0,0,0.3)',
            }}>{realQueueCount > 99 ? '99+' : realQueueCount}</span>
          )}
        </button>

        {/* 队列弹出面板：从按钮右侧向左？→ 不对，按钮在控制台左边，向右展开到控制台前方（紧贴右侧），
            宽度/高度按内容自适应（min~max），点击后整体变大。 */}
        {showPlaylist && (
          <div style={{
            position: 'absolute',
            top: 0,
            left: 'calc(100% + 10px)',
            // 自适应：最小 ~ 最大尺寸，内容撑开
            minWidth: 280,
            width: 'auto',
            maxWidth: 420,
            minHeight: 180,
            maxHeight: 'calc(100vh - 130px)',
            borderRadius: 16,
            background: 'linear-gradient(180deg, rgba(16,19,48,0.95), rgba(8,10,28,0.97))',
            border: '1px solid rgba(255,224,138,0.32)',
            backdropFilter: 'blur(14px) saturate(150%)',
            boxShadow: '0 18px 44px rgba(0,0,0,0.55), 0 0 34px rgba(255,224,138,0.08)',
            display: 'flex', flexDirection: 'column',
            overflow: 'hidden',
            zIndex: 30,
            // 弹出过渡（自适应展开）
            transformOrigin: '0% 0%',
            animation: 'queuePanelIn 0.22s cubic-bezier(0.22, 1, 0.36, 1)',
          }}>
            <style>{`
              @keyframes queuePanelIn {
                from { opacity: 0; transform: scale(0.94); }
                to   { opacity: 1; transform: scale(1); }
              }
            `}</style>
            <div style={{
              padding: '10px 14px',
              display: 'flex', alignItems: 'center', justifyContent: 'space-between',
              borderBottom: '1px solid rgba(120,160,255,0.18)',
              flexShrink: 0,
            }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                <span style={{ color: '#ffe08a' }}>{QueueIcon}</span>
                <span style={{
                  font: '700 12px "PingFang SC", system-ui',
                  color: '#eef4ff',
                  letterSpacing: 0.4,
                }}>播放队列</span>
                <span style={{
                  fontSize: 'var(--text-xs)', padding: '1px 7px', borderRadius: 999,
                  background: 'rgba(255,224,138,0.12)',
                  border: '1px solid rgba(255,224,138,0.28)',
                  color: 'rgba(255,224,138,0.9)',
                  font: '600 10px ui-monospace, monospace',
                }}>{realQueueCount} 首</span>
              </div>
              <button
                onClick={() => setShowPlaylist(false)}
                title="收起"
                style={{
                  width: 22, height: 22, borderRadius: 999,
                  border: 'none',
                  background: 'rgba(255,255,255,0.05)',
                  color: 'rgba(180,200,240,0.7)',
                  fontSize: 'var(--text-sm)', cursor: 'pointer',
                  display: 'inline-flex', alignItems: 'center', justifyContent: 'center',
                }}
              >×</button>
            </div>
            <div style={{
              padding: 10,
              overflow: 'auto',
              flex: 1,
              minHeight: 120,
            }}>
              {playlistItems.length > 0 ? (
                <ItemList items={playlistItems} accent="#ffe08a" />
              ) : (
                <div style={{
                  padding: '24px 12px', textAlign: 'center',
                  color: 'rgba(170,190,230,0.5)', fontSize: 'var(--text-xs)',
                }}>播放队列为空，请从歌单添加文件</div>
              )}
            </div>
          </div>
        )}
      </div>

      <div style={{
        width: 360,
        maxHeight: 'calc(100vh - 110px)',
        overflow: 'auto',
        display: 'flex', flexDirection: 'column', gap: 8,
      }}>

      <div style={{
        position: 'relative',
        borderRadius: 16,
        background: 'linear-gradient(180deg, rgba(16,19,48,0.82), rgba(8,10,28,0.88))',
        border: '1px solid rgba(130,170,255,0.3)',
        backdropFilter: 'blur(14px) saturate(150%)',
        boxShadow: '0 16px 44px rgba(0,0,0,0.55), 0 0 40px rgba(96,165,250,0.08)',
        overflow: 'hidden',
      }}>
        {/* 顶部光泽 */}
        <div style={{
          position: 'absolute', inset: 0,
          background: 'radial-gradient(ellipse at 0% 0%, rgba(139,180,255,0.18), transparent 55%), radial-gradient(ellipse at 100% 100%, rgba(196,166,255,0.16), transparent 55%)',
          pointerEvents: 'none',
        }} />

        {/* 头：标题 + 关闭按钮 */}
        <div style={{
          position: 'relative', padding: '8px 16px 10px',
          display: 'flex', alignItems: 'center', justifyContent: 'space-between',
          borderBottom: '1px solid rgba(120,160,255,0.18)',
        }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
            {module !== 'home' && (
              <button
                onClick={() => { setModule('home'); setDeviceSub('list') }}
                title="返回模块首页"
                style={{
                  width: 28, height: 28, borderRadius: 8,
                  border: '1px solid rgba(120,160,255,0.22)',
                  background: 'rgba(255,255,255,0.04)',
                  color: '#c6d4ff', fontSize: 'var(--text-base)', cursor: 'pointer',
                  display: 'inline-flex', alignItems: 'center', justifyContent: 'center',
                }}
              >{BackIcon}</button>
            )}
            <div>
              <div style={{ font: '700 14px "PingFang SC", system-ui', letterSpacing: 0.6, display: 'flex', alignItems: 'center', gap: 8 }}>
                <span style={{
                  background: 'linear-gradient(135deg, #8ab4ff, #c4a6ff)',
                  WebkitBackgroundClip: 'text', backgroundClip: 'text',
                  color: 'transparent',
                }}>维度控制台</span>
              </div>
            </div>
          </div>
          <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
            <button
              onClick={onClose}
              title="隐藏维度控制台"
              style={{
                width: 28, height: 28, borderRadius: 8,
                border: '1px solid rgba(120,160,255,0.22)',
                background: 'rgba(255,255,255,0.04)',
                color: '#c6d4ff',
                fontSize: 'var(--text-base)', cursor: 'pointer',
                display: 'inline-flex', alignItems: 'center', justifyContent: 'center',
              }}
            >−</button>
          </div>
        </div>

        {/* 主体：模块内容区（四格首页 or 单模块详情） */}
        <div style={{ position: 'relative', padding: '12px 14px 14px', minHeight: 260 }}>
          {module === 'home' && (
            <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 12 }}>
              {/* 播放控制 */}
              <button
                onClick={() => setModule('playback')}
                style={{
                  display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center',
                  width: '100%', height: 120,
                  borderRadius: 14,
                  background: 'linear-gradient(135deg, rgba(99,102,241,0.16), rgba(6,182,212,0.09))',
                  border: '1px solid rgba(120,160,255,0.22)',
                  color: '#eaf1ff',
                  cursor: 'pointer',
                  gap: 10,
                  transition: 'transform 0.15s, box-shadow 0.15s',
                }}
                onMouseEnter={(e) => { (e.currentTarget as HTMLButtonElement).style.transform = 'translateY(-2px)'; (e.currentTarget as HTMLButtonElement).style.boxShadow = '0 10px 26px rgba(0,0,0,0.45)' }}
                onMouseLeave={(e) => { (e.currentTarget as HTMLButtonElement).style.transform = ''; (e.currentTarget as HTMLButtonElement).style.boxShadow = '' }}
              >
                <div style={{ color: '#a7c5ff' }}>{PlaybackIcon}</div>
                <div style={{ fontSize: 'var(--text-sm)', fontWeight: 700, letterSpacing: 0.5 }}>播放控制</div>
                <div style={{ fontSize: 'var(--text-xs)', color: 'rgba(170,190,230,0.65)' }}>概览 · 队列 · 状态</div>
              </button>

              {/* 设备 DSP */}
              <button
                onClick={() => setModule('device')}
                style={{
                  display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center',
                  width: '100%', height: 120,
                  borderRadius: 14,
                  background: 'linear-gradient(135deg, rgba(34,211,238,0.14), rgba(127,233,163,0.09))',
                  border: '1px solid rgba(34,211,238,0.22)',
                  color: '#eaf1ff',
                  cursor: 'pointer',
                  gap: 10,
                  transition: 'transform 0.15s, box-shadow 0.15s',
                }}
                onMouseEnter={(e) => { (e.currentTarget as HTMLButtonElement).style.transform = 'translateY(-2px)'; (e.currentTarget as HTMLButtonElement).style.boxShadow = '0 10px 26px rgba(0,0,0,0.45)' }}
                onMouseLeave={(e) => { (e.currentTarget as HTMLButtonElement).style.transform = ''; (e.currentTarget as HTMLButtonElement).style.boxShadow = '' }}
              >
                <div style={{ color: '#7fe9a3' }}>{DeviceIcon}</div>
                <div style={{ fontSize: 'var(--text-sm)', fontWeight: 700, letterSpacing: 0.5 }}>设备 DSP</div>
                <div style={{ fontSize: 'var(--text-xs)', color: 'rgba(170,190,230,0.65)' }}>输出设备 · DSP 链</div>
              </button>

              {/* 可视化 */}
              <button
                onClick={() => setModule('visualization')}
                style={{
                  display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center',
                  width: '100%', height: 120,
                  borderRadius: 14,
                  background: 'linear-gradient(135deg, rgba(167,139,250,0.14), rgba(129,140,248,0.1))',
                  border: '1px solid rgba(167,139,250,0.22)',
                  color: '#eaf1ff',
                  cursor: 'pointer',
                  gap: 10,
                  transition: 'transform 0.15s, box-shadow 0.15s',
                }}
                onMouseEnter={(e) => { (e.currentTarget as HTMLButtonElement).style.transform = 'translateY(-2px)'; (e.currentTarget as HTMLButtonElement).style.boxShadow = '0 10px 26px rgba(0,0,0,0.45)' }}
                onMouseLeave={(e) => { (e.currentTarget as HTMLButtonElement).style.transform = ''; (e.currentTarget as HTMLButtonElement).style.boxShadow = '' }}
              >
                <div style={{ color: '#c4a6ff' }}>{VisualIcon}</div>
                <div style={{ fontSize: 'var(--text-sm)', fontWeight: 700, letterSpacing: 0.5 }}>可视化</div>
                <div style={{ fontSize: 'var(--text-xs)', color: 'rgba(170,190,230,0.65)' }}>歌词 · 频谱</div>
              </button>

              {/* 扩展设置 */}
              <button
                onClick={() => setModule('extensions')}
                style={{
                  display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center',
                  width: '100%', height: 120,
                  borderRadius: 14,
                  background: 'linear-gradient(135deg, rgba(251,191,36,0.14), rgba(251,113,133,0.1))',
                  border: '1px solid rgba(251,191,36,0.22)',
                  color: '#eaf1ff',
                  cursor: 'pointer',
                  gap: 10,
                  transition: 'transform 0.15s, box-shadow 0.15s',
                }}
                onMouseEnter={(e) => { (e.currentTarget as HTMLButtonElement).style.transform = 'translateY(-2px)'; (e.currentTarget as HTMLButtonElement).style.boxShadow = '0 10px 26px rgba(0,0,0,0.45)' }}
                onMouseLeave={(e) => { (e.currentTarget as HTMLButtonElement).style.transform = ''; (e.currentTarget as HTMLButtonElement).style.boxShadow = '' }}
              >
                <div style={{ color: '#ffe08a' }}>{SettingsIcon}</div>
                <div style={{ fontSize: 'var(--text-sm)', fontWeight: 700, letterSpacing: 0.5 }}>扩展设置</div>
                <div style={{ fontSize: 'var(--text-xs)', color: 'rgba(170,190,230,0.65)' }}>监控 · 增强 · 退出</div>
              </button>
            </div>
          )}

          {module === 'playback' && (
            <div style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
              {/* 当前曲目 — 左封面 + 右基本信息，进度条不可拖动 */}
              <div>
                <SectionTitle title="当前曲目" />
                <div style={{
                  borderRadius: 12,
                  padding: 12,
                  border: '1px solid rgba(120,160,255,0.22)',
                  background: 'linear-gradient(135deg, rgba(99,102,241,0.14), rgba(6,182,212,0.08))',
                  display: 'flex',
                  gap: 12,
                  alignItems: 'stretch',
                }}>
                  {/* 专辑封面 */}
                  <div style={{
                    width: 72, height: 72, flexShrink: 0,
                    borderRadius: 10,
                    overflow: 'hidden',
                    border: '1px solid rgba(120,160,255,0.25)',
                    background: 'linear-gradient(135deg, rgba(99,102,241,0.25), rgba(6,182,212,0.2))',
                    display: 'flex', alignItems: 'center', justifyContent: 'center',
                    boxShadow: '0 6px 16px rgba(0,0,0,0.35)',
                  }}>
                    {coverUrl ? (
                      <img
                        src={coverUrl}
                        alt="专辑封面"
                        style={{ width: '100%', height: '100%', objectFit: 'cover', display: 'block' }}
                      />
                    ) : (
                      <span style={{ fontSize: 28, opacity: 0.6 }}>💿</span>
                    )}
                  </div>

                  {/* 基本信息 + 进度 */}
                  <div style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column', justifyContent: 'space-between' }}>
                    <div style={{ minWidth: 0 }}>
                      {/* 标题 + 艺人（从 currentTrack 解析：支持 "title - artist" 或 "artist - title"）*/}
                      {(() => {
                        const raw = currentTrack || '未在播放'
                        let title = raw
                        let artist: string | null = null
                        if (raw.includes(' - ')) {
                          const parts = raw.split(' - ')
                          title = parts.slice(1).join(' - ').trim() || parts[0].trim()
                          artist = parts[0].trim()
                        } else if (raw.includes('·')) {
                          const parts = raw.split('·')
                          title = (parts[1] || parts[0]).trim()
                          artist = parts[0].trim()
                        }
                        return (
                          <>
                            <div style={{
                              font: '700 13px "PingFang SC", system-ui',
                              color: '#eef4ff',
                              whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
                              lineHeight: 1.4,
                            }} title={raw}>{title}</div>
                            {artist && (
                              <div style={{
                                marginTop: 2,
                                fontSize: 'var(--text-xs)', color: 'rgba(180,200,240,0.78)',
                                whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
                              }}>{artist}</div>
                            )}
                            {/* 格式信息 */}
                            <div style={{
                              marginTop: 6,
                              display: 'flex', alignItems: 'center', gap: 6, flexWrap: 'wrap',
                            }}>
                              <span style={{
                                fontSize: 'var(--text-xs)', padding: '2px 6px', borderRadius: 999,
                                background: 'rgba(99,102,241,0.14)',
                                border: '1px solid rgba(99,102,241,0.28)',
                                color: '#a7c5ff',
                                font: '600 9px ui-monospace, monospace',
                                letterSpacing: 0.3,
                              }}>FLAC</span>
                              <span style={{
                                fontSize: 'var(--text-xs)', padding: '2px 6px', borderRadius: 999,
                                background: 'rgba(6,182,212,0.12)',
                                border: '1px solid rgba(6,182,212,0.26)',
                                color: '#7fe9ff',
                                font: '600 9px ui-monospace, monospace',
                              }}>24bit · 96kHz</span>
                              <span style={{
                                fontSize: 'var(--text-xs)',
                                color: playback?.state === 'Playing' ? 'rgba(130,255,170,0.85)' : 'rgba(180,200,240,0.7)',
                                font: '600 10px "PingFang SC", system-ui',
                              }}>
                                {playback?.state === 'Playing' ? '▶ 播放中' : (playback?.state === 'Paused' ? '⏸ 已暂停' : '— 空闲')}
                              </span>
                            </div>
                          </>
                        )
                      })()}
                    </div>

                    {/* 进度 — 纯显示，不可拖动 */}
                    <div>
                      <div style={{
                        height: 5, borderRadius: 999,
                        background: 'rgba(120,160,255,0.1)',
                        overflow: 'hidden',
                        cursor: 'default',
                        userSelect: 'none',
                      }}>
                        <div style={{
                          width: `${progress * 100}%`, height: '100%',
                          background: 'linear-gradient(90deg, #60a5fa, #a78bfa)',
                          boxShadow: '0 0 10px rgba(96,165,250,0.55)',
                        }} />
                      </div>
                      <div style={{
                        display: 'flex', justifyContent: 'space-between',
                        fontSize: 'var(--text-xs)', color: 'rgba(180,200,240,0.72)', marginTop: 5,
                        font: '600 10px ui-monospace, monospace',
                      }}>
                        <span>{fmt(pos)}</span>
                        <span>{fmt(dur)}</span>
                      </div>
                    </div>
                  </div>
                </div>
              </div>

              {/* 播放控制按钮：上一首 / 播放 / 下一首 / 音量 / 播放队列（移到紫色位置，小图标按钮） */}
              <div style={{ position: 'relative', display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 12 }}>
                <button
                  onClick={() => invoke('previous').catch(() => {})}
                  title="上一首"
                  style={{
                    width: 40, height: 40, borderRadius: 999,
                    border: '1px solid rgba(120,160,255,0.25)',
                    background: 'rgba(255,255,255,0.05)',
                    color: '#c6d4ff', fontSize: 'var(--text-lg)', cursor: 'pointer',
                    display: 'inline-flex', alignItems: 'center', justifyContent: 'center',
                  }}
                >⏮</button>
                <button
                  onClick={() => {
                    const isPlaying = playback?.state === 'Playing'
                    invoke(isPlaying ? 'pause' : 'play').catch(() => {})
                  }}
                  title={playback?.state === 'Playing' ? '暂停' : '播放'}
                  style={{
                    width: 52, height: 52, borderRadius: 999,
                    border: '1px solid rgba(99,102,241,0.45)',
                    background: 'linear-gradient(135deg, rgba(99,102,241,0.3), rgba(6,182,212,0.2))',
                    color: '#eaf1ff', fontSize: 'var(--text-xl)', cursor: 'pointer',
                    display: 'inline-flex', alignItems: 'center', justifyContent: 'center',
                    boxShadow: '0 4px 16px rgba(99,102,241,0.3)',
                  }}
                >{playback?.state === 'Playing' ? '⏸' : '▶'}</button>
                <button
                  onClick={() => invoke('next').catch(() => {})}
                  title="下一首"
                  style={{
                    width: 40, height: 40, borderRadius: 999,
                    border: '1px solid rgba(120,160,255,0.25)',
                    background: 'rgba(255,255,255,0.05)',
                    color: '#c6d4ff', fontSize: 'var(--text-lg)', cursor: 'pointer',
                    display: 'inline-flex', alignItems: 'center', justifyContent: 'center',
                  }}
                >⏭</button>
                {/* 音量键 - 下一首旁 */}
                <button
                  onClick={() => { setShowVolumeSlider(v => !v); setShowPlaylist(false) }}
                  title="音量"
                  style={{
                    width: 40, height: 40, borderRadius: 999,
                    border: showVolumeSlider
                      ? '1px solid rgba(99,102,241,0.55)'
                      : '1px solid rgba(120,160,255,0.25)',
                    background: showVolumeSlider
                      ? 'linear-gradient(135deg, rgba(99,102,241,0.25), rgba(6,182,212,0.15))'
                      : 'rgba(255,255,255,0.05)',
                    color: showVolumeSlider ? '#a7c5ff' : '#c6d4ff',
                    fontSize: 'var(--text-lg)', cursor: 'pointer',
                    display: 'inline-flex', alignItems: 'center', justifyContent: 'center',
                  }}
                >{volumeLevel === 0 ? '🔇' : (volumeLevel < 50 ? '🔉' : '🔊')}</button>
                {/* 播放队列隐藏按钮 — 放到控制台左侧外侧（DimensionConsole 外层左侧），这里保留占位避免破坏 flex 间距 */}
                {/* 队列交互请见控制台外层左侧 QueueFloatButton */}

                {/* 竖直音量条弹出（支持鼠标拖动） */}
                {showVolumeSlider && (
                  <div style={{
                    position: 'absolute',
                    bottom: 'calc(100% + 8px)',
                    right: 52,
                    width: 52, height: 180,
                    borderRadius: 14,
                    background: 'linear-gradient(180deg, rgba(16,19,48,0.92), rgba(8,10,28,0.95))',
                    border: '1px solid rgba(130,170,255,0.3)',
                    backdropFilter: 'blur(14px) saturate(150%)',
                    boxShadow: '0 12px 32px rgba(0,0,0,0.5)',
                    display: 'flex', flexDirection: 'column', alignItems: 'center',
                    padding: '10px 0 8px',
                    zIndex: 25,
                  }}>
                    <span style={{
                      fontSize: 'var(--text-xs)', color: 'rgba(180,200,240,0.85)',
                      font: '600 10px ui-monospace, monospace',
                      marginBottom: 6,
                    }}>{Math.round(volumeLevel)}%</span>
                    <div style={{
                      position: 'relative', flex: 1, width: 28,
                      display: 'flex', justifyContent: 'center',
                    }}>
                      <div
                        ref={volDragRef}
                        onClick={(e) => syncVolumeFromClientY(e.clientY)}
                        onMouseDown={(e) => {
                          volDraggingRef.current = true
                          syncVolumeFromClientY(e.clientY)
                        }}
                        style={{
                          position: 'absolute', inset: 0,
                          width: 28, cursor: 'ns-resize',
                          display: 'flex', justifyContent: 'center',
                        }}
                      >
                        <div style={{
                          position: 'absolute', top: 0, bottom: 0,
                          width: 6, borderRadius: 999,
                          background: 'rgba(120,160,255,0.12)',
                          alignSelf: 'center',
                        }} />
                        <div style={{
                          position: 'absolute',
                          bottom: 0, left: '50%', transform: 'translateX(-50%)',
                          width: 6, borderRadius: 999,
                          height: `${volumeLevel}%`,
                          background: 'linear-gradient(to top, #6366f1, #06b6d4)',
                          boxShadow: '0 0 8px rgba(99,102,241,0.5)',
                          pointerEvents: 'none',
                        }} />
                        <div style={{
                          position: 'absolute',
                          bottom: `calc(${volumeLevel}% - 7px)`, left: '50%', transform: 'translateX(-50%)',
                          width: 16, height: 16, borderRadius: 999,
                          background: '#eaf1ff',
                          border: '2px solid #6366f1',
                          boxShadow: '0 2px 8px rgba(0,0,0,0.4)',
                          pointerEvents: 'none',
                        }} />
                      </div>
                    </div>
                    <span style={{ fontSize: 'var(--text-xs)', color: 'rgba(180,200,240,0.6)', marginTop: 4 }}>
                      {volumeLevel === 0 ? '🔇' : '🔊'}
                    </span>
                  </div>
                )}

                {/* 播放队列面板：已移到控制台外层左侧（QueueFloatButton），点击后自适应展开 */}
              </div>
            </div>
          )}

          {module === 'device' && deviceSub === 'list' && (
            <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
              <div>
                <SectionTitle title="输出设备" />
                <ItemList
                  items={deviceItems}
                  accent="#7fe9a3"
                  onSelect={(i) => { setSelectedDevice(i); setDeviceSub('device-detail') }}
                />
              </div>
              <div>
                <SectionTitle title="DSP 链" />
                <ItemList
                  items={dspItems}
                  accent="#c79eff"
                  onSelect={(i) => { setSelectedDsp(i); setDeviceSub('dsp-detail') }}
                />
              </div>
            </div>
          )}

          {module === 'device' && deviceSub === 'device-detail' && selectedDevice >= 0 && (
            <div style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
              <button
                onClick={() => setDeviceSub('list')}
                style={{
                  alignSelf: 'flex-start', padding: '5px 10px',
                  borderRadius: 8,
                  border: '1px solid rgba(120,160,255,0.22)',
                  background: 'rgba(255,255,255,0.04)',
                  color: '#c6d4ff', fontSize: 'var(--text-xs)', cursor: 'pointer',
                  display: 'inline-flex', alignItems: 'center', gap: 4,
                }}
              >{BackIcon} 返回列表</button>
              <div style={{
                borderRadius: 12, padding: 14,
                border: '1px solid rgba(127,233,163,0.3)',
                background: 'linear-gradient(135deg, rgba(127,233,163,0.1), rgba(6,182,212,0.06))',
              }}>
                <div style={{ font: '700 14px "PingFang SC", system-ui', color: '#eef4ff', marginBottom: 4 }}>
                  {deviceItems[selectedDevice].title}
                </div>
                <div style={{ fontSize: 'var(--text-xs)', color: 'rgba(180,200,240,0.7)', marginBottom: 12 }}>
                  {deviceItems[selectedDevice].sub || '默认设备'}
                </div>
                <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
                  {[
                    { label: '采样率', value: '48 kHz' },
                    { label: '位深', value: '24 bit' },
                    { label: '通道', value: '立体声 (2ch)' },
                    { label: '独占模式', value: '已启用' },
                    { label: '缓冲区', value: '2048 samples' },
                  ].map(row => (
                    <div key={row.label} style={{
                      display: 'flex', justifyContent: 'space-between',
                      padding: '6px 0', borderBottom: '1px solid rgba(120,160,255,0.1)',
                    }}>
                      <span style={{ fontSize: 'var(--text-xs)', color: 'rgba(180,200,240,0.7)' }}>{row.label}</span>
                      <span style={{ fontSize: 'var(--text-xs)', color: '#dde8ff', font: '600 11px ui-monospace, monospace' }}>{row.value}</span>
                    </div>
                  ))}
                </div>
              </div>
              <button
                style={{
                  width: '100%', padding: '10px 12px', borderRadius: 10,
                  border: '1px solid rgba(127,233,163,0.35)',
                  background: 'linear-gradient(135deg, rgba(127,233,163,0.18), rgba(6,182,212,0.1))',
                  color: '#eaf1ff', cursor: 'pointer',
                  font: '700 12px "PingFang SC", system-ui',
                }}
              >设为默认输出设备</button>
            </div>
          )}

          {module === 'device' && deviceSub === 'dsp-detail' && selectedDsp >= 0 && (
            <div style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
              <button
                onClick={() => setDeviceSub('list')}
                style={{
                  alignSelf: 'flex-start', padding: '5px 10px',
                  borderRadius: 8,
                  border: '1px solid rgba(120,160,255,0.22)',
                  background: 'rgba(255,255,255,0.04)',
                  color: '#c6d4ff', fontSize: 'var(--text-xs)', cursor: 'pointer',
                  display: 'inline-flex', alignItems: 'center', gap: 4,
                }}
              >{BackIcon} 返回列表</button>
              <div style={{
                borderRadius: 12, padding: 14,
                border: '1px solid rgba(199,158,255,0.3)',
                background: 'linear-gradient(135deg, rgba(199,158,255,0.1), rgba(99,102,241,0.06))',
              }}>
                <div style={{ font: '700 14px "PingFang SC", system-ui', color: '#eef4ff', marginBottom: 4 }}>
                  {dspItems[selectedDsp].title}
                </div>
                <div style={{ fontSize: 'var(--text-xs)', color: 'rgba(180,200,240,0.7)', marginBottom: 12 }}>
                  {dspItems[selectedDsp].sub || 'DSP 效果'}
                </div>
                <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
                  {[
                    { label: '状态', value: dspItems[selectedDsp].active ? '已启用' : '已禁用' },
                    { label: '类型', value: '实时效果链' },
                    { label: '延迟', value: '< 2ms' },
                    { label: 'CPU 占用', value: '~3.2%' },
                  ].map(row => (
                    <div key={row.label} style={{
                      display: 'flex', justifyContent: 'space-between',
                      padding: '6px 0', borderBottom: '1px solid rgba(120,160,255,0.1)',
                    }}>
                      <span style={{ fontSize: 'var(--text-xs)', color: 'rgba(180,200,240,0.7)' }}>{row.label}</span>
                      <span style={{ fontSize: 'var(--text-xs)', color: '#dde8ff', font: '600 11px ui-monospace, monospace' }}>{row.value}</span>
                    </div>
                  ))}
                </div>
              </div>
              <button
                style={{
                  width: '100%', padding: '10px 12px', borderRadius: 10,
                  border: `1px solid ${dspItems[selectedDsp].active ? 'rgba(244,63,94,0.35)' : 'rgba(199,158,255,0.35)'}`,
                  background: dspItems[selectedDsp].active
                    ? 'linear-gradient(135deg, rgba(244,63,94,0.14), rgba(251,146,60,0.08))'
                    : 'linear-gradient(135deg, rgba(199,158,255,0.18), rgba(99,102,241,0.1))',
                  color: '#eaf1ff', cursor: 'pointer',
                  font: '700 12px "PingFang SC", system-ui',
                }}
              >{dspItems[selectedDsp].active ? '禁用此效果' : '启用此效果'}</button>
            </div>
          )}

          {module === 'extensions' && (
            <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
              <div>
                <SectionTitle title="实时监控" />
                <VitalsPanel audioRef={audioRef} />
              </div>
              {typeof onExit3D === 'function' && (
                <button
                  onClick={onExit3D}
                  style={{
                    width: '100%', padding: '10px 12px',
                    borderRadius: 10,
                    border: '1px solid rgba(255,150,160,0.28)',
                    background: 'linear-gradient(135deg, rgba(244,63,94,0.14), rgba(251,146,60,0.08))',
                    color: '#ffd3d8',
                    font: '700 12px "PingFang SC", system-ui',
                    letterSpacing: 0.6,
                    cursor: 'pointer',
                  }}
                >↓ 退出维度（返回主界面）</button>
              )}
            </div>
          )}

          {module === 'visualization' && (
            <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
              <div>
                <SectionTitle title="歌词" />
                <LyricsPanel currentTrack={currentTrack} playback={playback} audioRef={audioRef} />
              </div>
              <div>
                <SectionTitle title="频谱分析" />
                <SpectrumPanel audioRef={audioRef} />
              </div>
            </div>
          )}
        </div>
      </div>
      </div> {/* /控制台外层容器宽度 div */}
    </div>
  )
}

function SectionTitle({ title }: { title: string }) {
  return (
    <div style={{
      marginBottom: 8,
      font: '700 11.5px "PingFang SC", system-ui',
      letterSpacing: 1,
      color: 'rgba(180,200,240,0.82)',
      paddingLeft: 8,
      borderLeft: '3px solid #6fa3ff',
    }}>
      {title}
    </div>
  )
}

function ItemList({ items, accent, onSelect }: { items: DimItem[]; accent: string; onSelect?: (index: number) => void }) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
      {items.map((it, i) => (
        <div
          key={i}
          onClick={() => {
            if (onSelect) {
              onSelect(i)
            } else {
              invoke('play_index', { index: i }).catch(() => {})
            }
          }}
          style={{
            borderRadius: 10, padding: '9px 11px',
            border: `1px solid ${it.active ? 'rgba(140,180,255,0.42)' : 'rgba(120,160,255,0.16)'}`,
            background: it.active
              ? 'linear-gradient(135deg, rgba(99,102,241,0.16), rgba(6,182,212,0.08))'
              : 'rgba(10,12,30,0.35)',
            display: 'flex', alignItems: 'center', gap: 10,
            cursor: 'pointer',
            transition: 'background 0.15s, border-color 0.15s',
          }}
          onMouseEnter={(e: React.MouseEvent<HTMLDivElement>) => {
            if (!it.active) {
              e.currentTarget.style.background = 'rgba(99,102,241,0.12)'
              e.currentTarget.style.borderColor = 'rgba(140,180,255,0.35)'
            }
          }}
          onMouseLeave={(e: React.MouseEvent<HTMLDivElement>) => {
            if (!it.active) {
              e.currentTarget.style.background = 'rgba(10,12,30,0.35)'
              e.currentTarget.style.borderColor = 'rgba(120,160,255,0.16)'
            }
          }}
        >
          <span style={{
            width: 8, height: 8, borderRadius: 999, flexShrink: 0,
            background: it.active ? (it.accent || accent) : 'rgba(255,255,255,0.18)',
            boxShadow: it.active ? `0 0 10px ${it.accent || accent}` : 'none',
          }} />
          <div style={{ flex: 1, minWidth: 0 }}>
            <div style={{
              font: it.active ? '700 11.5px "PingFang SC", system-ui' : '600 11.5px "PingFang SC", system-ui',
              color: it.active ? '#eef4ff' : 'rgba(210,222,255,0.82)',
              whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
            }}>{it.title}</div>
            {it.sub && (
              <div style={{
                marginTop: 2,
                fontSize: 'var(--text-xs)', color: it.accent ? it.accent : 'rgba(170,190,230,0.7)',
                whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
              }}>{it.sub}</div>
            )}
          </div>
          {it.active && (
            <span style={{
              fontSize: 'var(--text-xs)', padding: '2px 6px', borderRadius: 999,
              background: 'rgba(74,222,128,0.12)',
              border: '1px solid rgba(74,222,128,0.28)',
              color: 'rgba(130,255,170,0.85)',
            }}>ACTIVE</span>
          )}
        </div>
      ))}
    </div>
  )
}

// ========== 控制台「监控」Tab：6 项实时仪表（LUFs/RMS/Peak/谱质心/节拍冲量/DR 动态范围）+ 33Hz 滚动波形 ==========
function VitalsPanel({ audioRef }: { audioRef: React.MutableRefObject<SmoothedAudioData> }) {
  const canv = useRef<HTMLCanvasElement>(null)
  const [vitals, setVitals] = useState({ lufs: 0, rms: 0, peak: 0, centroid: 0, pulse: 0, dr: 0 })
  const ring = useMemo(() => new Array(40).fill(0).map(() => 0), [])
  const ringState = useRef<{ buf: number[]; head: number }>({ buf: ring.slice(), head: 0 })

  useEffect(() => {
    let raf = 0
    const step = () => {
      const a = audioRef.current
      if (a) {
        const s64 = a.spectrum64
        const bass = s64.length > 0 ? s64.slice(0, 8).reduce((s: number, x: number) => s + x, 0) / Math.max(1, Math.min(8, s64.length)) : 0
        const mid = s64.length > 0 ? s64.slice(8, 32).reduce((s: number, x: number) => s + x, 0) / Math.max(1, Math.min(24, s64.length)) : 0
        const treb = s64.length > 0 ? s64.slice(32).reduce((s: number, x: number) => s + x, 0) / Math.max(1, s64.length - 32) : 0
        const rms = Math.sqrt((bass * bass + mid * mid + treb * treb) / 3)
        const peak = Math.max(bass, mid, treb)
        const lufs = Math.max(-72, -18 * Math.log10(Math.max(1e-4, rms / 0.5)))
        let centroid = 0
        let wsum = 0
        for (let i = 0; i < s64.length; i++) {
          const w = (i + 1) / s64.length
          centroid += i * s64[i] * w
          wsum += s64[i] * w
        }
        centroid = wsum > 0 ? centroid / wsum / Math.max(1, s64.length) : 0
        const pulse = Math.pow(Math.max(0, bass - 0.35) * 1.85, 1.25)
        const rs = ringState.current
        rs.buf[rs.head] = bass
        rs.head = (rs.head + 1) % rs.buf.length
        const minB = Math.min(...rs.buf)
        const maxB = Math.max(...rs.buf)
        const dr = maxB - minB
        setVitals({
          lufs: clamp01(lufs / 18),
          rms: clamp01(rms),
          peak: clamp01(peak),
          centroid: clamp01(centroid),
          pulse: clamp01(pulse),
          dr: clamp01(dr),
        })
      }
      draw()
      raf = requestAnimationFrame(step)
    }
    raf = requestAnimationFrame(step)
    return () => cancelAnimationFrame(raf)
  }, [audioRef])

  const draw = () => {
    const c = canv.current
    if (!c) return
    const ctx = c.getContext('2d'); if (!ctx) return
    const W = c.width = c.clientWidth * (window.devicePixelRatio || 1)
    const H = c.height = c.clientHeight * (window.devicePixelRatio || 1)
    ctx.clearRect(0, 0, W, H)
    // 背景 33Hz 波形条（bass 滚动）
    const s64draw = audioRef.current.spectrum64
    const n = 128
    const gap = 1.5 * (window.devicePixelRatio || 1)
    const bw = (W - gap * (n - 1)) / n
    const baseH = H * 0.9
    for (let i = 0; i < n; i++) {
      const bi = Math.floor((i / n) * Math.min(12, s64draw.length))
      const v = Math.max(0.06, s64draw[bi] ?? 0)
      const h = v * baseH
      const g = ctx.createLinearGradient(0, H - h, 0, H)
      g.addColorStop(0, i < n * 0.3 ? '#7fe9ff' : i < n * 0.7 ? '#a78bfa' : '#fb7185')
      g.addColorStop(1, 'rgba(10,12,40,0.05)')
      ctx.fillStyle = g
      ctx.fillRect(i * (bw + gap), H - h, bw, h)
    }
  }

  const meters: { key: keyof typeof vitals; label: string; sub: string; color: string }[] = [
    { key: 'lufs',     label: '响度 LUFS', sub: 'Integrated Loudness', color: 'linear-gradient(90deg, #4ade80, #22d3ee)' },
    { key: 'rms',      label: '均方根 RMS', sub: 'Root Mean Square',   color: 'linear-gradient(90deg, #22d3ee, #818cf8)' },
    { key: 'peak',     label: '峰值 Peak', sub: 'True Peak',           color: 'linear-gradient(90deg, #f472b6, #fbbf24)' },
    { key: 'centroid', label: '谱质心',    sub: 'Spectral Centroid',   color: 'linear-gradient(90deg, #818cf8, #a78bfa)' },
    { key: 'pulse',    label: '节拍冲量',  sub: 'Beat Pulse (Bass)',   color: 'linear-gradient(90deg, #fbbf24, #fb7185)' },
    { key: 'dr',       label: '动态范围 DR', sub: 'Dynamic Range',     color: 'linear-gradient(90deg, #7fe9a3, #60a5fa)' },
  ]
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
      <div style={{
        borderRadius: 12,
        padding: 10,
        border: '1px solid rgba(120,160,255,0.22)',
        background: 'linear-gradient(135deg, rgba(15,23,42,0.5), rgba(76,29,149,0.12))',
      }}>
        <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'baseline', marginBottom: 6 }}>
          <div style={{ fontSize: 'var(--text-xs)', color: 'rgba(170,190,230,0.72)' }}>实时波形 · 33Hz 滚动</div>
          <div style={{ fontSize: 'var(--text-xs)', color: 'rgba(170,190,230,0.45)', font: '600 10px ui-monospace, Menlo, Consolas, monospace' }}>BASS-MID-TREB</div>
        </div>
        <canvas ref={canv} style={{ width: '100%', height: 86, borderRadius: 9, display: 'block', background: 'rgba(5,6,20,0.45)' }} />
      </div>
      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 8 }}>
        {meters.map(m => (
          <div key={m.key} style={{
            borderRadius: 10, padding: '9px 11px',
            border: '1px solid rgba(120,160,255,0.2)',
            background: 'rgba(10,12,30,0.45)',
          }}>
            <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'baseline' }}>
              <div style={{
                fontSize: 9.5, color: 'rgba(170,190,230,0.75)',
                textTransform: 'uppercase', letterSpacing: 0.4,
              }}>{m.label}</div>
              <div style={{
                font: '700 11.5px ui-monospace, Menlo, Consolas, monospace',
                color: '#eaf1ff',
              }}>{Math.round(vitals[m.key] * 100)}%</div>
            </div>
            <div style={{ marginTop: 2, fontSize: 8.5, color: 'rgba(150,170,210,0.55)', marginBottom: 7 }}>{m.sub}</div>
            <div style={{
              height: 5, borderRadius: 999, overflow: 'hidden',
              background: 'rgba(255,255,255,0.06)',
            }}>
              <div style={{
                width: `${vitals[m.key] * 100}%`,
                height: '100%',
                background: m.color,
                boxShadow: `0 0 8px rgba(120,160,255,0.35)`,
              }} />
            </div>
          </div>
        ))}
      </div>
    </div>
  )
}

// ========== 控制台「歌词」Tab：滚动仿音乐流滚动条 + 高亮当前行 + 音量/进度叠加 ==========
function LyricsPanel({ currentTrack, playback, audioRef }: {
  currentTrack: string
  playback?: Props['playback']
  audioRef: React.MutableRefObject<SmoothedAudioData>
}) {
  const canv = useRef<HTMLCanvasElement>(null)
  const boxRef = useRef<HTMLDivElement>(null)
  const curIdxRef = useRef<number>(-1)
  const liveProgress = useProgress()

  // 根据音轨名与播放位置生成仿真歌词（因为本层不持有歌词接口）；生产环境可接入 LyricsSlice 的真实歌词
  const mockLyrics: { t: number; text: string }[] = useMemo(() => {
    const title = currentTrack || '未命名音轨'
    const base = [
      '夜色漫过屋顶 光开始沉睡',
      '我听见时间在你指缝里低语',
      '每一次呼吸 都是重逢的回响',
      `——《${title}》`,
      '星光洒落在你 遗忘的窗沿',
      '我用一颗心跳 丈量人间',
      '有些话没说 不必再说',
      '风会替我抵达 每一个深夜',
      '如果还能遇见你 在另一条时间线',
      '我会带上所有 我写过的歌',
    ]
    const dur = Math.max(120, playback?.duration_secs ?? 180)
    return base.map((text, i) => ({ t: (i / base.length) * dur * 0.9, text }))
  }, [currentTrack, playback?.duration_secs])

  useEffect(() => {
    let raf = 0
    const step = () => {
      // Live read from the progress bus (no re-render needed in a rAF loop).
      const pos = getProgress().position
      let idx = 0
      for (let i = 0; i < mockLyrics.length; i++) if (pos >= mockLyrics[i].t) idx = i
      curIdxRef.current = idx
      draw()
      raf = requestAnimationFrame(step)
    }
    raf = requestAnimationFrame(step)
    return () => cancelAnimationFrame(raf)
  }, [audioRef, mockLyrics])

  const draw = () => {
    const c = canv.current
    if (!c) return
    const ctx = c.getContext('2d'); if (!ctx) return
    const DPR = window.devicePixelRatio || 1
    const W = c.width = c.clientWidth * DPR
    const H = c.height = c.clientHeight * DPR
    ctx.clearRect(0, 0, W, H)
    const barsL = audioRef.current.spectrum64
    // 顶部柔和光晕 + 频谱柱
    const n = 96
    const gap = 1 * DPR
    const bw = (W - gap * (n - 1)) / n
    for (let i = 0; i < n; i++) {
      const bi = Math.floor((i / n) * barsL.length)
      const v = Math.max(0.04, barsL[bi] ?? 0)
      const h = v * H * 0.24
      const g = ctx.createLinearGradient(0, H * 0.05 - h, 0, H * 0.05)
      g.addColorStop(0, 'rgba(167,139,250,0.85)')
      g.addColorStop(1, 'rgba(96,165,250,0.05)')
      ctx.fillStyle = g
      ctx.fillRect(i * (bw + gap), H * 0.05 - h + 2, bw, h)
    }
  }

  return (
    <div ref={boxRef} style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
      <div style={{
        borderRadius: 12,
        padding: 10,
        border: '1px solid rgba(120,160,255,0.22)',
        background: 'linear-gradient(135deg, rgba(15,23,42,0.5), rgba(30,64,175,0.14))',
      }}>
        <canvas ref={canv} style={{ width: '100%', height: 58, borderRadius: 9, display: 'block', background: 'rgba(5,6,20,0.45)' }} />
      </div>
      <div style={{
        borderRadius: 12,
        padding: '12px 12px 10px',
        border: '1px solid rgba(120,160,255,0.2)',
        background: 'linear-gradient(180deg, rgba(255,255,255,0.03), rgba(10,12,30,0.45))',
        maxHeight: 220,
        overflow: 'hidden',
        position: 'relative',
      }}>
        {/* 顶部/底部渐隐条 */}
        <div style={{
          position: 'absolute', inset: 0, pointerEvents: 'none', zIndex: 2,
          background: 'linear-gradient(180deg, rgba(8,10,28,0.9) 0%, rgba(8,10,28,0) 14%, rgba(8,10,28,0) 80%, rgba(8,10,28,0.92) 100%)',
        }} />
        <div style={{
          position: 'relative', display: 'flex', flexDirection: 'column', gap: 10,
          transform: `translateY(${-Math.max(0, curIdxRef.current - 1) * 40}px)`,
          transition: 'transform 0.55s cubic-bezier(0.22, 1, 0.36, 1)',
          padding: '8px 2px',
        }}>
          {mockLyrics.map((ln, i) => {
            const cur = i === curIdxRef.current
            return (
              <div key={i} style={{
                font: `${cur ? 700 : 500} ${cur ? 14.5 : 12.5}px "PingFang SC", system-ui`,
                lineHeight: 1.5,
                letterSpacing: cur ? 0.6 : 0.2,
                textAlign: 'center',
                color: cur
                  ? '#ffffff'
                  : (Math.abs(i - curIdxRef.current) <= 2 ? 'rgba(210,222,255,0.72)' : 'rgba(180,195,230,0.36)'),
                textShadow: cur ? '0 0 18px rgba(167,139,250,0.6), 0 0 6px rgba(96,165,250,0.55)' : 'none',
                transform: cur ? 'scale(1.02)' : 'scale(1)',
                transition: 'all 0.45s cubic-bezier(0.22, 1, 0.36, 1)',
              }}>{ln.text}</div>
            )
          })}
        </div>
      </div>
      <div style={{
        display: 'flex', justifyContent: 'space-between', alignItems: 'center',
        fontSize: 'var(--text-xs)', color: 'rgba(170,190,230,0.65)',
        padding: '0 2px',
      }}>
        <span>{fmtLocal(liveProgress.position)}</span>
        <span style={{
          font: '700 10px "PingFang SC", system-ui', letterSpacing: 0.8,
          color: 'rgba(220,230,255,0.82)',
        }}>LIVE 滚动歌词</span>
        <span>{fmtLocal(liveProgress.duration ?? playback?.duration_secs ?? 0)}</span>
      </div>
    </div>
  )
}

// ========== 控制台「频谱」Tab：3 层可视化（33Hz/主频/高频）+ 曲线包络 + 相位渐变 ==========
function SpectrumPanel({ audioRef }: { audioRef: React.MutableRefObject<SmoothedAudioData> }) {
  const canv = useRef<HTMLCanvasElement>(null)
  const specRef = useRef<number[]>(new Array(96).fill(0.04))

  useEffect(() => {
    let raf = 0
    const step = () => {
      const s64 = audioRef.current.spectrum64
      const s = specRef.current
      for (let i = 0; i < s.length; i++) {
        const bi = Math.floor((i / s.length) * s64.length)
        const target = Math.max(0.04, s64[bi] ?? 0)
        s[i] += (target - s[i]) * 0.35
      }
      draw()
      raf = requestAnimationFrame(step)
    }
    raf = requestAnimationFrame(step)
    return () => cancelAnimationFrame(raf)
  }, [audioRef])

  const draw = () => {
    const c = canv.current
    if (!c) return
    const ctx = c.getContext('2d'); if (!ctx) return
    const DPR = window.devicePixelRatio || 1
    const W = c.width = c.clientWidth * DPR
    const H = c.height = c.clientHeight * DPR
    ctx.clearRect(0, 0, W, H)
    // 背景网格
    ctx.strokeStyle = 'rgba(120,160,255,0.07)'
    ctx.lineWidth = 1 * DPR
    for (let i = 1; i < 6; i++) {
      ctx.beginPath()
      const y = (i / 6) * H
      ctx.moveTo(0, y); ctx.lineTo(W, y)
      ctx.stroke()
    }
    const s = specRef.current
    const n = s.length
    const gap = 1 * DPR
    const bw = (W - gap * (n - 1)) / n
    // 包络曲线
    ctx.beginPath()
    ctx.moveTo(0, H)
    for (let i = 0; i < n; i++) {
      const x = i * (bw + gap) + bw / 2
      const y = H - s[i] * H * 0.92
      if (i === 0) ctx.lineTo(x, y); else ctx.lineTo(x, y)
    }
    ctx.lineTo(W, H)
    ctx.closePath()
    const fillG = ctx.createLinearGradient(0, 0, 0, H)
    fillG.addColorStop(0, 'rgba(251,113,133,0.55)')
    fillG.addColorStop(0.5, 'rgba(167,139,250,0.35)')
    fillG.addColorStop(1, 'rgba(34,211,238,0.05)')
    ctx.fillStyle = fillG
    ctx.fill()
    // 频谱柱（三分段着色：bass/mid/treb）
    for (let i = 0; i < n; i++) {
      const v = s[i]
      const h = v * H * 0.86
      const x = i * (bw + gap)
      const y = H - h
      const ratio = i / n
      const g = ctx.createLinearGradient(x, y, x, H)
      if (ratio < 0.33) {
        g.addColorStop(0, '#7fe9ff'); g.addColorStop(1, 'rgba(34,211,238,0.1)')
      } else if (ratio < 0.72) {
        g.addColorStop(0, '#a78bfa'); g.addColorStop(1, 'rgba(167,139,250,0.08)')
      } else {
        g.addColorStop(0, '#fb7185'); g.addColorStop(1, 'rgba(251,113,133,0.1)')
      }
      ctx.fillStyle = g
      ctx.fillRect(x, y, bw, h)
    }
  }

  const low = specRef.current.slice(0, 24).reduce((s, x) => s + x, 0) / 24
  const mid = specRef.current.slice(24, 64).reduce((s, x) => s + x, 0) / 40
  const hi  = specRef.current.slice(64).reduce((s, x) => s + x, 0) / 32
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
      <div style={{
        borderRadius: 12,
        padding: 10,
        border: '1px solid rgba(120,160,255,0.22)',
        background: 'linear-gradient(135deg, rgba(15,23,42,0.5), rgba(124,58,237,0.1))',
      }}>
        <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'baseline', marginBottom: 6 }}>
          <div style={{ fontSize: 'var(--text-xs)', color: 'rgba(170,190,230,0.72)' }}>频谱分析 · 96 频段 · 平滑 0.35</div>
          <div style={{ fontSize: 'var(--text-xs)', color: 'rgba(170,190,230,0.45)', font: '600 10px ui-monospace, Menlo, Consolas, monospace' }}>20Hz – 20kHz</div>
        </div>
        <canvas ref={canv} style={{ width: '100%', height: 140, borderRadius: 9, display: 'block', background: 'rgba(5,6,20,0.5)' }} />
      </div>
      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 1fr', gap: 8 }}>
        {([
          { k: '低频 BASS', v: low, color: '#7fe9ff' },
          { k: '中频 MID',  v: mid, color: '#a78bfa' },
          { k: '高频 TREB', v: hi,  color: '#fb7185' },
        ] as { k: string; v: number; color: string }[]).map(b => (
          <div key={b.k} style={{
            borderRadius: 10, padding: '9px 10px',
            border: '1px solid rgba(120,160,255,0.2)',
            background: 'rgba(10,12,30,0.45)',
          }}>
            <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'baseline' }}>
              <div style={{
                fontSize: 'var(--text-xs)', color: 'rgba(170,190,230,0.75)', letterSpacing: 0.5,
              }}>{b.k}</div>
              <div style={{
                font: '700 11.5px ui-monospace, Menlo, Consolas, monospace', color: b.color,
              }}>{Math.round(b.v * 100)}%</div>
            </div>
            <div style={{
              height: 5, borderRadius: 999, overflow: 'hidden',
              background: 'rgba(255,255,255,0.06)', marginTop: 7,
            }}>
              <div style={{
                width: `${Math.min(100, b.v * 100)}%`, height: '100%',
                background: b.color,
                boxShadow: `0 0 8px ${b.color}77`,
              }} />
            </div>
          </div>
        ))}
      </div>
    </div>
  )
}

// 小工具
function clamp01(x: number) { return Math.max(0, Math.min(1, x)) }
function fmtLocal(s: number | null | undefined) {
  if (s == null) return '00:00'
  const m = Math.floor(s / 60)
  const sec = Math.floor(s % 60)
  return `${String(m).padStart(2, '0')}:${String(sec).padStart(2, '0')}`
}
