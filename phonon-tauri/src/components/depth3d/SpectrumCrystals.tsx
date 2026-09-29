// 频谱晶体环 — InstancedMesh 版本（性能优化：60 draw calls → 3）
// 三圈晶体随频谱高低生长，低频内圈矮粗，高频外圈细长
import { useRef, useMemo } from 'react'
import { useFrame } from '@react-three/fiber'
import * as THREE from 'three'
import type { MutableRefObject } from 'react'
import type { SmoothedAudioData } from './audioBridge'

interface Props {
  audioRef: MutableRefObject<SmoothedAudioData>
}

const RINGS = [
  { count: 10, radius: 3.5, baseHeight: 1.4, baseWidth: 0.1, freqRange: [0, 12] },   // 内圈 - 低频
  { count: 18, radius: 5.8, baseHeight: 2.2, baseWidth: 0.07, freqRange: [12, 36] },  // 中圈 - 中频
  { count: 32, radius: 8.0, baseHeight: 3.2, baseWidth: 0.045, freqRange: [36, 64] },  // 外圈 - 高频
]

// HSL 色相偏移（每圈不同颜色）
const RING_HUES = [0.58, 0.72, 0.08] // 冰蓝 → 紫罗兰 → 金橙

export function SpectrumCrystals({ audioRef }: Props) {
  const instancedRefs = useRef<(THREE.InstancedMesh | null)[]>([])
  const targetHeightsRef = useRef<Float32Array[]>([])
  const currentHeightsRef = useRef<Float32Array[]>([])
  const beatFlashRef = useRef(0)
  const lastBeatRef = useRef(false)

  // 每根晶体的随机偏移（天然生长感）
  const randOffsetsRef = useRef<{ heightMul: number; widthMul: number; tiltX: number; tiltZ: number; yOffset: number }[][]>([])

  // 初始化高度数组和随机偏移
  useMemo(() => {
    targetHeightsRef.current = RINGS.map(r => new Float32Array(r.count))
    currentHeightsRef.current = RINGS.map(r => new Float32Array(r.count))
    randOffsetsRef.current = RINGS.map(r =>
      Array.from({ length: r.count }, () => ({
        heightMul: 0.75 + Math.random() * 0.5, // 0.75~1.25
        widthMul: 0.8 + Math.random() * 0.4,  // 0.8~1.2
        tiltX: (Math.random() - 0.5) * 0.25,  // -0.25~0.25 rad
        tiltZ: (Math.random() - 0.5) * 0.25,  // -0.25~0.25 rad
        yOffset: (Math.random() - 0.5) * 0.3, // -0.15~0.15
      }))
    )
  }, [])

  // 晶体几何体（共享一个）
  const crystalGeo = useMemo(() => new THREE.CylinderGeometry(0.4, 1, 1, 6, 1), [])

  // 每圈的材质（共享一个）
  const ringMaterials = useMemo(() => RINGS.map((_, ringIdx) => {
    const hue = RING_HUES[ringIdx]
    return new THREE.MeshStandardMaterial({
      color: new THREE.Color().setHSL(hue, 0.85, 0.22),
      emissive: new THREE.Color().setHSL(hue, 1.0, 0.42),
      emissiveIntensity: 0.15,
      metalness: 0.15,
      roughness: 0.4,
    })
  }), [])

  // 预计算每个实例的基础位置/旋转/基础缩放
  const baseMatrices = useMemo(() => {
    return RINGS.map((ring, ringIdx) => {
      const matrices: THREE.Matrix4[] = []
      const quat = new THREE.Quaternion()
      const euler = new THREE.Euler()
      const pos = new THREE.Vector3()
      const scl = new THREE.Vector3()

      for (let i = 0; i < ring.count; i++) {
        const angle = (i / ring.count) * Math.PI * 2
        const rand = randOffsetsRef.current[ringIdx][i]
        pos.set(
          Math.cos(angle) * ring.radius,
          ring.baseHeight / 2 - 5.5 + rand.yOffset,
          Math.sin(angle) * ring.radius,
        )
        euler.set(rand.tiltX, 0, rand.tiltZ)
        quat.setFromEuler(euler)
        scl.set(
          ring.baseWidth * rand.widthMul,
          ring.baseHeight * rand.heightMul,
          ring.baseWidth * rand.widthMul,
        )
        const m = new THREE.Matrix4()
        m.compose(pos, quat, scl)
        matrices.push(m)
      }
      return matrices
    })
  }, [crystalGeo])

  // 每帧更新晶体高度（修改实例矩阵的 y 缩放）
  useFrame(() => {
    const d = audioRef.current
    const spectrum = d.spectrum64
    const beatFlash = beatFlashRef.current

    // beat 闪烁
    if (d.beat && !lastBeatRef.current) beatFlashRef.current = 1
    beatFlashRef.current *= 0.85
    lastBeatRef.current = d.beat

    for (let ringIdx = 0; ringIdx < RINGS.length; ringIdx++) {
      const ring = RINGS[ringIdx]
      const inst = instancedRefs.current[ringIdx]
      if (!inst) continue

      const targetHeights = targetHeightsRef.current[ringIdx]
      const currentHeights = currentHeightsRef.current[ringIdx]
      const [f0, f1] = ring.freqRange
      const baseMatricesArr = baseMatrices[ringIdx]

      // 计算目标高度
      for (let i = 0; i < ring.count; i++) {
        // 将晶体索引映射到频谱频段
        const t = i / ring.count
        const binIdx = Math.floor(f0 + t * (f1 - f0))
        const specVal = spectrum[Math.min(63, Math.max(0, binIdx))] || 0
        // 目标高度倍率（1~3倍）
        targetHeights[i] = 1 + specVal * 2.5
      }

      // 平滑 + 更新实例矩阵
      const tmpMat = new THREE.Matrix4()
      const tmpPos = new THREE.Vector3()
      const tmpQuat = new THREE.Quaternion()
      const tmpScale = new THREE.Vector3()

      for (let i = 0; i < ring.count; i++) {
        // 平滑到目标高度
        currentHeights[i] += (targetHeights[i] - currentHeights[i]) * 0.18

        // 从基础矩阵分解，修改 y 缩放
        baseMatricesArr[i].decompose(tmpPos, tmpQuat, tmpScale)
        const heightScale = currentHeights[i]
        tmpScale.y = Math.abs(tmpScale.y) * heightScale

        // beat 时加一点发光闪烁（通过缩放微调模拟）
        const pulse = 1 + beatFlash * 0.08
        tmpScale.x *= pulse
        tmpScale.z *= pulse

        tmpMat.compose(tmpPos, tmpQuat, tmpScale)
        inst.setMatrixAt(i, tmpMat)
      }
      inst.instanceMatrix.needsUpdate = true

      // 更新发光强度
      const mat = inst.material as THREE.MeshStandardMaterial
      const avg = ringIdx < 2 ? d.lowFreqAvg : d.highFreqAvg
      mat.emissiveIntensity = 0.15 + avg * 0.6 + beatFlash * 0.5
    }
  })

  return (
    <group>
      {RINGS.map((ring, ringIdx) => (
        <instancedMesh
          key={ringIdx}
          ref={(el) => { instancedRefs.current[ringIdx] = el }}
          args={[crystalGeo, ringMaterials[ringIdx], ring.count]}
          castShadow
          receiveShadow
        />
      ))}
    </group>
  )
}
