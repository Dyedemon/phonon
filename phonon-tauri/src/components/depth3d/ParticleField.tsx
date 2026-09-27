// 粒子星云系统（3000 Points，BufferGeometry）+ 地面网格 ShaderMaterial
import { useMemo, useRef } from 'react'
import { useFrame } from '@react-three/fiber'
import * as THREE from 'three'
import type { MutableRefObject } from 'react'
import type { SmoothedAudioData } from './audioBridge'
import { lerp } from './audioBridge'

const POINT_COUNT = 3000

// ========= 粒子星云 =========
interface ParticleProps {
  audioRef: MutableRefObject<SmoothedAudioData>
}

export function ParticleField({ audioRef }: ParticleProps) {
  const pointsRef = useRef<THREE.Points>(null)
  const burstAccumRef = useRef(0)
  const positions = useMemo(() => {
    const arr = new Float32Array(POINT_COUNT * 3)
    for (let i = 0; i < POINT_COUNT; i++) {
      // 以中心为原点在球壳内均匀分布
      const r = 12 + Math.random() * 20
      const theta = Math.random() * Math.PI * 2
      const phi = Math.acos(2 * Math.random() - 1)
      arr[i * 3]     = r * Math.sin(phi) * Math.cos(theta)
      arr[i * 3 + 1] = r * Math.sin(phi) * Math.sin(theta)
      arr[i * 3 + 2] = r * Math.cos(phi) - 4
    }
    return arr
  }, [])

  const velocities = useMemo(() => {
    const arr = new Float32Array(POINT_COUNT * 3)
    for (let i = 0; i < POINT_COUNT; i++) {
      arr[i * 3]     = (Math.random() - 0.5) * 0.02
      arr[i * 3 + 1] = (Math.random() - 0.5) * 0.02
      arr[i * 3 + 2] = (Math.random() - 0.5) * 0.02
    }
    return arr
  }, [])

  const initPositions = useMemo(() => positions.slice(), [positions])

  useFrame(({ clock }) => {
    const d = audioRef.current
    const geo = pointsRef.current?.geometry as THREE.BufferGeometry | undefined
    if (!geo) return
    const posAttr = geo.attributes.position as THREE.BufferAttribute
    const arr = posAttr.array as Float32Array
    const t = clock.elapsedTime

    // Onset burst：瞬间把粒子向外推（脉冲）
    const burst = d.onset
    if (burst > 0.05) burstAccumRef.current = Math.min(1, burstAccumRef.current + burst * 0.8)
    const burstScale = 1 + burstAccumRef.current * 0.4
    burstAccumRef.current *= 0.9

    const rmsScale = 1 + d.rms * 0.12

    for (let i = 0; i < POINT_COUNT; i++) {
      const i3 = i * 3
      // 缓慢绕 Y 轴旋转 + 回到初始位置的拉力（spring）
      const ox = initPositions[i3]
      const oy = initPositions[i3 + 1]
      const oz = initPositions[i3 + 2]
      const cx = arr[i3]
      const cy = arr[i3 + 1]
      const cz = arr[i3 + 2]

      const spring = 0.01
      arr[i3]     = cx + (ox - cx) * spring + Math.sin(t * 0.3 + i * 0.01) * 0.01
      arr[i3 + 1] = cy + (oy - cy) * spring + Math.cos(t * 0.25 + i * 0.015) * 0.01
      arr[i3 + 2] = cz + (oz - cz) * spring + velocities[i3 + 2] * 2

      // burst & rms 整体从中心放大
      const s = burstScale * rmsScale
      if (s !== 1) {
        const nx = ox * s
        const ny = oy * s
        const nz = oz * s
        arr[i3]     = lerp(arr[i3],     nx, 0.05)
        arr[i3 + 1] = lerp(arr[i3 + 1], ny, 0.05)
        arr[i3 + 2] = lerp(arr[i3 + 2], nz, 0.05)
      }
    }
    posAttr.needsUpdate = true

    // 整体 group 缓慢旋转
    const pr = pointsRef.current
    if (pr) {
      pr.rotation.y = t * 0.02
      pr.rotation.x = Math.sin(t * 0.05) * 0.05
      // 颜色 & size 随 rms
      const mat = pr.material as THREE.ShaderMaterial
      mat.uniforms.uTime.value = t
      mat.uniforms.uRms.value = d.rms
      mat.uniforms.uOnset.value = d.onset
      mat.uniforms.uHue.value = (() => {
        const c = d.chroma12
        let maxI = 0, maxV = 0
        for (let i = 0; i < 12; i++) if (c[i] > maxV) { maxV = c[i]; maxI = i }
        return (maxI * 30) / 360
      })()
    }
  })

  // 自定义 shader：支持 HSL 色相 + rms 尺寸 + 加性混合
  const shaderMat = useMemo(() => new THREE.ShaderMaterial({
    uniforms: {
      uTime: { value: 0 },
      uRms: { value: 0 },
      uOnset: { value: 0 },
      uHue: { value: 0.6 },
    },
    transparent: true,
    depthWrite: false,
    blending: THREE.AdditiveBlending,
    vertexShader: /* glsl */`
      attribute float aSize;
      uniform float uTime;
      uniform float uRms;
      uniform float uOnset;
      varying float vSize;
      varying float vRms;
      void main() {
        vRms = uRms;
        vSize = aSize * (1.0 + uRms * 2.5 + uOnset * 3.0);
        vec4 mv = modelViewMatrix * vec4(position, 1.0);
        gl_Position = projectionMatrix * mv;
        gl_PointSize = vSize * (240.0 / -mv.z);
      }
    `,
    fragmentShader: /* glsl */`
      precision highp float;
      uniform float uHue;
      uniform float uRms;
      uniform float uOnset;
      varying float vRms;
      vec3 hsv2rgb(vec3 c) {
        vec4 K = vec4(1.0, 2.0/3.0, 1.0/3.0, 3.0);
        vec3 p = abs(fract(c.xxx + K.xyz) * 6.0 - K.www);
        return c.z * mix(K.xxx, clamp(p - K.xxx, 0.0, 1.0), c.y);
      }
      void main() {
        vec2 uv = gl_PointCoord - 0.5;
        float d = length(uv);
        if (d > 0.5) discard;
        float glow = smoothstep(0.5, 0.0, d);
        vec3 col = hsv2rgb(vec3(uHue, 0.8, 0.9));
        float alpha = glow * (0.35 + vRms * 0.9 + uOnset * 1.2);
        gl_FragColor = vec4(col * (0.6 + vRms * 0.8), alpha);
      }
    `,
  }), [])

  const geometry = useMemo(() => {
    const g = new THREE.BufferGeometry()
    g.setAttribute('position', new THREE.BufferAttribute(positions, 3))
    const sizes = new Float32Array(POINT_COUNT)
    for (let i = 0; i < POINT_COUNT; i++) sizes[i] = 0.6 + Math.random() * 1.6
    g.setAttribute('aSize', new THREE.BufferAttribute(sizes, 1))
    return g
  }, [positions])

  return <points ref={pointsRef} geometry={geometry} material={shaderMat} />
}

// ========= 地面网格（带低频震波 Shader） =========
interface GridProps {
  audioRef: MutableRefObject<SmoothedAudioData>
}

export function FloorGrid({ audioRef }: GridProps) {
  const meshRef = useRef<THREE.Mesh>(null)
  const ringRef = useRef<THREE.Mesh>(null)
  const lowSmRef = useRef(0)

  useFrame(({ clock }) => {
    const d = audioRef.current
    lowSmRef.current = lerp(lowSmRef.current, d.lowFreqAvg, 0.12)
    if (meshRef.current) {
      const mat = meshRef.current.material as THREE.ShaderMaterial
      mat.uniforms.uTime.value = clock.elapsedTime
      mat.uniforms.uLow.value = lowSmRef.current
      mat.uniforms.uRms.value = d.rms
      mat.uniforms.uOnset.value = d.onset
    }
    // 从中心辐射的震波圆环
    if (ringRef.current) {
      const s = 2 + (clock.elapsedTime % 4) * 4 + lowSmRef.current * 8
      ringRef.current.scale.set(s, s, s)
      const mat = ringRef.current.material as THREE.Material & { uniforms: any }
      if (mat.uniforms) {
        mat.uniforms.uAlpha.value = Math.max(0, 0.9 - (clock.elapsedTime % 4) / 4 - 0.1)
        mat.uniforms.uThick.value = 0.02 + lowSmRef.current * 0.15
      }
    }
  })

  const gridMat = useMemo(() => new THREE.ShaderMaterial({
    uniforms: {
      uTime: { value: 0 },
      uLow: { value: 0 },
      uRms: { value: 0 },
      uOnset: { value: 0 },
    },
    transparent: true,
    depthWrite: false,
    side: THREE.DoubleSide,
    vertexShader: /* glsl */`
      varying vec2 vUv;
      varying vec3 vPos;
      uniform float uTime;
      uniform float uLow;
      void main() {
        vUv = uv;
        vec3 p = position;
        float r = length(p.xz);
        // 低频 → 顶点上下波动（径向正弦波纹）
        float wave = sin(r * 0.8 - uTime * 1.4) * 0.15 * (0.2 + uLow * 2.0)
                   + sin(r * 2.4 - uTime * 3.1) * 0.05 * (0.4 + uLow * 1.5);
        p.y += wave;
        vPos = p;
        gl_Position = projectionMatrix * modelViewMatrix * vec4(p, 1.0);
      }
    `,
    fragmentShader: /* glsl */`
      varying vec2 vUv;
      varying vec3 vPos;
      uniform float uTime;
      uniform float uLow;
      uniform float uRms;
      uniform float uOnset;
      float grid(vec2 uv, float size) {
        vec2 g = abs(fract(uv * size - 0.5) - 0.5) / fwidth(uv * size);
        return 1.0 - min(min(g.x, g.y), 1.0);
      }
      void main() {
        vec2 uv = vUv - 0.5;
        float r = length(uv);
        float big = grid(vUv, 20.0);
        float small = grid(vUv, 5.0);
        vec3 bigCol = vec3(0.45, 0.6, 1.0);
        vec3 smallCol = vec3(0.35, 0.5, 0.9);
        vec3 col = mix(bigCol * 0.6, bigCol, big)
                 + smallCol * small * 0.4;
        // 外到内的渐变（边缘更透明）
        float edge = smoothstep(0.5, 0.0, r);
        float centerBoost = smoothstep(0.5, 0.0, r * 2.0) * (0.3 + uRms * 1.2);
        col += vec3(0.5, 0.6, 1.0) * centerBoost;
        float alpha = edge * (0.25 + uLow * 0.9 + uOnset * 0.6);
        gl_FragColor = vec4(col, alpha);
      }
    `,
  }), [])

  const ringMat = useMemo(() => {
    return new THREE.ShaderMaterial({
      uniforms: {
        uAlpha: { value: 0 },
        uThick: { value: 0.03 },
      },
      transparent: true,
      depthWrite: false,
      side: THREE.DoubleSide,
      vertexShader: /* glsl */`
        varying vec2 vUv;
        void main() {
          vUv = uv;
          gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
        }
      `,
      fragmentShader: /* glsl */`
        varying vec2 vUv;
        uniform float uAlpha;
        uniform float uThick;
        void main() {
          vec2 uv = vUv - 0.5;
          float d = length(uv);
          float ring = 1.0 - smoothstep(uThick, uThick + 0.008, abs(d - 0.48));
          if (ring < 0.001) discard;
          vec3 col = vec3(0.55, 0.75, 1.0);
          gl_FragColor = vec4(col, uAlpha * ring);
        }
      `,
    })
  }, [])

  return (
    <group position={[0, -4, -2]} rotation={[-Math.PI / 2, 0, 0]}>
      {/* 大圆盘 */}
      <mesh ref={meshRef}>
        <planeGeometry args={[40, 40, 160, 160]} />
        <primitive object={gridMat} attach="material" />
      </mesh>
      {/* 震波圆环（repeatedly scale） */}
      <mesh ref={ringRef}>
        <planeGeometry args={[1, 1]} />
        <primitive object={ringMat} attach="material" />
      </mesh>
      <mesh ref={ringRef as any}>
        <planeGeometry args={[1, 1]} />
        <primitive object={ringMat} attach="material" />
      </mesh>
    </group>
  )
}
