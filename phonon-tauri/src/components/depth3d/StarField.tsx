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

// 远景星空 — 银河底 + 亮星呼吸层
// （原 750 颗静态小星层已整层删除——用户反馈"环绕的小星星太多"）
export function StarField({ audioRef, quality }: Props) {
  const twinkleRef = useRef<THREE.Points>(null)
  const hiSmRef = useRef(0)
  const initFadeRef = useRef(0)

  // ─── 银河底：一次性 CPU 烘焙到 CanvasTexture（零每帧采样成本之外的开销） ───
  // 设计原则（用户反馈）：深空要"空"，银河带只覆盖部分方位角且不环绕；
  // 三枚离散远星系涂斑作为点缀（双层高斯：柔光晕 + 亮核）。逐像素写
  // ImageData（canvas 渐变会被 Chromium 抖动渲染，量化出网状纹理）；
  // 噪声全部整数周期正弦，水平方向天然无缝
  const galaxyTex = useMemo(() => {
    const w = 2048
    const h = 1024
    // 1) 云雾层（1024×512 逐像素计算，2× 放大——分辨率翻倍 + 高频丝缕
    // 响应"银河有点糊"）
    const cw = 1024
    const ch = 512
    const cloud = document.createElement('canvas')
    cloud.width = cw
    cloud.height = ch
    const cctx = cloud.getContext('2d')!
    const img = cctx.createImageData(cw, ch)
    const px = img.data
    const base = [5, 7, 24]
    const tintA = [108, 128, 255]
    const tintB = [172, 134, 255]
    // 离散远星系涂斑：恢复原来的三枚（用户要求），并优化观感——
    // 每枚 = 大而柔的光晕 + 收紧的亮核（双层高斯），盘面渐散不像脏块
    const smudges = [
      { cx: 0.62, cy: 0.24, rx: 0.065, ry: 0.026, rot: -0.4, c: [150, 150, 225], a: 0.1 },
      { cx: 0.86, cy: 0.62, rx: 0.075, ry: 0.032, rot: 0.2, c: [118, 140, 225], a: 0.085 },
      { cx: 0.4, cy: 0.78, rx: 0.052, ry: 0.024, rot: -0.7, c: [145, 140, 228], a: 0.095 },
    ]
    for (let y = 0; y < ch; y++) {
      for (let x = 0; x < cw; x++) {
        const u = x / cw
        // 方位角环形距离：带中心在 u=0.5，可见范围 ~半个天空，两端收尾
        const az = Math.abs(((((u - 0.5) % 1) + 1.5) % 1) - 0.5)
        const azEnv = Math.exp(-(az * az) / (0.13 * 0.13))
        const by = ch * 0.42 + Math.sin(u * Math.PI * 2) * ch * 0.07
        const dy = (y - by) / (ch * 0.042)
        let a = Math.exp(-dy * dy) * azEnv
        // 平滑伪噪声：整数周期正弦叠加（水平无缝），高频项给丝缕感
        const n1 = Math.sin(u * Math.PI * 2 * 3 + y * 0.11) * 0.5 + 0.5
        const n2 = Math.sin(u * Math.PI * 2 * 7 + y * 0.23 + 1.7) * 0.5 + 0.5
        const n3 = Math.sin(u * Math.PI * 2 * 13 + y * 0.05 + 4.2) * 0.5 + 0.5
        const n4 = Math.sin(u * Math.PI * 2 * 21 + y * 0.31 + 2.4) * 0.5 + 0.5
        a *= (0.4 + 0.4 * n1 * n2 + 0.12 * n3 + 0.08 * n4) * 0.18
        // 尘埃暗纹（只落在带内）
        const dLane = (y - by - Math.sin(u * Math.PI * 2 * 2) * ch * 0.045) / (ch * 0.035)
        a -= Math.exp(-dLane * dLane) * 0.22 * Math.exp(-dy * dy) * azEnv
        a = Math.max(0, Math.min(1, a))
        // 涂斑与云带取最大值合成（避免叠亮）；双层高斯 = 柔和光晕 + 亮核
        let sr = 0, sg = 0, sb = 0, sa = 0
        for (const s of smudges) {
          const dxs = x - s.cx * cw
          const dys = y - s.cy * ch
          const cosr = Math.cos(s.rot)
          const sinr = Math.sin(s.rot)
          const ex = (dxs * cosr + dys * sinr) / (s.rx * cw)
          const ey = (-dxs * sinr + dys * cosr) / (s.ry * ch)
          const e2 = ex * ex + ey * ey
          const g = s.a * (0.55 * Math.exp(-e2) + 0.45 * Math.exp(-e2 * 4))
          if (g > sa) { sr = s.c[0]; sg = s.c[1]; sb = s.c[2]; sa = g }
        }
        const o = (y * cw + x) * 4
        if (sa > a) {
          px[o] = base[0] + (sr - base[0]) * sa
          px[o + 1] = base[1] + (sg - base[1]) * sa
          px[o + 2] = base[2] + (sb - base[2]) * sa
        } else {
          px[o] = base[0] + (tintA[0] + (tintB[0] - tintA[0]) * n3 - base[0]) * a
          px[o + 1] = base[1] + (tintA[1] + (tintB[1] - tintA[1]) * n3 - base[1]) * a
          px[o + 2] = base[2] + (tintA[2] + (tintB[2] - tintA[2]) * n3 - base[2]) * a
        }
        px[o + 3] = 255
      }
    }
    cctx.putImageData(img, 0, 0)

    // 2) 全分辨率合成：放大云雾（插值平滑）。
    // 烘焙暗星已全部移除（用户反馈：跟着银河环绕的小星星）——
    // 天空的离散星点只由亮星呼吸层与星流承担
    const canvas = document.createElement('canvas')
    canvas.width = w
    canvas.height = h
    const ctx = canvas.getContext('2d')!
    ctx.imageSmoothingEnabled = true
    ctx.imageSmoothingQuality = 'high'
    ctx.drawImage(cloud, 0, 0, w, h)

    const tex = new THREE.CanvasTexture(canvas)
    tex.colorSpace = THREE.SRGBColorSpace
    tex.anisotropy = 8
    return tex
  }, [])

  useEffect(() => () => { galaxyTex.dispose() }, [galaxyTex])

  const qScale = qualityScale(quality)
  // 150：静态小星层已整层删除（用户反馈），天空的点状元素只留呼吸亮星
  const TWINKLE_COUNT = Math.floor(150 * qScale)

  // 较亮的闪烁星（随高频响应）
  const twinkleGeo = useMemo(() => {
    const geo = new THREE.BufferGeometry()
    const positions = new Float32Array(TWINKLE_COUNT * 3)
    const sizes = new Float32Array(TWINKLE_COUNT)
    const phases = new Float32Array(TWINKLE_COUNT)
    const colors = new Float32Array(TWINKLE_COUNT * 3)

    for (let i = 0; i < TWINKLE_COUNT; i++) {
      // 星壳推到银河距离（80~94，紧贴银河球内侧）——用户要求亮星层
      // 静止且"迎合银河的最近处"；单粒加大补偿远距离
      const r = 80 + 14 * Math.random()
      const theta = Math.random() * Math.PI * 2
      const phi = Math.acos(2 * Math.random() - 1)
      positions[i * 3] = r * Math.sin(phi) * Math.cos(theta)
      positions[i * 3 + 1] = r * Math.sin(phi) * Math.sin(theta)
      positions[i * 3 + 2] = r * Math.cos(phi)

      sizes[i] = 1.2 + Math.random() * 1.8
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

    // 亮星呼吸层（静止——用户要求停掉环绕旋转）
    if (twinkleRef.current) {
      const mat = twinkleRef.current.material as THREE.ShaderMaterial
      mat.uniforms.uTime.value = t
      mat.uniforms.uIntensity.value = 0.3 + hi * 0.3
      mat.uniforms.uPixelRatio.value = dpr
    }
  })

  return (
    <group>
      {/* 银河底：远球内侧一次烘焙贴图，完全静止（用户要求不移动旋转）。
          fog 必须关——场景雾 far=65，95 距离会被整个吞掉 */}
      <mesh>
        <sphereGeometry args={[95, 32, 16]} />
        <meshBasicMaterial map={galaxyTex} side={THREE.BackSide} depthWrite={false} fog={false} toneMapped={false} />
      </mesh>
      <points ref={twinkleRef} geometry={twinkleGeo}>
        <primitive object={twinkleMat} attach="material" />
      </points>
    </group>
  )
}

function lerp(a: number, b: number, t: number) { return a + (b - a) * t }
