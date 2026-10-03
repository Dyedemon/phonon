import { useEffect, useRef, useMemo } from 'react'
import { useFrame } from '@react-three/fiber'
import type { MutableRefObject } from 'react'
import * as THREE from 'three'
import type { SmoothedAudioData } from './audioBridge'

interface Props {
  audioRef: MutableRefObject<SmoothedAudioData>
  quality?: string
}

function qualityScale(quality?: string) {
  switch (quality) {
    case 'low': return 0.4
    case 'mid': return 0.7
    case 'ultra': return 1.5
    case 'high':
    default: return 1.0
  }
}

// 远景星空 — 成千上万的小星星闪烁
export function StarField({ audioRef, quality }: Props) {
  const starsRef = useRef<THREE.Points>(null)
  const twinkleRef = useRef<THREE.Points>(null)
  const hiSmRef = useRef(0)
  const initFadeRef = useRef(0)

  const qScale = qualityScale(quality)
  const STAR_COUNT = Math.floor(2000 * qScale)
  const TWINKLE_COUNT = Math.floor(300 * qScale)

  // 远景星星（固定，微闪）
  const starsGeo = useMemo(() => {
    const geo = new THREE.BufferGeometry()
    const positions = new Float32Array(STAR_COUNT * 3)
    const sizes = new Float32Array(STAR_COUNT)
    const phases = new Float32Array(STAR_COUNT)

    for (let i = 0; i < STAR_COUNT; i++) {
      // 球形分布在远处
      const r = 40 + Math.random() * 30
      const theta = Math.random() * Math.PI * 2
      const phi = Math.acos(2 * Math.random() - 1)
      positions[i * 3] = r * Math.sin(phi) * Math.cos(theta)
      positions[i * 3 + 1] = r * Math.sin(phi) * Math.sin(theta)
      positions[i * 3 + 2] = r * Math.cos(phi)

      sizes[i] = 0.3 + Math.random() * 0.6
      phases[i] = Math.random() * Math.PI * 2
    }

    geo.setAttribute('position', new THREE.BufferAttribute(positions, 3))
    geo.setAttribute('size', new THREE.BufferAttribute(sizes, 1))
    geo.setAttribute('phase', new THREE.BufferAttribute(phases, 1))
    return geo
  }, [qScale])

  // 预设切换重建几何体时主动 dispose 旧的（与 NebulaCore 行为一致）
  useEffect(() => () => { starsGeo.dispose() }, [starsGeo])

  const starsMat = useMemo(() => new THREE.ShaderMaterial({
    transparent: true,
    depthWrite: false,
    blending: THREE.AdditiveBlending,
    uniforms: {
      uTime: { value: 0 },
      uIntensity: { value: 0.8 },
      uPixelRatio: { value: Math.min(window.devicePixelRatio, 2) },
    },
    vertexShader: /* glsl */`
      attribute float size;
      attribute float phase;
      uniform float uTime;
      uniform float uIntensity;
      uniform float uPixelRatio;
      varying float vAlpha;

      void main() {
        // 闪烁
        float twinkle = sin(uTime * 1.5 + phase) * 0.3 + 0.7;
        vAlpha = twinkle * uIntensity;

        vec4 mvPosition = modelViewMatrix * vec4(position, 1.0);
        gl_PointSize = size * uPixelRatio * 1.5 / -mvPosition.z * 80.0;
        gl_Position = projectionMatrix * mvPosition;
      }
    `,
    fragmentShader: /* glsl */`
      varying float vAlpha;
      void main() {
        vec2 uv = gl_PointCoord - 0.5;
        float d = length(uv);
        if (d > 0.5) discard;
        // 十字星芒感
        float star = smoothstep(0.5, 0.0, d);
        star = pow(star, 1.5);
        gl_FragColor = vec4(0.95, 0.97, 1.0, star * vAlpha);
      }
    `,
  }), [])

  // 较亮的闪烁星（随高频响应）
  const twinkleGeo = useMemo(() => {
    const geo = new THREE.BufferGeometry()
    const positions = new Float32Array(TWINKLE_COUNT * 3)
    const sizes = new Float32Array(TWINKLE_COUNT)
    const phases = new Float32Array(TWINKLE_COUNT)
    const colors = new Float32Array(TWINKLE_COUNT * 3)

    for (let i = 0; i < TWINKLE_COUNT; i++) {
      const r = 30 + Math.random() * 25
      const theta = Math.random() * Math.PI * 2
      const phi = Math.acos(2 * Math.random() - 1)
      positions[i * 3] = r * Math.sin(phi) * Math.cos(theta)
      positions[i * 3 + 1] = r * Math.sin(phi) * Math.sin(theta)
      positions[i * 3 + 2] = r * Math.cos(phi)

      sizes[i] = 1.0 + Math.random() * 1.5
      phases[i] = Math.random() * Math.PI * 2

      // 白偏蓝/偏粉
      const hue = 0.6 + Math.random() * 0.25
      const col = new THREE.Color().setHSL(hue, 0.3, 0.95)
      colors[i * 3] = col.r
      colors[i * 3 + 1] = col.g
      colors[i * 3 + 2] = col.b
    }

    geo.setAttribute('position', new THREE.BufferAttribute(positions, 3))
    geo.setAttribute('size', new THREE.BufferAttribute(sizes, 1))
    geo.setAttribute('phase', new THREE.BufferAttribute(phases, 1))
    geo.setAttribute('color', new THREE.BufferAttribute(colors, 3))
    return geo
  }, [qScale])

  useEffect(() => () => { twinkleGeo.dispose() }, [twinkleGeo])

  const twinkleMat = useMemo(() => new THREE.ShaderMaterial({
    transparent: true,
    depthWrite: false,
    blending: THREE.AdditiveBlending,
    uniforms: {
      uTime: { value: 0 },
      uIntensity: { value: 0.5 },
      uPixelRatio: { value: Math.min(window.devicePixelRatio, 2) },
    },
    vertexShader: /* glsl */`
      attribute float size;
      attribute float phase;
      attribute vec3 color;
      uniform float uTime;
      uniform float uIntensity;
      uniform float uPixelRatio;
      varying vec3 vColor;
      varying float vAlpha;

      void main() {
        vColor = color;
        float twinkle = sin(uTime * 3.0 + phase) * 0.5 + 0.5;
        vAlpha = twinkle * uIntensity;

        vec4 mvPosition = modelViewMatrix * vec4(position, 1.0);
        gl_PointSize = size * uPixelRatio / -mvPosition.z * 120.0;
        gl_Position = projectionMatrix * mvPosition;
      }
    `,
    fragmentShader: /* glsl */`
      varying vec3 vColor;
      varying float vAlpha;
      void main() {
        vec2 uv = gl_PointCoord - 0.5;
        float d = length(uv);
        if (d > 0.5) discard;
        // 柔和发光点
        float glow = smoothstep(0.5, 0.0, d);
        glow = pow(glow, 1.2);
        gl_FragColor = vec4(vColor, glow * vAlpha);
      }
    `,
  }), [])

  useFrame(({ clock, viewport }, delta) => {
    const d = audioRef.current
    const t = clock.elapsedTime
    // 启动淡入（delta 驱动，帧率无关）
    initFadeRef.current = Math.min(1, initFadeRef.current + delta * 0.72)
    const fade = initFadeRef.current
    hiSmRef.current = lerp(hiSmRef.current, d.highFreqAvg * fade, 0.08)
    const hi = hiSmRef.current
    // 粒子尺寸跟随画布实际 dpr（governor 降档时不突变）
    const dpr = viewport.dpr || 1

    // 缓慢整体旋转
    if (starsRef.current) {
      starsRef.current.rotation.y = t * 0.01
      const mat = starsRef.current.material as THREE.ShaderMaterial
      mat.uniforms.uTime.value = t
      mat.uniforms.uIntensity.value = 0.6 + hi * 0.25
      mat.uniforms.uPixelRatio.value = dpr
    }

    if (twinkleRef.current) {
      twinkleRef.current.rotation.y = -t * 0.015
      const mat = twinkleRef.current.material as THREE.ShaderMaterial
      mat.uniforms.uTime.value = t
      mat.uniforms.uIntensity.value = 0.3 + hi * 0.3
      mat.uniforms.uPixelRatio.value = dpr
    }
  })

  return (
    <group>
      <points ref={starsRef} geometry={starsGeo}>
        <primitive object={starsMat} attach="material" />
      </points>
      <points ref={twinkleRef} geometry={twinkleGeo}>
        <primitive object={twinkleMat} attach="material" />
      </points>
    </group>
  )
}

function lerp(a: number, b: number, t: number) { return a + (b - a) * t }
