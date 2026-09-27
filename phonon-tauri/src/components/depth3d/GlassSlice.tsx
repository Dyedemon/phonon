// 公共：玻璃切片 MeshPhysicalMaterial + 发光边框 LineSegments
// 所有 3D 界面切片都复用 <GlassSlice>，统一质感参数，保持与主界面玻璃一致
import { forwardRef, useMemo, type ReactNode } from 'react'
import * as THREE from 'three'

export interface GlassSliceProps {
  width: number
  height: number
  position?: [number, number, number]
  rotation?: [number, number, number]
  color?: string
  edgeColor?: string
  opacity?: number
  thickness?: number
  emissive?: string
  emissiveIntensity?: number
  children?: ReactNode
  castShadow?: boolean
  receiveShadow?: boolean
}

export const GlassSlice = forwardRef<THREE.Group, GlassSliceProps>(function GlassSlice(
  {
    width, height, position = [0, 0, 0], rotation = [0, 0, 0],
    color = '#ffffff', edgeColor = '#b4d2ff',
    opacity = 0.82, thickness = 0.4,
    emissive = '#4f6bff', emissiveIntensity = 0.05,
    children, castShadow = true, receiveShadow = true,
  },
  ref,
) {
  const geom = useMemo(() => new THREE.BoxGeometry(width, height, thickness), [width, height, thickness])
  const edgeGeom = useMemo(() => {
    const edges = new THREE.EdgesGeometry(geom, 25)
    return edges
  }, [geom])

  return (
    <group ref={ref} position={position} rotation={rotation}>
      <mesh castShadow={castShadow} receiveShadow={receiveShadow} geometry={geom}>
        <meshPhysicalMaterial
          color={color}
          transparent
          opacity={opacity}
          transmission={0.72}
          thickness={thickness}
          roughness={0.12}
          metalness={0.08}
          ior={1.5}
          clearcoat={1.0}
          clearcoatRoughness={0.08}
          side={THREE.DoubleSide}
          emissive={emissive}
          emissiveIntensity={emissiveIntensity}
        />
      </mesh>
      <lineSegments geometry={edgeGeom}>
        <lineBasicMaterial color={edgeColor} transparent opacity={0.85} />
      </lineSegments>
      {children}
    </group>
  )
})

/** 根据 chroma12 计算一个色相值（0..360）—— 取能量最强的 2~3 个音高加权平均 */
export function chromaHue(chroma: Float32Array): number {
  if (!chroma || chroma.length === 0) return 220
  let maxIdx = 0
  let maxV = 0
  for (let i = 0; i < chroma.length; i++) {
    if (chroma[i] > maxV) {
      maxV = chroma[i]
      maxIdx = i
    }
  }
  if (maxV < 0.0001) return 220
  // 12 音阶映射到色相环
  return (maxIdx * 30) % 360
}

/** 返回 HSL → CSS/THREE 色字符串 */
export function hsl(h: number, s = 70, l = 60, a = 1): string {
  return `hsla(${h}, ${s}%, ${l}%, ${a})`
}