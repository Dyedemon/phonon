// 频谱柱切片：64 根 3D Box 柱（元素级），height 与 spec 值线性映射，
// 顶面 emissive 颜色根据 chroma hue 呼吸
import { useMemo, useRef } from 'react'
import { useFrame } from '@react-three/fiber'
import * as THREE from 'three'
import type { MutableRefObject } from 'react'
import type { SmoothedAudioData } from './audioBridge'
import { lerp } from './audioBridge'

interface Props {
  audioRef: MutableRefObject<SmoothedAudioData>
  position?: [number, number, number]
  rotation?: [number, number, number]
  count?: number
  maxHeight?: number
  width?: number
}

export default function SpectrumSlices({
  audioRef, position = [0, -0.8, -6],
  rotation = [-0.22, 0, 0], count = 64,
  maxHeight = 4.5, width = 11,
}: Props) {
  const groupRef = useRef<THREE.Group>(null)
  const meshRefs = useRef<THREE.Mesh[]>([])
  const heights = useMemo(() => new Float32Array(count), [count])
  const targetH = useMemo(() => new Float32Array(count), [count])
  const hueRef = useRef(220)

  const spacing = width / count
  const barW = spacing * 0.78
  const { geometries, edgeGeometries } = useMemo(() => {
    const g = new Array(count) as THREE.BoxGeometry[]
    const e = new Array(count) as THREE.EdgesGeometry[]
    for (let i = 0; i < count; i++) {
      g[i] = new THREE.BoxGeometry(barW, 1, barW * 1.2)
      e[i] = new THREE.EdgesGeometry(g[i], 20)
    }
    return { geometries: g, edgeGeometries: e }
  }, [count, barW])

  useFrame(() => {
    const d = audioRef.current
    const spec = d.spectrum64
    const rms = d.rms
    for (let i = 0; i < count; i++) {
      const raw = spec[i] ?? 0
      // 非线性映射（给低频补偿一点），再夹逼
      const bias = 1 + (1 - i / count) * 0.35
      targetH[i] = Math.min(maxHeight, 0.06 + Math.pow(raw, 0.8) * bias * maxHeight * 1.1)
      heights[i] = lerp(heights[i], targetH[i], 0.32)
      const m = meshRefs.current[i]
      if (!m) continue
      m.scale.y = heights[i]
      m.position.y = heights[i] * 0.5
      const mat = m.material as THREE.MeshPhysicalMaterial
      const intensity = 0.1 + Math.min(1.4, raw * 2.5 + rms * 0.8)
      mat.emissiveIntensity = intensity
    }

    // 根据 chroma 更新 hue（lerp）
    const targetHue = (() => {
      const c = d.chroma12
      let maxI = 0, maxV = 0
      for (let i = 0; i < 12; i++) if (c[i] > maxV) { maxV = c[i]; maxI = i }
      if (maxV < 0.001) return 220
      return (maxI * 30) % 360
    })()
    hueRef.current = lerp(hueRef.current, targetHue, 0.08)

    // 每 ~8 根 bar 共享颜色更新（节省 CPU）
    const sharedMat0 = meshRefs.current[0]?.material as THREE.MeshPhysicalMaterial | undefined
    if (sharedMat0) {
      const hue = hueRef.current
      const c = new THREE.Color(`hsl(${hue}, 75%, 62%)`)
      sharedMat0.emissive.copy(c)
    }
    // 整体 slice 随 lowFreq 呼吸 scale & tilt
    if (groupRef.current) {
      const s = 1 + d.lowFreqAvg * 0.06
      groupRef.current.scale.set(s, s, s)
    }
  })

  // 预分配 mesh + 复用同一个材质
  const sharedMat = useMemo(() => new THREE.MeshPhysicalMaterial({
    color: '#ffffff',
    transparent: true,
    opacity: 0.82,
    transmission: 0.65,
    thickness: 0.35,
    roughness: 0.18,
    metalness: 0.05,
    ior: 1.45,
    clearcoat: 0.9,
    clearcoatRoughness: 0.1,
    side: THREE.DoubleSide,
    emissive: new THREE.Color('#4f6bff'),
    emissiveIntensity: 0.1,
  }), [])

  const edgeMat = useMemo(() => new THREE.LineBasicMaterial({
    color: '#b8d0ff', transparent: true, opacity: 0.9,
  }), [])

  return (
    <group position={position} rotation={rotation} ref={groupRef}>
      {geometries.map((geom, i) => {
        const x = -width / 2 + spacing * (i + 0.5)
        return (
          <group key={i} position={[x, 0, 0]}>
            <mesh
              ref={(el) => { meshRefs.current[i] = el as any }}
              castShadow
              receiveShadow
              geometry={geom}
              material={sharedMat}
            />
            <lineSegments geometry={edgeGeometries[i]} material={edgeMat} />
          </group>
        )
      })}
    </group>
  )
}
