// 流体地面 — 圆形液态平面，带冲击波、波纹、反射
// 低频驱动表面起伏 + 冲击波扩散；中频驱动细腻波纹
import { useMemo, useRef } from 'react'
import { useFrame } from '@react-three/fiber'
import * as THREE from 'three'
import type { MutableRefObject } from 'react'
import type { SmoothedAudioData } from './audioBridge'
import { lerp } from './audioBridge'

interface Props {
  audioRef: MutableRefObject<SmoothedAudioData>
}

const RADIUS = 18
const SEGMENTS = 200

export function FluidGround({ audioRef }: Props) {
  const meshRef = useRef<THREE.Mesh>(null)
  const shockWavesRef = useRef<{ time: number; strength: number }[]>([])
  const lastBeatRef = useRef(false)
  const lowSmRef = useRef(0)
  const midSmRef = useRef(0)

  useFrame(({ clock }) => {
    const d = audioRef.current
    const t = clock.elapsedTime

    lowSmRef.current = lerp(lowSmRef.current, d.lowFreqAvg, 0.08)
    midSmRef.current = lerp(midSmRef.current, d.rms, 0.1)

    // beat 时触发新的冲击波
    if (d.beat && !lastBeatRef.current) {
      shockWavesRef.current.push({ time: t, strength: 0.6 + d.onset * 0.8 })
      if (shockWavesRef.current.length > 6) shockWavesRef.current.shift()
    }
    lastBeatRef.current = d.beat

    // 清理过期冲击波
    shockWavesRef.current = shockWavesRef.current.filter(w => t - w.time < 3.5)

    if (meshRef.current) {
      const mat = meshRef.current.material as THREE.ShaderMaterial
      mat.uniforms.uTime.value = t
      mat.uniforms.uLow.value = lowSmRef.current
      mat.uniforms.uMid.value = midSmRef.current
      mat.uniforms.uOnset.value = d.onset

      // 更新冲击波数组
      const shocks = mat.uniforms.uShocks.value as THREE.Vector4[]
      for (let i = 0; i < 6; i++) {
        const w = shockWavesRef.current[i]
        if (w) {
          shocks[i].x = t - w.time      // age
          shocks[i].y = w.strength      // strength
          shocks[i].z = 0               // unused
          shocks[i].w = 0               // unused
        } else {
          shocks[i].set(-1, 0, 0, 0)    // 负数 age = 无效
        }
      }
    }
  })

  const groundMat = useMemo(() => {
    const shocks = new Array(6).fill(null).map(() => new THREE.Vector4(-1, 0, 0, 0))
    return new THREE.ShaderMaterial({
      uniforms: {
        uTime: { value: 0 },
        uLow: { value: 0 },
        uMid: { value: 0 },
        uOnset: { value: 0 },
        uShocks: { value: shocks },
        uRadius: { value: RADIUS },
      },
      transparent: true,
      depthWrite: false,
      side: THREE.DoubleSide,
      vertexShader: /* glsl */`
        uniform float uTime;
        uniform float uLow;
        uniform float uMid;
        uniform vec4 uShocks[6];
        uniform float uRadius;

        varying vec3 vPos;
        varying vec3 vNormal;
        varying float vHeight;

        void main() {
          vec3 p = position;
          float r = length(p.xz);

          // 基础低频波浪（大尺度、缓慢）
          float waveLow = sin(r * 0.35 - uTime * 0.8) * 0.25 * (0.15 + uLow * 1.8);
          waveLow += sin(r * 0.7 + uTime * 1.1) * 0.12 * (0.1 + uLow * 1.2);
          waveLow += sin(r * 1.5 - uTime * 2.0) * 0.05 * (0.2 + uMid * 1.5);

          // 中频细腻扰动
          float waveMid = sin(r * 4.0 + uTime * 3.0 + p.x * 2.0) * 0.025 * (0.3 + uMid * 2.0);
          waveMid += sin(r * 6.0 - uTime * 4.5 + p.z * 3.0) * 0.015 * (0.2 + uMid * 1.5);

          // 冲击波（从中心向外扩散的圆环隆起）
          float shockHeight = 0.0;
          for (int i = 0; i < 6; i++) {
            float age = uShocks[i].x;
            float strength = uShocks[i].y;
            if (age < 0.0) continue;
            float waveR = age * 6.0;  // 扩散速度
            float dist = abs(r - waveR);
            float envelope = exp(-age * 0.9) * strength;
            shockHeight += exp(-dist * dist * 1.8) * envelope * 0.6;
          }

          p.y += waveLow + waveMid + shockHeight;

          // 边缘下沉（圆形盘子效果，更平滑的过渡避免可见折线）
          float edgeFade = smoothstep(uRadius, uRadius * 0.35, r);
          p.y -= (1.0 - edgeFade) * 0.5;

          vPos = p;
          vHeight = p.y;
          vNormal = normal;

          gl_Position = projectionMatrix * modelViewMatrix * vec4(p, 1.0);
        }
      `,
      fragmentShader: /* glsl */`
        uniform float uTime;
        uniform float uLow;
        uniform float uMid;
        uniform float uOnset;
        uniform vec4 uShocks[6];
        uniform float uRadius;

        varying vec3 vPos;
        varying vec3 vNormal;
        varying float vHeight;

        // HSL 转 RGB
        vec3 hsv2rgb(vec3 c) {
          vec4 K = vec4(1.0, 2.0/3.0, 1.0/3.0, 3.0);
          vec3 p = abs(fract(c.xxx + K.xyz) * 6.0 - K.www);
          return c.z * mix(K.xxx, clamp(p - K.xxx, 0.0, 1.0), c.y);
        }

        void main() {
          float r = length(vPos.xz);

          // 基底色：几乎完全透明，只靠波点亮
          vec3 baseColor = vec3(0.003, 0.004, 0.01);

          // 低频时极微弱偏蓝
          vec3 lowTint = vec3(0.008, 0.012, 0.03) * uLow * 0.3;

          // 中心极微弱辉光
          float centerGlow = pow(smoothstep(uRadius * 0.2, 0.0, r), 3.0);
          vec3 glowColor = vec3(0.05, 0.09, 0.2) * centerGlow * (0.05 + uLow * 0.1);

          // 冲击波亮环（纤细、明亮、快速扩散）
          float shockGlow = 0.0;
          for (int i = 0; i < 6; i++) {
            float age = uShocks[i].x;
            float strength = uShocks[i].y;
            if (age < 0.0) continue;
            float waveR = age * 8.0;
            float dist = abs(r - waveR);
            float envelope = exp(-age * 1.2) * strength;
            shockGlow += exp(-dist * dist * 12.0) * envelope * 0.45;
          }
          vec3 shockColor = vec3(0.3, 0.45, 0.8) * shockGlow;

          // 波峰极细高光
          float waveHighlight = smoothstep(0.0, 0.03, abs(vHeight) * 30.0) * 0.04;
          vec3 highlightColor = vec3(0.35, 0.5, 0.8) * waveHighlight * (0.15 + uMid * 0.3);

          // 边缘淡出（边缘完全透明，只有中心区域可见）
          float edgeAlpha = smoothstep(uRadius, uRadius * 0.3, r);
          float alpha = edgeAlpha * (0.04 + uLow * 0.06 + uOnset * 0.05);

          // 颜色混合
          vec3 col = baseColor + lowTint + glowColor + shockColor + highlightColor;

          gl_FragColor = vec4(col, alpha);
        }
      `,
    })
  }, [])

  return (
    <group position={[0, -3.5, 0]} rotation={[-Math.PI / 2, 0, 0]}>
      {/* 流体地面主体 */}
      <mesh ref={meshRef}>
        <circleGeometry args={[RADIUS, SEGMENTS]} />
        <primitive object={groundMat} attach="material" />
      </mesh>
    </group>
  )
}
