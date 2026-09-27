// 元素级「侧面板切片」：每行一个玻璃 Plane，标题 + 若干 item
// 左：设备列表；右：播放列表 / DSP
import { useMemo, useRef } from 'react'
import { useFrame } from '@react-three/fiber'
import * as THREE from 'three'
import type { MutableRefObject } from 'react'
import type { SmoothedAudioData } from './audioBridge'
import { lerp } from './audioBridge'
import { chromaHue, hsl } from './GlassSlice'

interface Item {
  title: string
  sub?: string
  active?: boolean
  accent?: string
}

interface Props {
  audioRef: MutableRefObject<SmoothedAudioData>
  position?: [number, number, number]
  rotation?: [number, number, number]
  title: string
  items: Item[]
  /** 驱动数据：'lowFreq' / 'rms' / 'hiFreq' */
  drive?: 'lowFreq' | 'rms' | 'hiFreq'
  accentHue?: number
  tiltPulse?: boolean
}

export default function SidePanelSlice({
  audioRef, position = [-7, 1, -2], rotation = [0, 0.25, 0],
  title, items, drive = 'lowFreq', accentHue = 220, tiltPulse = true,
}: Props) {
  const groupRef = useRef<THREE.Group>(null)
  const rowRefs = useRef<THREE.Group[]>([])
  const itemCount = Math.max(1, items.length)
  const panelH = 0.8 + itemCount * 1.12 + 0.5
  const panelW = 5.6
  const hueRef = useRef(accentHue)

  useFrame(({ clock }) => {
    const d = audioRef.current
    const signal =
      drive === 'lowFreq' ? d.lowFreqAvg :
      drive === 'hiFreq' ? d.highFreqAvg : d.rms

    if (groupRef.current) {
      // 左右摇摆呼吸
      if (tiltPulse) {
        const baseY = rotation[1]
        groupRef.current.rotation.y = baseY + Math.sin(clock.elapsedTime * 0.7) * 0.03 + signal * 0.08
      }
      const s = 1 + signal * 0.04
      groupRef.current.scale.setScalar(s)
    }

    hueRef.current = lerp(hueRef.current, chromaHue(d.chroma12), 0.05)

    for (let i = 0; i < rowRefs.current.length; i++) {
      const g = rowRefs.current[i]
      if (!g) continue
      const item = items[i]
      const wob = (item?.active ? 1.04 : 1) + d.rms * 0.03 + Math.sin(clock.elapsedTime * 1.2 + i) * 0.008
      g.scale.setScalar(wob)
      g.position.x = -panelW / 2 + 0.2 + (d.onset * (i % 2 === 0 ? 0.12 : -0.1))
    }
  })

  // 生成每行的 canvas text
  const itemTextures = useMemo(() => {
    return items.map((it) => {
      const canvas = document.createElement('canvas')
      canvas.width = 512
      canvas.height = 96
      const ctx = canvas.getContext('2d')!
      ctx.clearRect(0, 0, 512, 96)
      ctx.font = 'bold 34px "PingFang SC", "Microsoft YaHei", system-ui'
      ctx.textAlign = 'left'
      ctx.textBaseline = 'top'
      ctx.fillStyle = it.active ? '#dbe9ff' : 'rgba(220,230,255,0.82)'
      ctx.fillText(it.title.slice(0, 20), 12, 8)
      if (it.sub) {
        ctx.font = '24px "PingFang SC", "Microsoft YaHei", system-ui'
        ctx.fillStyle = it.accent ?? 'rgba(155,180,230,0.82)'
        ctx.fillText(it.sub.slice(0, 30), 12, 50)
      }
      const t = new THREE.CanvasTexture(canvas)
      t.anisotropy = 8
      return t
    })
  }, [items])

  const titleTex = useMemo(() => {
    const canvas = document.createElement('canvas')
    canvas.width = 512
    canvas.height = 80
    const ctx = canvas.getContext('2d')!
    ctx.clearRect(0, 0, 512, 80)
    ctx.font = 'bold 40px "PingFang SC", "Microsoft YaHei", system-ui'
    ctx.textAlign = 'left'
    ctx.textBaseline = 'middle'
    const grad = ctx.createLinearGradient(0, 0, 512, 0)
    grad.addColorStop(0, 'rgba(180,210,255,1)')
    grad.addColorStop(1, 'rgba(140,180,255,0.85)')
    ctx.fillStyle = grad
    ctx.fillText(title.slice(0, 14), 16, 40)
    const t = new THREE.CanvasTexture(canvas)
    return t
  }, [title])

  return (
    <group position={position} rotation={rotation} ref={groupRef}>
      {/* 面板玻璃底 */}
      <mesh position={[0, 0, 0]} castShadow receiveShadow>
        <boxGeometry args={[panelW, panelH, 0.3]} />
        <meshPhysicalMaterial
          color="#ffffff"
          transparent
          opacity={0.65}
          transmission={0.7}
          thickness={0.35}
          roughness={0.2}
          metalness={0.06}
          ior={1.5}
          clearcoat={1}
          clearcoatRoughness={0.1}
          emissive={new THREE.Color(hsl(accentHue, 70, 40, 1))}
          emissiveIntensity={0.05}
        />
      </mesh>
      <lineSegments position={[0, 0, 0]}>
        <edgesGeometry args={[new THREE.BoxGeometry(panelW, panelH, 0.3), 15]} />
        <lineBasicMaterial color="#b4d2ff" transparent opacity={0.9} />
      </lineSegments>

      {/* 标题切片 */}
      <group position={[0, panelH / 2 - 0.7, 0.18]}>
        <mesh>
          <boxGeometry args={[panelW - 0.4, 0.8, 0.1]} />
          <meshPhysicalMaterial
            color="#0e1028"
            transparent
            opacity={0.55}
            transmission={0.3}
            roughness={0.4}
          />
        </mesh>
        <mesh position={[0, 0, 0.08]}>
          <planeGeometry args={[panelW - 0.5, 0.65]} />
          <meshBasicMaterial map={titleTex} transparent depthWrite={false} />
        </mesh>
      </group>

      {/* 每行元素级切片 */}
      {items.map((it, i) => {
        const y = panelH / 2 - 1.35 - i * 1.12
        return (
          <group
            key={i}
            position={[0, y, 0.18]}
            ref={(el) => { rowRefs.current[i] = el as any }}
          >
            {/* 每行玻璃底 */}
            <mesh>
              <boxGeometry args={[panelW - 0.4, 0.95, 0.12]} />
              <meshPhysicalMaterial
                color={it.active ? hsl(hueRef.current, 70, 75, 1) : '#ffffff'}
                transparent
                opacity={it.active ? 0.85 : 0.55}
                transmission={0.55}
                thickness={0.15}
                roughness={0.22}
                clearcoat={0.9}
                emissive={new THREE.Color(hsl(hueRef.current, 70, 50, 1))}
                emissiveIntensity={it.active ? 0.2 : 0.06}
              />
            </mesh>
            <lineSegments>
              <edgesGeometry args={[new THREE.BoxGeometry(panelW - 0.4, 0.95, 0.12), 20]} />
              <lineBasicMaterial
                color={it.active ? 'rgba(180,210,255,0.95)' : 'rgba(150,175,225,0.65)'}
                transparent
                opacity={0.9}
              />
            </lineSegments>
            {/* 行文字 */}
            <mesh position={[0, 0, 0.1]}>
              <planeGeometry args={[panelW - 0.5, 0.9]} />
              <meshBasicMaterial map={itemTextures[i]} transparent depthWrite={false} />
            </mesh>
            {/* active 状态小发光点 */}
            {it.active && (
              <mesh position={[-panelW / 2 + 0.25, 0, 0.1]}>
                <sphereGeometry args={[0.07, 20, 20]} />
                <meshBasicMaterial color={new THREE.Color(hsl(hueRef.current, 90, 75, 1))} />
              </mesh>
            )}
          </group>
        )
      })}
    </group>
  )
}

// ========== 歌词切片（顶部悬浮） ==========
interface LyricsProps {
  audioRef: MutableRefObject<SmoothedAudioData>
  lines?: { text: string; active?: boolean; t?: number }[]
}

export function LyricsSlice({ audioRef, lines = [] }: LyricsProps) {
  const groupRef = useRef<THREE.Group>(null)
  const displayLines = lines.length > 0
    ? lines.slice(0, 3)
    : [{ text: 'Phonon · 三维承空', active: true }, { text: '在空间里，让音乐被看见', active: false }, { text: 'BPM · 频谱 · 响度驱动中', active: false }]

  const textures = useMemo(() => {
    return displayLines.map((ln) => {
      const canvas = document.createElement('canvas')
      canvas.width = 1024
      canvas.height = 120
      const ctx = canvas.getContext('2d')!
      ctx.clearRect(0, 0, 1024, 120)
      ctx.textAlign = 'center'
      ctx.textBaseline = 'middle'
      if (ln.active) {
        ctx.font = 'bold 54px "PingFang SC", "Microsoft YaHei", system-ui'
        const grad = ctx.createLinearGradient(0, 0, 1024, 0)
        grad.addColorStop(0, 'rgba(230,245,255,1)')
        grad.addColorStop(0.5, 'rgba(170,200,255,1)')
        grad.addColorStop(1, 'rgba(240,210,255,1)')
        ctx.fillStyle = grad
        ctx.shadowColor = 'rgba(120,160,255,0.8)'
        ctx.shadowBlur = 18
      } else {
        ctx.font = '34px "PingFang SC", "Microsoft YaHei", system-ui'
        ctx.fillStyle = 'rgba(170,185,225,0.7)'
      }
      ctx.fillText(ln.text.slice(0, 32), 512, 60)
      const t = new THREE.CanvasTexture(canvas)
      t.anisotropy = 8
      return t
    })
  }, [displayLines])

  useFrame(({ clock }) => {
    const d = audioRef.current
    if (groupRef.current) {
      groupRef.current.position.y = 5.2 + Math.sin(clock.elapsedTime * 0.5) * 0.08 + d.rms * 0.2
      groupRef.current.rotation.x = -0.05 + d.lowFreqAvg * 0.06
    }
  })

  return (
    <group position={[0, 5.2, 2.2]} ref={groupRef}>
      {/* 悬浮大玻璃底板 */}
      <mesh>
        <boxGeometry args={[9, 2.6, 0.3]} />
        <meshPhysicalMaterial
          color="#ffffff"
          transparent
          opacity={0.55}
          transmission={0.75}
          thickness={0.45}
          roughness={0.2}
          clearcoat={1}
          emissive="#3850ff"
          emissiveIntensity={0.06}
        />
      </mesh>
      <lineSegments>
        <edgesGeometry args={[new THREE.BoxGeometry(9, 2.6, 0.3), 18]} />
        <lineBasicMaterial color="#b4d2ff" transparent opacity={0.9} />
      </lineSegments>

      {textures.map((tex, i) => (
        <mesh key={i} position={[0, 0.8 - i * 0.75, 0.18]}>
          <planeGeometry args={[8.6, 0.72]} />
          <meshBasicMaterial map={tex} transparent depthWrite={false} />
        </mesh>
      ))}
    </group>
  )
}
