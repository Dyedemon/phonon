// 播放器中心切片：封面玻璃方块 + 播放/暂停按钮 + 进度条 + 音量旋钮
// 元素级切片：每个控件独立一个小玻璃 mesh
import { useMemo, useRef } from 'react'
import { useFrame } from '@react-three/fiber'
import * as THREE from 'three'
import type { MutableRefObject } from 'react'
import type { SmoothedAudioData } from './audioBridge'
import { lerp } from './audioBridge'
import { GlassSlice } from './GlassSlice'

interface Props {
  audioRef: MutableRefObject<SmoothedAudioData>
  position?: [number, number, number]
  rotation?: [number, number, number]
  coverUrl?: string
  progress?: number // 0..1
  volume?: number   // 0..1
  isPlaying?: boolean
  currentTrack?: string | null
  duration?: number | null
  positionSecs?: number
}

export default function PlaybackSlice({
  audioRef, position = [0, 1.4, 0], rotation = [0, 0, 0],
  progress = 0, volume = 0.8, isPlaying = true,
  currentTrack, duration, positionSecs,
}: Props) {
  const coverScale = useRef(1)
  const coverRef = useRef<THREE.Group>(null)
  const progressRef = useRef<THREE.Mesh>(null)
  const btnRef = useRef<THREE.Mesh>(null)
  const rotRef = useRef(0)
  const titleRef = useRef<THREE.Group>(null)

  // 封面纹理（如果没有 URL，就用默认的玻璃 + 渐变 emissive）
  const coverTex = useMemo(() => {
    const canvas = document.createElement('canvas')
    canvas.width = canvas.height = 512
    const ctx = canvas.getContext('2d')!
    const grad = ctx.createLinearGradient(0, 0, 512, 512)
    grad.addColorStop(0, '#2d326a')
    grad.addColorStop(0.5, '#6366f1')
    grad.addColorStop(1, '#0ea5e9')
    ctx.fillStyle = grad
    ctx.fillRect(0, 0, 512, 512)
    // 音符占位
    ctx.fillStyle = 'rgba(255,255,255,0.25)'
    ctx.font = 'bold 200px system-ui'
    ctx.textAlign = 'center'
    ctx.textBaseline = 'middle'
    ctx.fillText('♪', 256, 256)
    const tex = new THREE.CanvasTexture(canvas)
    tex.anisotropy = 8
    return tex
  }, [])

  useFrame((_, delta) => {
    const d = audioRef.current
    const target = 1 + d.rms * 0.09
    coverScale.current = lerp(coverScale.current, target, 0.15)
    if (coverRef.current) {
      coverRef.current.scale.setScalar(coverScale.current)
    }
    // 封面缓慢旋转（播放时）
    rotRef.current += delta * (isPlaying ? 0.08 : 0.01)
    if (coverRef.current) {
      coverRef.current.rotation.y = rotRef.current
    }
    // 进度条 x-scale（以左对齐）
    if (progressRef.current) {
      const cur = progressRef.current.scale.x
      progressRef.current.scale.x = lerp(cur, Math.max(0.001, progress), 0.2)
    }
    // 播放按钮脉冲
    if (btnRef.current) {
      const s = 1 + d.onset * 0.3
      btnRef.current.scale.setScalar(s)
      const mat = btnRef.current.material as THREE.MeshPhysicalMaterial
      mat.emissiveIntensity = 0.2 + d.rms * 0.8
    }
    // 歌词/标题轻微浮动
    if (titleRef.current) {
      titleRef.current.position.y = 2.1 + Math.sin(performance.now() * 0.001) * 0.03 + d.rms * 0.05
    }
  })

  // 进度条背景与前景
  const PROG_W = 5.2
  const PROG_H = 0.1

  return (
    <group position={position} rotation={rotation}>
      {/* 大切片（封面 + 控制条容器） */}
      <GlassSlice
        width={7.2} height={8.2}
        color="#ffffff"
        opacity={0.68}
        emissive="#374bff"
        emissiveIntensity={0.05}
      />

      {/* 封面 */}
      <group position={[0, 1.1, 0.26]} ref={coverRef}>
        <mesh>
          <boxGeometry args={[4.2, 4.2, 0.45]} />
          <meshPhysicalMaterial
            map={coverTex}
            transparent
            opacity={0.95}
            roughness={0.22}
            metalness={0.1}
            clearcoat={1}
            clearcoatRoughness={0.08}
            emissive="#39428d"
            emissiveIntensity={0.15}
            side={THREE.DoubleSide}
          />
        </mesh>
        <lineSegments>
          <edgesGeometry args={[new THREE.BoxGeometry(4.2, 4.2, 0.45), 20]} />
          <lineBasicMaterial color="#b4d2ff" transparent opacity={0.85} />
        </lineSegments>
      </group>

      {/* 曲名（简单的 canvas 贴图 text） */}
      <TitlePlate title={currentTrack ?? 'No track playing'} ref={titleRef} />

      {/* 播放按钮 */}
      <group position={[0, -1.9, 0.3]}>
        <mesh ref={btnRef as any} castShadow>
          <cylinderGeometry args={[0.6, 0.6, 0.2, 48]} />
          <meshPhysicalMaterial
            color="#ffffff"
            emissive="#6366f1"
            emissiveIntensity={0.3}
            transmission={0.5}
            thickness={0.4}
            roughness={0.2}
            metalness={0.2}
            clearcoat={1}
            clearcoatRoughness={0.1}
            transparent
            opacity={0.92}
          />
        </mesh>
        {/* 播放三角/暂停 */}
        {isPlaying ? (
          <group position={[0, 0, 0.14]}>
            <mesh>
              <boxGeometry args={[0.16, 0.5, 0.08]} />
              <meshStandardMaterial color="#e9efff" emissive="#bdd2ff" emissiveIntensity={0.5} />
            </mesh>
            <mesh position={[0.3, 0, 0]}>
              <boxGeometry args={[0.16, 0.5, 0.08]} />
              <meshStandardMaterial color="#e9efff" emissive="#bdd2ff" emissiveIntensity={0.5} />
            </mesh>
          </group>
        ) : (
          <mesh position={[0.08, 0, 0.14]} rotation={[0, 0, 0]}>
            <coneGeometry args={[0.32, 0.55, 3]} />
            <meshStandardMaterial color="#e9efff" emissive="#bdd2ff" emissiveIntensity={0.5} />
          </mesh>
        )}
      </group>

      {/* 进度条容器 */}
      <group position={[0, -2.9, 0.2]}>
        <mesh>
          <boxGeometry args={[PROG_W + 0.2, PROG_H + 0.1, 0.08]} />
          <meshPhysicalMaterial color="#111429" transparent opacity={0.6} />
        </mesh>
        <mesh ref={progressRef as any} position={[-(PROG_W / 2) + (PROG_W * Math.max(0.001, progress)) / 2, 0, 0.06]}>
          <boxGeometry args={[PROG_W, PROG_H, 0.06]} />
          <meshPhysicalMaterial
            color="#ffffff"
            emissive="#38bdf8"
            emissiveIntensity={0.9}
            transparent
            opacity={0.95}
            transmission={0.3}
            roughness={0.2}
          />
        </mesh>
      </group>

      {/* 时间文本（进度/总时长） */}
      <TimeLabel positionSecs={positionSecs ?? 0} duration={duration ?? null} />

      {/* 音量旋钮（右下） */}
      <VolumeKnob volume={volume} audioRef={audioRef} />
    </group>
  )
}

// ========== 辅助子组件 ==========

import { forwardRef } from 'react'

const TitlePlate = forwardRef<THREE.Group, { title: string }>(function TitlePlate({ title }, ref) {
  const tex = useMemo(() => {
    const canvas = document.createElement('canvas')
    canvas.width = 1024
    canvas.height = 128
    const ctx = canvas.getContext('2d')!
    ctx.clearRect(0, 0, 1024, 128)
    ctx.fillStyle = 'rgba(15,18,45,0.01)'
    ctx.fillRect(0, 0, 1024, 128)
    const safe = (title || '').slice(0, 28)
    ctx.font = 'bold 56px system-ui, "PingFang SC", "Microsoft YaHei"'
    ctx.textAlign = 'center'
    ctx.textBaseline = 'middle'
    const grad = ctx.createLinearGradient(0, 0, 1024, 0)
    grad.addColorStop(0, 'rgba(220,235,255,0.95)')
    grad.addColorStop(1, 'rgba(150,200,255,0.95)')
    ctx.fillStyle = grad
    ctx.fillText(safe, 512, 64)
    const t = new THREE.CanvasTexture(canvas)
    t.anisotropy = 8
    return t
  }, [title])

  return (
    <group position={[0, 2.1, 0.3]} ref={ref}>
      <mesh>
        <planeGeometry args={[6.8, 0.85]} />
        <meshStandardMaterial
          map={tex}
          transparent
          depthWrite={false}
          color="#ffffff"
        />
      </mesh>
    </group>
  )
})

function TimeLabel({ positionSecs, duration }: { positionSecs: number; duration: number | null }) {
  const tex = useMemo(() => {
    const canvas = document.createElement('canvas')
    canvas.width = 512
    canvas.height = 64
    const ctx = canvas.getContext('2d')!
    ctx.clearRect(0, 0, 512, 64)
    ctx.font = '36px "JetBrains Mono", ui-monospace, Menlo, monospace'
    ctx.textAlign = 'left'
    ctx.textBaseline = 'middle'
    const fmt = (s: number | null | undefined) => {
      if (s == null || !isFinite(s)) return '--:--'
      const mm = Math.floor(s / 60)
      const ss = Math.floor(s % 60)
      return `${String(mm).padStart(2, '0')}:${String(ss).padStart(2, '0')}`
    }
    ctx.fillStyle = 'rgba(200,220,255,0.85)'
    ctx.fillText(fmt(positionSecs), 10, 32)
    ctx.textAlign = 'right'
    ctx.fillStyle = 'rgba(150,170,210,0.85)'
    ctx.fillText(fmt(duration), 500, 32)
    const t = new THREE.CanvasTexture(canvas)
    return t
  }, [positionSecs, duration])

  return (
    <mesh position={[0, -3.45, 0.21]}>
      <planeGeometry args={[5.2, 0.65]} />
      <meshBasicMaterial map={tex} transparent depthWrite={false} />
    </mesh>
  )
}

function VolumeKnob({ volume, audioRef }: { volume: number; audioRef: MutableRefObject<SmoothedAudioData> }) {
  const groupRef = useRef<THREE.Group>(null)
  useFrame(() => {
    const d = audioRef.current
    if (groupRef.current) {
      const targetRot = -0.5 * Math.PI + volume * Math.PI
      groupRef.current.rotation.z = lerp(groupRef.current.rotation.z, targetRot, 0.2)
      const s = 1 + d.highFreqAvg * 0.12
      groupRef.current.scale.setScalar(s)
    }
  })
  return (
    <group position={[2.8, -1.9, 0.2]}>
      <mesh>
        <boxGeometry args={[1.1, 1.1, 0.25]} />
        <meshPhysicalMaterial
          color="#111429"
          roughness={0.3}
          metalness={0.15}
          transmission={0.5}
          thickness={0.3}
          transparent
          opacity={0.8}
        />
      </mesh>
      <group ref={groupRef} position={[0, 0, 0.14]}>
        <mesh>
          <cylinderGeometry args={[0.42, 0.42, 0.08, 48]} />
          <meshPhysicalMaterial
            color="#cfd8ff"
            metalness={0.6}
            roughness={0.25}
            emissive="#4f6bff"
            emissiveIntensity={0.25}
          />
        </mesh>
        <mesh position={[0.3, 0, 0]}>
          <boxGeometry args={[0.15, 0.04, 0.1]} />
          <meshStandardMaterial color="#ffffff" emissive="#ffffff" emissiveIntensity={1} />
        </mesh>
      </group>
    </group>
  )
}
