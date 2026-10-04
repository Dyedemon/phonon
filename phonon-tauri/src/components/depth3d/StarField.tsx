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
  const galaxyRef = useRef<THREE.Mesh>(null)
  const hiSmRef = useRef(0)
  const initFadeRef = useRef(0)

  // ─── 银河底：一次性 CPU 烘焙到 CanvasTexture（零每帧采样成本之外的开销） ───
  // 背景原来是纯色，深空没有纵深。斜向银河带 + 尘埃暗纹 + 密集暗星，
  // 所有绘制水平补一份（x±w）保证 equirect 左右无缝
  const galaxyTex = useMemo(() => {
    const w = 2048
    const h = 1024
    const canvas = document.createElement('canvas')
    canvas.width = w
    canvas.height = h
    const ctx = canvas.getContext('2d')!
    ctx.fillStyle = '#050718'
    ctx.fillRect(0, 0, w, h)

    const bandY = (x: number) => h * 0.5 + Math.sin((x / w) * Math.PI * 2 * 1.5) * h * 0.1
    const blob = (x: number, y: number, r: number, color: string) => {
      for (const dx of [-w, 0, w]) {
        const g = ctx.createRadialGradient(x + dx, y, 0, x + dx, y, r)
        g.addColorStop(0, color)
        g.addColorStop(1, 'rgba(0,0,0,0)')
        ctx.fillStyle = g
        ctx.fillRect(x + dx - r, y - r, r * 2, r * 2)
      }
    }

    // 银河带：沿正弦曲线叠柔光斑（蓝紫青三族）
    const tints = ['rgba(110,130,255,0.05)', 'rgba(150,120,255,0.045)', 'rgba(90,170,230,0.04)', 'rgba(180,150,255,0.035)']
    for (let i = 0; i < 260; i++) {
      const x = (i / 260) * w + Math.random() * 14
      const y = bandY(x) + (Math.random() - 0.5) * h * 0.16
      blob(x, y, 50 + Math.random() * 150, tints[Math.floor(Math.random() * tints.length)])
    }
    // 尘埃暗纹
    for (let i = 0; i < 50; i++) {
      const x = Math.random() * w
      const y = bandY(x) + (Math.random() - 0.5) * h * 0.1
      blob(x, y, 30 + Math.random() * 90, 'rgba(4,6,20,0.28)')
    }
    // 密集暗星：六成集中在带附近
    for (let i = 0; i < 2600; i++) {
      const near = Math.random() < 0.6
      const x = Math.random() * w
      const y = near ? bandY(x) + (Math.random() - 0.5) * h * 0.22 : Math.random() * h
      ctx.fillStyle = `rgba(210,220,255,${0.12 + Math.random() * 0.35})`
      const s = Math.random() < 0.85 ? 1 : 2
      for (const dx of [-w, 0, w]) ctx.fillRect(x + dx, y, s, s)
    }
    // 少量亮星
    for (let i = 0; i < 70; i++) {
      const x = Math.random() * w
      const y = Math.random() * h
      ctx.fillStyle = `rgba(235,240,255,${0.5 + Math.random() * 0.4})`
      for (const dx of [-w, 0, w]) ctx.fillRect(x + dx, y, 2, 2)
    }

    const tex = new THREE.CanvasTexture(canvas)
    tex.colorSpace = THREE.SRGBColorSpace
    return tex
  }, [])

  useEffect(() => () => { galaxyTex.dispose() }, [galaxyTex])

  const qScale = qualityScale(quality)
  // 1000/150：用户反馈最小星星太多；尺寸下限同步抬高（0.45 起），
  // 最小的一档砍得最狠
  const STAR_COUNT = Math.floor(1000 * qScale)
  const TWINKLE_COUNT = Math.floor(150 * qScale)

  // 远景星星（常亮，微闪已按用户要求移除；phase 属性随之删除）
  const starsGeo = useMemo(() => {
    const geo = new THREE.BufferGeometry()
    const positions = new Float32Array(STAR_COUNT * 3)
    const sizes = new Float32Array(STAR_COUNT)

    for (let i = 0; i < STAR_COUNT; i++) {
      // 球形分布在远处
      const r = 40 + Math.random() * 30
      const theta = Math.random() * Math.PI * 2
      const phi = Math.acos(2 * Math.random() - 1)
      positions[i * 3] = r * Math.sin(phi) * Math.cos(theta)
      positions[i * 3 + 1] = r * Math.sin(phi) * Math.sin(theta)
      positions[i * 3 + 2] = r * Math.cos(phi)

      sizes[i] = 0.45 + Math.random() * 0.55
    }

    geo.setAttribute('position', new THREE.BufferAttribute(positions, 3))
    geo.setAttribute('size', new THREE.BufferAttribute(sizes, 1))
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
      uniform float uIntensity;
      uniform float uPixelRatio;
      varying float vAlpha;

      void main() {
        // 远景星层常亮：用户两轮反馈后彻底去掉闪烁（音乐响应保留在 uIntensity）
        vec4 mvPosition = modelViewMatrix * vec4(position, 1.0);
        // 近距星钳制点径 + 近距淡出，防失焦灰斑。
        // 远处亚像素星会因跨像素采样忽明忽暗（假闪烁）：设最小点径，
        // 理想尺寸不足时按能量守恒比例调暗——稳定不闪的暗星
        float depth = -mvPosition.z;
        float ps = size * uPixelRatio * 1.5 / depth * 80.0;
        float clamped = clamp(ps, 1.5, 5.0 * uPixelRatio);
        gl_PointSize = clamped;
        vAlpha = 0.85 * uIntensity * smoothstep(6.0, 20.0, depth) * min(ps / clamped, 1.0);
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
        // 亮星层只留极轻呼吸（幅度 0.15），不再有可察觉的"一闪一闪"。
        // 同样做最小点径 + 能量守恒调暗，杜绝亚像素跳变
        float twinkle = sin(uTime * 0.6 + phase) * 0.15 + 0.85;

        vec4 mvPosition = modelViewMatrix * vec4(position, 1.0);
        float depth = -mvPosition.z;
        float ps = size * uPixelRatio / depth * 120.0;
        float clamped = clamp(ps, 2.0, 7.0 * uPixelRatio);
        gl_PointSize = clamped;
        vAlpha = twinkle * uIntensity * smoothstep(8.0, 25.0, depth) * min(ps / clamped, 1.0);
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

    // 银河底极慢漂移
    if (galaxyRef.current) galaxyRef.current.rotation.y = t * 0.003

    // 缓慢整体旋转
    if (starsRef.current) {
      starsRef.current.rotation.y = t * 0.01
      const mat = starsRef.current.material as THREE.ShaderMaterial
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
      {/* 银河底：远球内侧一次烘焙贴图。fog 必须关——场景雾 far=65，95 距离会被整个吞掉 */}
      <mesh ref={galaxyRef}>
        <sphereGeometry args={[95, 32, 16]} />
        <meshBasicMaterial map={galaxyTex} side={THREE.BackSide} depthWrite={false} fog={false} toneMapped={false} />
      </mesh>
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
