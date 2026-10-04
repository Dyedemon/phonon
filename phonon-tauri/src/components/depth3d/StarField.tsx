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
  // 设计原则（用户反馈）：深空要"空"，银河带只覆盖部分方位角且不环绕；
  // 离散涂斑已全部删除——galaxy 缓慢漂移 + 相机自由 orbit，任何一枚
  // 都会时不时转到右上角被当成"脏块"（两轮反馈）。逐像素写 ImageData
  // （canvas 渐变会被 Chromium 抖动渲染，量化出网状纹理）；噪声全部
  // 整数周期正弦，水平方向天然无缝
  const galaxyTex = useMemo(() => {
    const w = 2048
    const h = 1024
    // 1) 云雾层（低分辨率逐像素计算，放大后插值平滑）
    const cw = 512
    const ch = 256
    const cloud = document.createElement('canvas')
    cloud.width = cw
    cloud.height = ch
    const cctx = cloud.getContext('2d')!
    const img = cctx.createImageData(cw, ch)
    const px = img.data
    const base = [5, 7, 24]
    const tintA = [108, 128, 255]
    const tintB = [172, 134, 255]
    // 离散远星系涂斑：只留两枚，放在银河带附近（用户要求）
    const smudges = [
      { cx: 0.42, cy: 0.5, rx: 0.03, ry: 0.013, rot: 0.4, c: [150, 160, 255], a: 0.22 },
      { cx: 0.6, cy: 0.4, rx: 0.024, ry: 0.011, rot: -0.35, c: [190, 170, 230], a: 0.2 },
    ]
    for (let y = 0; y < ch; y++) {
      for (let x = 0; x < cw; x++) {
        const u = x / cw
        // 方位角环形距离：带中心在 u=0.5，可见范围 ~半个天空，两端收尾
        const az = Math.abs(((((u - 0.5) % 1) + 1.5) % 1) - 0.5)
        const azEnv = Math.exp(-(az * az) / (0.13 * 0.13))
        const by = ch * 0.42 + Math.sin(u * Math.PI * 2) * ch * 0.07
        const dy = (y - by) / (ch * 0.07)
        let a = Math.exp(-dy * dy) * azEnv
        // 平滑伪噪声：整数周期正弦叠加（水平无缝）
        const n1 = Math.sin(u * Math.PI * 2 * 3 + y * 0.11) * 0.5 + 0.5
        const n2 = Math.sin(u * Math.PI * 2 * 7 + y * 0.23 + 1.7) * 0.5 + 0.5
        const n3 = Math.sin(u * Math.PI * 2 * 13 + y * 0.05 + 4.2) * 0.5 + 0.5
        a *= (0.45 + 0.4 * n1 * n2 + 0.15 * n3) * 0.8
        // 尘埃暗纹（只落在带内）
        const dLane = (y - by - Math.sin(u * Math.PI * 2 * 2) * ch * 0.045) / (ch * 0.035)
        a -= Math.exp(-dLane * dLane) * 0.5 * Math.exp(-dy * dy) * azEnv
        a = Math.max(0, Math.min(1, a))
        // 涂斑与云带取最大值合成（避免叠亮）
        let sr = 0, sg = 0, sb = 0, sa = 0
        for (const s of smudges) {
          const dxs = x - s.cx * cw
          const dys = y - s.cy * ch
          const cosr = Math.cos(s.rot)
          const sinr = Math.sin(s.rot)
          const ex = (dxs * cosr + dys * sinr) / (s.rx * cw)
          const ey = (-dxs * sinr + dys * cosr) / (s.ry * ch)
          const g = Math.exp(-(ex * ex + ey * ey)) * s.a
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

    // 2) 全分辨率合成：放大云雾（插值平滑）+ 锐利暗星
    const canvas = document.createElement('canvas')
    canvas.width = w
    canvas.height = h
    const ctx = canvas.getContext('2d')!
    ctx.imageSmoothingEnabled = true
    ctx.imageSmoothingQuality = 'high'
    ctx.drawImage(cloud, 0, 0, w, h)

    // 暗星：六成集中在带附近（按全分辨率尺度重算带位置；散布全天的
    // 孤星不受方位角包络限制——点状星不构成"包裹感"）
    const bandYFull = (x: number) => h * 0.42 + Math.sin((x / w) * Math.PI * 2) * h * 0.07
    for (let i = 0; i < 2600; i++) {
      const near = Math.random() < 0.6
      const x = Math.random() * w
      const y = near ? bandYFull(x) + (Math.random() - 0.5) * h * 0.2 : Math.random() * h
      ctx.fillStyle = 'rgba(210,220,255,' + (0.12 + Math.random() * 0.35) + ')'
      const s = Math.random() < 0.85 ? 1 : 2
      for (const dx of [-w, 0, w]) ctx.fillRect(x + dx, y, s, s)
    }
    for (let i = 0; i < 70; i++) {
      const x = Math.random() * w
      const y = Math.random() * h
      ctx.fillStyle = 'rgba(235,240,255,' + (0.5 + Math.random() * 0.4) + ')'
      for (const dx of [-w, 0, w]) ctx.fillRect(x + dx, y, 2, 2)
    }

    const tex = new THREE.CanvasTexture(canvas)
    tex.colorSpace = THREE.SRGBColorSpace
    tex.anisotropy = 8
    return tex
  }, [])

  useEffect(() => () => { galaxyTex.dispose() }, [galaxyTex])

  const qScale = qualityScale(quality)
  // 750/150：用户多轮反馈后继续收敛——满天小星点太碎，碎点让位给
  // 一条椭圆柱星流（见 streamGeo）
  const STAR_COUNT = Math.floor(750 * qScale)
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

  // ─── 星流：椭圆柱粒子流（用户要求以单一流代替碎星点/彗星） ───
  // 锚定在高纬天空，轴向指向深空；粒子沿轴流向远处并循环，
  // 两端淡入淡出（近端浮现、远端没入），不环绕不消失
  const streamRef = useRef<THREE.Points>(null)
  const streamGeo = useMemo(() => {
    const N = 400
    const dir = new THREE.Vector3(0, 0.3, -0.95).normalize()
    const e1 = new THREE.Vector3().crossVectors(dir, new THREE.Vector3(0, 1, 0)).normalize()
    const e2 = new THREE.Vector3().crossVectors(dir, e1).normalize()
    const origin = new THREE.Vector3(0, 12, -20)
    const len = 70

    const geo = new THREE.BufferGeometry()
    const aT = new Float32Array(N)
    const aOffset = new Float32Array(N * 3)
    const positions = new Float32Array(N * 3) // 静态基准位（包围球用），实际位置由 shader 计算
    const sizes = new Float32Array(N)
    const alphas = new Float32Array(N)

    for (let i = 0; i < N; i++) {
      aT[i] = Math.random()
      const th = Math.random() * Math.PI * 2
      const rr = Math.sqrt(Math.random()) // 椭圆截面内均匀分布
      const off = e1.clone().multiplyScalar(Math.cos(th) * rr * 2.8)
        .add(e2.clone().multiplyScalar(Math.sin(th) * rr * 1.4))
      aOffset[i * 3] = off.x
      aOffset[i * 3 + 1] = off.y
      aOffset[i * 3 + 2] = off.z
      const base = origin.clone().add(dir.clone().multiplyScalar(aT[i] * len)).add(off)
      positions[i * 3] = base.x
      positions[i * 3 + 1] = base.y
      positions[i * 3 + 2] = base.z
      sizes[i] = 0.5 + Math.random() * 0.7
      alphas[i] = 0.18 + Math.random() * 0.32
    }

    geo.setAttribute('position', new THREE.BufferAttribute(positions, 3))
    geo.setAttribute('aT', new THREE.BufferAttribute(aT, 1))
    geo.setAttribute('aOffset', new THREE.BufferAttribute(aOffset, 3))
    geo.setAttribute('size', new THREE.BufferAttribute(sizes, 1))
    geo.setAttribute('alpha', new THREE.BufferAttribute(alphas, 1))
    return geo
  }, [])

  useEffect(() => () => { streamGeo.dispose() }, [streamGeo])

  const streamMat = useMemo(() => new THREE.ShaderMaterial({
    transparent: true,
    depthWrite: false,
    blending: THREE.AdditiveBlending,
    uniforms: {
      uTime: { value: 0 },
      uPixelRatio: { value: Math.min(window.devicePixelRatio, 2) },
      uOrigin: { value: new THREE.Vector3(0, 12, -20) },
      uDir: { value: new THREE.Vector3(0, 0.3, -0.95).normalize() },
      uLen: { value: 70 },
    },
    vertexShader: /* glsl */`
      attribute float aT;
      attribute vec3 aOffset;
      attribute float size;
      attribute float alpha;
      uniform float uTime;
      uniform float uPixelRatio;
      uniform vec3 uOrigin;
      uniform vec3 uDir;
      uniform float uLen;
      varying float vAlpha;
      void main() {
        // 沿轴流动：相位循环，粒子从近端流向远端（路径加长后放慢流速）
        float t = fract(aT + uTime * 0.03);
        vec3 pos = uOrigin + uDir * (t * uLen) + aOffset;
        vec4 mvPosition = modelViewMatrix * vec4(pos, 1.0);
        float ps = size * uPixelRatio * 60.0 / -mvPosition.z;
        gl_PointSize = clamp(ps, 1.0, 8.0 * uPixelRatio);
        // 近端浮现、远端没入深处
        vAlpha = alpha * smoothstep(0.0, 0.18, t) * (1.0 - smoothstep(0.72, 1.0, t));
        gl_Position = projectionMatrix * mvPosition;
      }
    `,
    fragmentShader: /* glsl */`
      varying float vAlpha;
      void main() {
        vec2 uv = gl_PointCoord - 0.5;
        float d = length(uv);
        if (d > 0.5) discard;
        float glow = smoothstep(0.5, 0.0, d);
        gl_FragColor = vec4(vec3(0.75, 0.82, 1.0), pow(glow, 1.3) * vAlpha);
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

    // 星流：粒子沿轴流向深处
    if (streamRef.current) {
      const m = streamRef.current.material as THREE.ShaderMaterial
      m.uniforms.uTime.value = t
    }

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
      {/* 星流：椭圆柱粒子流，粒子沿轴流向深处并循环（不环绕） */}
      <points ref={streamRef} geometry={streamGeo} frustumCulled={false}>
        <primitive object={streamMat} attach="material" />
      </points>
    </group>
  )
}

function lerp(a: number, b: number, t: number) { return a + (b - a) * t }
