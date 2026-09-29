import { useRef, useMemo, useEffect, useState } from 'react'
import { useFrame } from '@react-three/fiber'
import { Html } from '@react-three/drei'
import type { MutableRefObject } from 'react'
import * as THREE from 'three'
import type { SmoothedAudioData } from './audioBridge'

interface Props {
  audioRef: MutableRefObject<SmoothedAudioData>
}

const LANE_COUNT = 4  // 4 轨道对应 D/F/J/K 四个按键
const LANE_KEYS = ['KeyD', 'KeyF', 'KeyJ', 'KeyK']
const LANE_LABELS = ['D', 'F', 'J', 'K']

// 透视参数：近处宽、远处窄
const NEAR_WIDTH = 9      // 近处轨道总宽度
const FAR_WIDTH = 2.2     // 远处轨道总宽度
const TRACK_DEPTH = 22    // 轨道纵深
const HIT_LINE_Z = 2      // 判定线 Z 位置（靠近相机）
const FAR_LINE_Z = -20    // 远处汇聚点 Z 位置
const NOTE_TRAVEL_TIME = 2.0

interface Note {
  lane: number
  spawnTime: number
  hitTime: number
  hit: boolean
  missed: boolean
}

interface HitEffect {
  lane: number
  time: number
  type: 'perfect' | 'good' | 'miss'
}

// 计算某 z 位置处的轨道宽度（透视缩放）
function widthAtZ(z: number) {
  const t = (HIT_LINE_Z - z) / (HIT_LINE_Z - FAR_LINE_Z) // 0=近, 1=远
  return NEAR_WIDTH * (1 - t) + FAR_WIDTH * t
}

// 计算某轨道在某 z 位置的 x 坐标
function laneX(lane: number, z: number) {
  const w = widthAtZ(z)
  const step = w / (LANE_COUNT - 1)
  return -w / 2 + lane * step
}

// 日系风格音游：梯形透视轨道 + 圆形音符 + 光环击中效果
export function RhythmVisualizer({ audioRef }: Props) {
  const groupRef = useRef<THREE.Group>(null)
  const hitLineRef = useRef<THREE.Mesh>(null)
  const notesRef = useRef<Note[]>([])
  const instancedRef = useRef<THREE.InstancedMesh>(null)
  const burstRef = useRef<THREE.Points>(null)

  const [hitEffects, setHitEffects] = useState<HitEffect[]>([])
  const [score, setScore] = useState({ perfect: 0, good: 0, miss: 0, combo: 0 })
  const [keyStates, setKeyStates] = useState<boolean[]>([false, false, false, false])
  const keyStatesRef = useRef<boolean[]>([false, false, false, false])
  const laneFlashRef = useRef<number[]>([0, 0, 0, 0])
  const hitRingRefs = useRef<(THREE.Mesh | null)[]>([])

  const lowSmRef = useRef(0)
  const beatPulseRef = useRef(0)
  const lastBeatRef = useRef(false)
  const lastOnsetRef = useRef(0)

  const laneColors = useMemo(() => {
    // 参考图风格：粉紫→蓝→青→绿 渐变
    const hues = [0.92, 0.08, 0.52, 0.75]
    return hues.map(h => new THREE.Color().setHSL(h, 0.85, 0.7))
  }, [])

  // ─── 判定线 ───
  const hitLineMat = useMemo(() => new THREE.ShaderMaterial({
    transparent: true,
    depthWrite: false,
    blending: THREE.AdditiveBlending,
    uniforms: {
      uPulse: { value: 0 },
      uTime: { value: 0 },
    },
    vertexShader: /* glsl */`
      varying vec2 vUv;
      void main() {
        vUv = uv;
        gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
      }
    `,
    fragmentShader: /* glsl */`
      uniform float uPulse;
      uniform float uTime;
      varying vec2 vUv;
      void main() {
        float center = abs(vUv.y - 0.5);
        // 核心亮线
        float core = smoothstep(0.06, 0.0, center);
        // 外辉光
        float glow = smoothstep(0.5, 0.0, center) * 0.35;
        // 闪烁
        float flicker = sin(uTime * 6.0) * 0.08 + 0.92;

        vec3 col = mix(vec3(0.5, 0.85, 1.0), vec3(1.0, 0.9, 0.5), uPulse);
        float alpha = (core * 1.0 + glow) * (0.8 + uPulse * 0.4) * flicker;
        gl_FragColor = vec4(col, alpha);
      }
    `,
  }), [])

  // ─── 音符（圆形，带发光边缘） ───
  const MAX_NOTES = 100
  const noteGeo = useMemo(() => new THREE.CircleGeometry(0.5, 32), [])
  const noteMat = useMemo(() => new THREE.ShaderMaterial({
    transparent: true,
    depthWrite: false,
    blending: THREE.AdditiveBlending,
    side: THREE.DoubleSide,
    uniforms: { uColor: { value: new THREE.Color(0xffffff) } },
    vertexShader: /* glsl */`
      varying vec2 vUv;
      void main() {
        vUv = uv;
        gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
      }
    `,
    fragmentShader: /* glsl */`
      uniform vec3 uColor;
      varying vec2 vUv;
      void main() {
        vec2 uv = vUv - 0.5;
        float d = length(uv);
        if (d > 0.5) discard;

        // 核心
        float core = smoothstep(0.5, 0.35, d);
        // 边缘辉光
        float rim = smoothstep(0.5, 0.2, d) * 0.4;
        // 内部渐变
        float inner = 1.0 - d * 1.2;
        inner = max(0.3, inner);

        vec3 col = uColor * (core * 1.2 + rim + inner * 0.5);
        float alpha = core * 0.95 + rim * 0.6;

        gl_FragColor = vec4(col, alpha);
      }
    `,
  }), [])

  // ─── 击中光环 ───
  const hitRingMat = useMemo(() => new THREE.ShaderMaterial({
    transparent: true,
    depthWrite: false,
    blending: THREE.AdditiveBlending,
    side: THREE.DoubleSide,
    uniforms: {
      uProgress: { value: 0 },
      uColor: { value: new THREE.Color(0xffffff) },
    },
    vertexShader: /* glsl */`
      varying vec2 vUv;
      void main() {
        vUv = uv;
        gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
      }
    `,
    fragmentShader: /* glsl */`
      uniform float uProgress;
      uniform vec3 uColor;
      varying vec2 vUv;
      void main() {
        vec2 uv = vUv - 0.5;
        float d = length(uv);
        // 环形：中心在 progress 位置
        float ringCenter = uProgress * 0.4 + 0.1;
        float ringWidth = 0.08 + uProgress * 0.05;
        float ring = smoothstep(ringWidth, 0.0, abs(d - ringCenter));

        float alpha = ring * (1.0 - uProgress) * 1.2;
        gl_FragColor = vec4(uColor, alpha);
      }
    `,
  }), [])

  // ─── 轨道线 ───
  const laneLineMat = useMemo(() => new THREE.ShaderMaterial({
    transparent: true,
    depthWrite: false,
    blending: THREE.AdditiveBlending,
    uniforms: {
      uTime: { value: 0 },
      uFlash: { value: 0 },
    },
    vertexShader: /* glsl */`
      varying float vDepth;
      varying vec3 vPos;
      void main() {
        vPos = position;
        vDepth = (2.0 - position.z) / 22.0; // 0=近, 1=远
        gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
      }
    `,
    fragmentShader: /* glsl */`
      uniform float uTime;
      uniform float uFlash;
      varying float vDepth;
      varying vec3 vPos;
      void main() {
        float fade = 1.0 - vDepth * 0.6;
        fade = max(0.0, fade);
        // 流动光点效果
        float flow = fract(-vDepth * 5.0 - uTime * 0.5);
        flow = smoothstep(0.0, 0.15, flow) * smoothstep(1.0, 0.85, flow);
        vec3 col = mix(vec3(0.5, 0.75, 1.0), vec3(0.8, 0.6, 1.0), vDepth * 0.5);
        float alpha = fade * (0.35 + uFlash * 0.5 + flow * 0.2);
        gl_FragColor = vec4(col, alpha);
      }
    `,
  }), [])

  // ─── 按键指示器（判定线上的圆形） ───
  const keyIndicatorMat = useMemo(() => {
    return laneColors.map(col => new THREE.ShaderMaterial({
      transparent: true,
      depthWrite: false,
      blending: THREE.AdditiveBlending,
      side: THREE.DoubleSide,
      uniforms: {
        uActive: { value: 0 },
        uColor: { value: col.clone() },
      },
      vertexShader: /* glsl */`
        varying vec2 vUv;
        void main() {
          vUv = uv;
          gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
        }
      `,
      fragmentShader: /* glsl */`
        uniform float uActive;
        uniform vec3 uColor;
        varying vec2 vUv;
        void main() {
          vec2 uv = vUv - 0.5;
          float d = length(uv);
          if (d > 0.5) discard;

          // 外圈
          float outer = smoothstep(0.5, 0.4, d) * 0.8;
          // 内圈
          float inner = smoothstep(0.35, 0.25, d) * 0.5;
          // 中心填充（按下时）
          float fill = smoothstep(0.25, 0.0, d) * uActive * 0.6;
          // 星形高光
          float star = 0.0;

          vec3 col = uColor * (outer + inner + fill + star);
          float alpha = outer * 0.8 + inner * 0.5 + fill + uActive * 0.3;

          gl_FragColor = vec4(col, alpha);
        }
      `,
    }))
  }, [laneColors])

  // ─── 爆裂粒子（星星形状） ───
  const MAX_BURSTS = 400
  const burstGeo = useMemo(() => {
    const geo = new THREE.BufferGeometry()
    geo.setAttribute('position', new THREE.BufferAttribute(new Float32Array(MAX_BURSTS * 3), 3))
    geo.setAttribute('lifetime', new THREE.BufferAttribute(new Float32Array(MAX_BURSTS), 1))
    geo.setAttribute('color', new THREE.BufferAttribute(new Float32Array(MAX_BURSTS * 3), 3))
    geo.setAttribute('size', new THREE.BufferAttribute(new Float32Array(MAX_BURSTS), 1))
    return geo
  }, [])

  const burstMat = useMemo(() => new THREE.ShaderMaterial({
    transparent: true,
    depthWrite: false,
    blending: THREE.AdditiveBlending,
    uniforms: { uPixelRatio: { value: Math.min(window.devicePixelRatio, 2) } },
    vertexShader: /* glsl */`
      attribute float size;
      attribute float lifetime;
      attribute vec3 color;
      uniform float uPixelRatio;
      varying float vLife;
      varying vec3 vColor;
      void main() {
        vColor = color;
        vLife = lifetime;
        vec4 mvPosition = modelViewMatrix * vec4(position, 1.0);
        gl_PointSize = size * uPixelRatio * 55.0 / -mvPosition.z * lifetime;
        gl_Position = projectionMatrix * mvPosition;
      }
    `,
    fragmentShader: /* glsl */`
      varying float vLife;
      varying vec3 vColor;
      void main() {
        if (vLife <= 0.0) discard;
        vec2 uv = gl_PointCoord - 0.5;
        float d = length(uv);
        if (d > 0.5) discard;
        // 星形
        float angle = atan(uv.y, uv.x);
        float r = d * 2.0;
        float starShape = cos(angle * 4.0) * 0.15 + 0.85;
        float star = smoothstep(starShape, starShape - 0.1, r) * (1.0 - r);
        float alpha = (star + smoothstep(0.5, 0.0, d) * 0.3) * vLife;
        gl_FragColor = vec4(vColor, alpha);
      }
    `,
  }), [])

  const burstState = useMemo(() => ({
    pos: new Float32Array(MAX_BURSTS * 3),
    vel: new Float32Array(MAX_BURSTS * 3),
    life: new Float32Array(MAX_BURSTS),
    col: new Float32Array(MAX_BURSTS * 3),
    size: new Float32Array(MAX_BURSTS),
    nextIdx: 0,
  }), [])

  // 轨道线几何体（用梯形）
  const laneLinesGeo = useMemo(() => {
    const geos: THREE.BufferGeometry[] = []
    for (let i = 0; i < LANE_COUNT; i++) {
      const nearX = laneX(i, HIT_LINE_Z)
      const farX = laneX(i, FAR_LINE_Z)
      const shape = new THREE.Shape()
      const lineWidth = 0.025
      shape.moveTo(nearX - lineWidth, HIT_LINE_Z)
      shape.lineTo(nearX + lineWidth, HIT_LINE_Z)
      shape.lineTo(farX + lineWidth * 0.3, FAR_LINE_Z)
      shape.lineTo(farX - lineWidth * 0.3, FAR_LINE_Z)
      shape.closePath()
      const geo = new THREE.ShapeGeometry(shape)
      geos.push(geo)
    }
    return geos
  }, [])

  // ─── 键盘事件 ───
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      const lane = LANE_KEYS.indexOf(e.code)
      if (lane < 0) return
      if (keyStatesRef.current[lane]) return
      keyStatesRef.current[lane] = true
      setKeyStates(prev => { const n = [...prev]; n[lane] = true; return n })
      laneFlashRef.current[lane] = 1
      tryHit(lane, performance.now() / 1000)
    }
    const onKeyUp = (e: KeyboardEvent) => {
      const lane = LANE_KEYS.indexOf(e.code)
      if (lane < 0) return
      keyStatesRef.current[lane] = false
      setKeyStates(prev => { const n = [...prev]; n[lane] = false; return n })
    }
    window.addEventListener('keydown', onKeyDown)
    window.addEventListener('keyup', onKeyUp)
    return () => {
      window.removeEventListener('keydown', onKeyDown)
      window.removeEventListener('keyup', onKeyUp)
    }
  }, [])

  function tryHit(lane: number, t: number) {
    const notes = notesRef.current
    let bestNote: Note | null = null
    let bestDist = Infinity
    for (const n of notes) {
      if (n.lane !== lane || n.hit || n.missed) continue
      const dist = Math.abs(n.hitTime - t)
      if (dist < bestDist) {
        bestDist = dist
        bestNote = n
      }
    }
    if (bestNote && bestDist < 0.3) {
      bestNote.hit = true
      const type = bestDist < 0.12 ? 'perfect' : 'good'
      triggerHitEffect(lane, laneColors[lane], type)
      setHitEffects(prev => [...prev, { lane, time: t, type }])
      setScore(prev => ({
        perfect: prev.perfect + (type === 'perfect' ? 1 : 0),
        good: prev.good + (type === 'good' ? 1 : 0),
        miss: prev.miss,
        combo: prev.combo + 1,
      }))
    }
  }

  function spawnNote(t: number, d: SmoothedAudioData) {
    const spectrum = d.spectrum64
    let maxBin = 0, maxVal = 0
    for (let i = 0; i < spectrum.length; i++) {
      if (spectrum[i] > maxVal) { maxVal = spectrum[i]; maxBin = i }
    }
    // 按频率分配轨道：低频左，高频右
    const laneRaw = maxBin / spectrum.length * LANE_COUNT
    const lane = Math.min(LANE_COUNT - 1, Math.max(0, Math.floor(laneRaw)))

    // 30% 概率随机偏移避免单调
    const finalLane = Math.random() < 0.25
      ? Math.min(LANE_COUNT - 1, Math.max(0, lane + (Math.random() < 0.5 ? -1 : 1)))
      : lane

    notesRef.current.push({
      lane: finalLane,
      spawnTime: t,
      hitTime: t + NOTE_TRAVEL_TIME,
      hit: false,
      missed: false,
    })
    if (notesRef.current.length > MAX_NOTES) notesRef.current.shift()
  }

  function triggerHitEffect(lane: number, color: THREE.Color, type: string) {
    const x = laneX(lane, HIT_LINE_Z)
    const particleCount = type === 'perfect' ? 20 : 12

    // 星星粒子爆发
    for (let i = 0; i < particleCount; i++) {
      const idx = burstState.nextIdx
      burstState.nextIdx = (burstState.nextIdx + 1) % MAX_BURSTS
      burstState.pos[idx * 3] = x
      burstState.pos[idx * 3 + 1] = 0.15
      burstState.pos[idx * 3 + 2] = HIT_LINE_Z
      const angle = (i / particleCount) * Math.PI * 2 + Math.random() * 0.4
      const speed = 2 + Math.random() * 3.5
      burstState.vel[idx * 3] = Math.cos(angle) * speed
      burstState.vel[idx * 3 + 1] = Math.random() * 3 + 1.5
      burstState.vel[idx * 3 + 2] = Math.sin(angle) * speed * 0.3
      burstState.life[idx] = 1.0
      burstState.col[idx * 3] = color.r
      burstState.col[idx * 3 + 1] = color.g
      burstState.col[idx * 3 + 2] = color.b
      burstState.size[idx] = 0.2 + Math.random() * 0.35
    }

    // 光环扩散（用 laneFlash 触发，简化处理）
    laneFlashRef.current[lane] = 1.5
  }

  useFrame(({ clock }) => {
    const d = audioRef.current
    const t = clock.elapsedTime

    lowSmRef.current = lerp(lowSmRef.current, d.lowFreqAvg, 0.08)

    // beat → 生成音符
    if (d.beat && !lastBeatRef.current) {
      beatPulseRef.current = 1
      spawnNote(t, d)
    }
    beatPulseRef.current *= 0.86
    lastBeatRef.current = d.beat

    // onset 也生成
    if (d.onset > 0.5 && d.onset - lastOnsetRef.current > 0.17) {
      spawnNote(t, d)
      lastOnsetRef.current = d.onset
    }

    const pulse = beatPulseRef.current
    const low = lowSmRef.current

    // 判定线
    if (hitLineRef.current) {
      const mat = hitLineRef.current.material as THREE.ShaderMaterial
      mat.uniforms.uPulse.value = pulse
      mat.uniforms.uTime.value = t
      hitLineRef.current.scale.setScalar(1 + pulse * 0.04)
    }

    // 轨道线
    for (let i = 0; i < LANE_COUNT; i++) {
      laneFlashRef.current[i] *= 0.88
    }

    // 按键指示器
    for (let i = 0; i < LANE_COUNT; i++) {
      const mat = keyIndicatorMat[i] as THREE.ShaderMaterial
      // 结合按键状态和击中闪光
      const active = Math.max(keyStates[i] ? 1 : 0, laneFlashRef.current[i] * 0.8)
      mat.uniforms.uActive.value = active
    }

    // 更新音符 — 检查 miss
    const notes = notesRef.current
    for (let i = notes.length - 1; i >= 0; i--) {
      const note = notes[i]
      const age = t - note.spawnTime
      const progress = age / NOTE_TRAVEL_TIME
      if (progress >= 1.1 && !note.hit && !note.missed) {
        note.missed = true
        setScore(prev => ({ ...prev, miss: prev.miss + 1, combo: 0 }))
        setHitEffects(prev => [...prev, { lane: note.lane, time: t, type: 'miss' }])
      }
      if (progress > 1.25) notes.splice(i, 1)
    }

    // 更新 InstancedMesh 音符（圆形，透视缩放）
    if (instancedRef.current) {
      const inst = instancedRef.current
      const dummy = new THREE.Object3D()
      const active = notes.filter(n => {
        const progress = (t - n.spawnTime) / NOTE_TRAVEL_TIME
        return progress < 1.1 && !n.missed
      })
      for (let i = 0; i < MAX_NOTES; i++) {
        if (i < active.length) {
          const note = active[i]
          const progress = (t - note.spawnTime) / NOTE_TRAVEL_TIME
          const z = FAR_LINE_Z + progress * (HIT_LINE_Z - FAR_LINE_Z)
          const x = laneX(note.lane, z)
          // 音符大小随透视变化（近处大远处小）
          const sizeFactor = 1.0 - progress * 0.65
          const noteSize = 0.65 * sizeFactor
          dummy.position.set(x, 0.1, z)
          const scale = note.hit
            ? Math.max(0.001, 1.8 - (progress - 1) * 8) * noteSize
            : noteSize
          dummy.scale.set(scale, scale, 1)
          dummy.rotation.x = -Math.PI / 2
          dummy.updateMatrix()
          inst.setMatrixAt(i, dummy.matrix)
          inst.setColorAt(i, note.hit
            ? new THREE.Color(0.4, 0.4, 0.4)
            : laneColors[note.lane]
          )
        } else {
          dummy.position.set(0, 999, 0)
          dummy.scale.setScalar(0.001)
          dummy.updateMatrix()
          inst.setMatrixAt(i, dummy.matrix)
        }
      }
      inst.instanceMatrix.needsUpdate = true
      if (inst.instanceColor) inst.instanceColor.needsUpdate = true
    }

    // 更新爆裂粒子
    const dt = 0.016
    const posAttr = burstGeo.attributes.position as THREE.BufferAttribute
    const lifeAttr = burstGeo.attributes.lifetime as THREE.BufferAttribute
    for (let i = 0; i < MAX_BURSTS; i++) {
      if (burstState.life[i] <= 0) continue
      burstState.pos[i * 3] += burstState.vel[i * 3] * dt
      burstState.pos[i * 3 + 1] += burstState.vel[i * 3 + 1] * dt
      burstState.pos[i * 3 + 2] += burstState.vel[i * 3 + 2] * dt
      burstState.vel[i * 3 + 1] -= 4.5 * dt // 重力
      burstState.life[i] -= dt * 1.4
      posAttr.array[i * 3] = burstState.pos[i * 3]
      posAttr.array[i * 3 + 1] = burstState.pos[i * 3 + 1]
      posAttr.array[i * 3 + 2] = burstState.pos[i * 3 + 2]
      lifeAttr.array[i] = Math.max(0, burstState.life[i])
    }
    posAttr.needsUpdate = true
    lifeAttr.needsUpdate = true
  })

  return (
    <>
      <group ref={groupRef} position={[0, 0, 0]}>
        {/* 轨道线（梯形透视） */}
        {laneLinesGeo.map((geo, i) => (
          <mesh key={`lane-${i}`} geometry={geo} material={laneLineMat} position={[0, 0.02, 0]} rotation={[Math.PI / 2, 0, 0]} />
        ))}

        {/* 判定线 */}
        <mesh ref={hitLineRef} position={[0, 0.05, HIT_LINE_Z]} rotation={[-Math.PI / 2, 0, 0]}>
          <planeGeometry args={[NEAR_WIDTH + 1.5, 0.6]} />
          <primitive object={hitLineMat} attach="material" />
        </mesh>

        {/* 按键指示器（判定线上的圆形） */}
        {Array.from({ length: LANE_COUNT }).map((_, i) => {
          const x = laneX(i, HIT_LINE_Z)
          return (
            <mesh key={`key-${i}`} position={[x, 0.08, HIT_LINE_Z + 0.01]} rotation={[-Math.PI / 2, 0, 0]}>
              <circleGeometry args={[0.45, 32]} />
              <primitive object={keyIndicatorMat[i]} attach="material" />
            </mesh>
          )
        })}

        {/* 音符（InstancedMesh 圆形） */}
        <instancedMesh
          ref={instancedRef}
          args={[noteGeo, noteMat, MAX_NOTES]}
        />

        {/* 爆裂粒子（星星） */}
        <points ref={burstRef} geometry={burstGeo}>
          <primitive object={burstMat} attach="material" />
        </points>
      </group>

      {/* HUD */}
      <RhythmHUD score={score} hitEffects={hitEffects} />
    </>
  )
}

// ─── HUD：Combo + 判定文字 ───
function RhythmHUD({
  score,
  hitEffects,
}: {
  score: { perfect: number; good: number; miss: number; combo: number }
  hitEffects: HitEffect[]
}) {
  const recent = hitEffects.slice(-3).reverse()
  const latest = recent[0]

  return (
    <group>
      {/* 右侧 Combo */}
      <Html position={[4.5, 2.5, HIT_LINE_Z - 1]} style={{ pointerEvents: 'none', textAlign: 'right' }}>
        {score.combo > 1 && (
          <div style={{
            display: 'flex',
            flexDirection: 'column',
            alignItems: 'flex-end',
            pointerEvents: 'none',
            userSelect: 'none',
          }}>
            <div style={{
              fontSize: 56,
              fontWeight: 900,
              color: '#fff',
              textShadow: '0 0 16px rgba(120, 180, 255, 0.9), 0 0 32px rgba(120, 180, 255, 0.5), 0 2px 4px rgba(0,0,0,0.5)',
              fontVariantNumeric: 'tabular-nums',
              lineHeight: 1,
              letterSpacing: 1,
            }}>
              {score.combo}
            </div>
            <div style={{
              fontSize: 20,
              fontWeight: 800,
              color: 'rgba(220, 230, 255, 0.95)',
              textShadow: '0 0 8px rgba(120, 180, 255, 0.6), 0 2px 4px rgba(0,0,0,0.5)',
              letterSpacing: 4,
              marginTop: 2,
            }}>
              COMBO
            </div>
          </div>
        )}
      </Html>

      {/* 中央判定文字 */}
      <Html position={[0, 1.2, HIT_LINE_Z - 0.5]} center style={{ pointerEvents: 'none' }}>
        <div style={{
          display: 'flex',
          flexDirection: 'column',
          alignItems: 'center',
          gap: 4,
          pointerEvents: 'none',
          userSelect: 'none',
          minWidth: 200,
        }}>
          {latest && (
            <div style={{
              fontSize: 36,
              fontWeight: 900,
              letterSpacing: 4,
              color: latest.type === 'perfect'
                ? '#fff'
                : latest.type === 'good'
                ? '#ffdd77'
                : '#ff6b6b',
              textShadow: latest.type === 'perfect'
                ? '0 0 20px rgba(120, 255, 180, 0.8), 0 0 40px rgba(180, 120, 255, 0.5), 0 2px 8px rgba(0,0,0,0.6)'
                : latest.type === 'good'
                ? '0 0 15px rgba(255, 220, 120, 0.7), 0 2px 6px rgba(0,0,0,0.5)'
                : '0 0 12px rgba(255, 100, 100, 0.7), 0 2px 6px rgba(0,0,0,0.5)',
              animation: 'judgePop 0.3s ease-out',
              background: latest.type === 'perfect'
                ? 'linear-gradient(90deg, #ff6b9d, #c44dff, #6b9dff, #6bffb5, #ffdd6b, #ff6b9d)'
                : 'none',
              WebkitBackgroundClip: latest.type === 'perfect' ? 'text' : 'border-box',
              backgroundClip: latest.type === 'perfect' ? 'text' : 'border-box',
              WebkitTextFillColor: latest.type === 'perfect' ? 'transparent' : '#fff',
            }}>
              {latest.type === 'perfect' ? 'PERFECT' : latest.type === 'good' ? 'GREAT' : 'MISS'}
            </div>
          )}
        </div>
      </Html>

      {/* 左上角分数 */}
      <Html position={[-5, 3.5, HIT_LINE_Z - 1]} style={{ pointerEvents: 'none' }}>
        <div style={{
          display: 'flex',
          flexDirection: 'column',
          gap: 4,
          pointerEvents: 'none',
          userSelect: 'none',
        }}>
          <div style={{
            fontSize: 11,
            fontWeight: 600,
            color: 'rgba(180, 200, 255, 0.6)',
            letterSpacing: 2,
          }}>
            SCORE
          </div>
          <div style={{
            fontSize: 22,
            fontWeight: 800,
            color: 'rgba(220, 230, 255, 0.95)',
            textShadow: '0 0 8px rgba(120, 180, 255, 0.5), 0 2px 4px rgba(0,0,0,0.5)',
            fontVariantNumeric: 'tabular-nums',
            letterSpacing: 1,
          }}>
            {String((score.perfect * 100 + score.good * 50)).padStart(7, '0')}
          </div>
          <div style={{
            display: 'flex',
            gap: 12,
            fontSize: 11,
            fontWeight: 600,
            fontVariantNumeric: 'tabular-nums',
            marginTop: 4,
          }}>
            <span style={{ color: 'rgba(120, 255, 180, 0.8)' }}>P {score.perfect}</span>
            <span style={{ color: 'rgba(255, 220, 120, 0.8)' }}>G {score.good}</span>
            <span style={{ color: 'rgba(255, 120, 120, 0.8)' }}>M {score.miss}</span>
          </div>
        </div>
      </Html>

      <style>{`
        @keyframes judgePop {
          0% { transform: scale(0.7); opacity: 0; }
          50% { transform: scale(1.15); }
          100% { transform: scale(1); opacity: 1; }
        }
      `}</style>
    </group>
  )
}

function lerp(a: number, b: number, t: number) { return a + (b - a) * t }
