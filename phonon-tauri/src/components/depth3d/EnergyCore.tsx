// 中心能量核心 — 圆环结几何体 + 内部发光 + 上下体积光束
// 低频时膨胀发光，高频时表面闪烁，beat 时脉冲光爆发
import { useMemo, useRef } from 'react'
import { useFrame } from '@react-three/fiber'
import * as THREE from 'three'
import type { MutableRefObject } from 'react'
import type { SmoothedAudioData } from './audioBridge'
import { lerp } from './audioBridge'

interface Props {
  audioRef: MutableRefObject<SmoothedAudioData>
  showHalo?: boolean
}

// Fresnel 边缘发光材质（用于核心轮廓光）
function FresnelMaterial() {
  const mat = useMemo(() => new THREE.ShaderMaterial({
    uniforms: {
      uColor: { value: new THREE.Color(0.6, 0.75, 1.0) },
      uIntensity: { value: 2.0 },
      uPower: { value: 1.8 },
    },
    transparent: true,
    depthWrite: false,
    side: THREE.BackSide,
    blending: THREE.AdditiveBlending,
    vertexShader: /* glsl */`
      varying vec3 vNormal;
      varying vec3 vViewDir;
      void main() {
        vec4 mv = modelViewMatrix * vec4(position, 1.0);
        vNormal = normalize(normalMatrix * normal);
        vViewDir = normalize(-mv.xyz);
        gl_Position = projectionMatrix * mv;
      }
    `,
    fragmentShader: /* glsl */`
      uniform vec3 uColor;
      uniform float uIntensity;
      uniform float uPower;
      varying vec3 vNormal;
      varying vec3 vViewDir;
      void main() {
        float fresnel = pow(1.0 - abs(dot(vNormal, vViewDir)), uPower);
        gl_FragColor = vec4(uColor, fresnel * uIntensity);
      }
    `,
  }), [])
  return <primitive object={mat} attach="material" />
}

export function EnergyCore({ audioRef, showHalo = true }: Props) {
  const coreRef = useRef<THREE.Mesh>(null)
  const innerGlowRef = useRef<THREE.Mesh>(null)
  const accretionRef = useRef<THREE.Mesh>(null)
  const sparklesRef = useRef<THREE.Points>(null)
  const pulseRingRef = useRef<THREE.Mesh>(null)
  const ringsRef = useRef<THREE.Group>(null)

  const lowSmRef = useRef(0)
  const hiSmRef = useRef(0)
  const beatPulseRef = useRef(0)
  const lastBeatRef = useRef(false)
  const pulseAgeRef = useRef(0)

  useFrame(({ clock }) => {
    const d = audioRef.current
    const t = clock.elapsedTime

    lowSmRef.current = lerp(lowSmRef.current, d.lowFreqAvg, 0.1)
    hiSmRef.current = lerp(hiSmRef.current, d.highFreqAvg, 0.08)

    // beat 脉冲
    if (d.beat && !lastBeatRef.current) {
      beatPulseRef.current = 1
      pulseAgeRef.current = 0
    }
    beatPulseRef.current *= 0.9
    pulseAgeRef.current += 0.016
    lastBeatRef.current = d.beat

    const low = lowSmRef.current
    const hi = hiSmRef.current
    const pulse = beatPulseRef.current

    // ===== 核心晶核 =====
    if (coreRef.current) {
      const core = coreRef.current
      const breath = Math.sin(t * 1.2) * 0.06 + 1
      const scale = (1 + low * 0.3 + pulse * 0.2) * breath
      core.scale.setScalar(scale)
      core.rotation.x = t * 0.4
      core.rotation.y = t * 0.6
      core.rotation.z = t * 0.25

      const mat = core.material as THREE.MeshPhysicalMaterial
      const emitIntensity = 0.3 + low * 0.8 + pulse * 1.0
      mat.emissiveIntensity = emitIntensity

      const chroma = d.chroma12
      let maxI = 0, maxV = 0
      for (let i = 0; i < 12; i++) if (chroma[i] > maxV) { maxV = chroma[i]; maxI = i }
      const hue = (0.58 + maxI * 0.025 + low * 0.05) % 1
      mat.emissive.setHSL(hue, 0.95, 0.6)
      mat.color.setHSL(hue, 0.5, 0.25)
    }

    // ===== 环绕能量环 =====
    if (ringsRef.current) {
      const rings = ringsRef.current.children
      for (let i = 0; i < rings.length; i++) {
        const ring = rings[i] as THREE.Mesh
        const speed = 0.3 + i * 0.25
        const dir = i % 2 === 0 ? 1 : -1
        // 每个环绕不同轴旋转，形成交错感
        if (i === 0) {
          ring.rotation.x = t * speed * dir
          ring.rotation.z = t * speed * 0.3 * dir
        } else if (i === 1) {
          ring.rotation.y = t * speed * dir
          ring.rotation.x = t * speed * 0.2 * dir
        } else {
          ring.rotation.z = t * speed * dir
          ring.rotation.y = t * speed * 0.4 * dir
        }
        // 低频时环膨胀
        const baseScale = 1 + i * 0.35
        const ringScale = baseScale * (1 + low * 0.2 + pulse * 0.12)
        ring.scale.setScalar(ringScale)

        const mat = ring.material as THREE.ShaderMaterial
        if (mat && mat.uniforms) {
          mat.uniforms.uIntensity.value = 0.4 + low * 0.6 + pulse * 0.5
          mat.uniforms.uHue.value = (0.58 + i * 0.08 + low * 0.05) % 1
        }
      }
    }

    // ===== 内部光晕 =====
    if (innerGlowRef.current) {
      const glow = innerGlowRef.current
      const glowScale = 0.9 + low * 0.3 + pulse * 0.2
      glow.scale.setScalar(glowScale)
      const mat = glow.material as THREE.ShaderMaterial
      mat.uniforms.uTime.value = t
      mat.uniforms.uIntensity.value = 0.18 + low * 0.4 + pulse * 0.3
      mat.uniforms.uHue.value = (0.58 + low * 0.05) % 1
    }

    // ===== 吸积盘（水平光环） =====
    if (accretionRef.current) {
      const ring = accretionRef.current
      ring.rotation.z = t * 0.6
      const ringScale = 1 + low * 0.3 + pulse * 0.15
      ring.scale.set(ringScale, ringScale, ringScale)
      const mat = ring.material as THREE.ShaderMaterial
      mat.uniforms.uTime.value = t
      mat.uniforms.uIntensity.value = 0.7 + low * 1.0 + pulse * 0.6
      mat.uniforms.uHue.value = (0.58 + low * 0.05) % 1
      mat.uniforms.uLow.value = low
    }

    // ===== 表面闪烁粒子 =====
    if (sparklesRef.current) {
      const points = sparklesRef.current
      points.rotation.y = t * 0.5
      points.rotation.x = t * 0.3
      const mat = points.material as THREE.ShaderMaterial
      mat.uniforms.uTime.value = t
      mat.uniforms.uHi.value = hi
      mat.uniforms.uBeat.value = pulse
    }

    // ===== 脉冲光环 =====
    if (pulseRingRef.current) {
      const ring = pulseRingRef.current
      const ringScale = 1 + pulseAgeRef.current * 8
      ring.scale.set(ringScale, ringScale, ringScale)
      const ringAlpha = Math.max(0, 1 - pulseAgeRef.current * 0.4) * pulse
      const mat = ring.material as THREE.MeshBasicMaterial
      mat.opacity = ringAlpha * 0.6
    }
  })

  // 核心几何体：八面体晶核（无穿模，有能量感）
  const coreGeo = useMemo(() => {
    return new THREE.OctahedronGeometry(0.9, 1)
  }, [])

  // 能量环 shader 材质（发光圆环）
  const ringMat = useMemo(() => new THREE.ShaderMaterial({
    uniforms: {
      uHue: { value: 0.58 },
      uIntensity: { value: 0.5 },
    },
    transparent: true,
    depthWrite: false,
    blending: THREE.AdditiveBlending,
    side: THREE.DoubleSide,
    vertexShader: /* glsl */`
      varying vec2 vUv;
      varying float vAngle;
      void main() {
        vUv = uv;
        vAngle = uv.x * 6.28318;
        gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
      }
    `,
    fragmentShader: /* glsl */`
      uniform float uHue;
      uniform float uIntensity;
      varying vec2 vUv;
      varying float vAngle;

      vec3 hsv2rgb(vec3 c) {
        vec4 K = vec4(1.0, 2.0/3.0, 1.0/3.0, 3.0);
        vec3 p = abs(fract(c.xxx + K.xyz) * 6.0 - K.www);
        return c.z * mix(K.xxx, clamp(p - K.xxx, 0.0, 1.0), c.y);
      }

      void main() {
        // 环的宽度方向渐变（中间亮两边淡）
        float widthFade = smoothstep(0.0, 0.5, vUv.y) * smoothstep(1.0, 0.5, vUv.y);
        widthFade = pow(widthFade, 1.5);

        // 周向脉动（几个亮点沿环移动）
        float pulse = sin(vAngle * 3.0 + uIntensity * 10.0) * 0.5 + 0.5;
        float bright = 0.5 + pulse * 0.5;

        float alpha = widthFade * bright * uIntensity * 0.8;
        vec3 col = hsv2rgb(vec3(uHue, 0.9, 0.85));
        gl_FragColor = vec4(col, alpha);
      }
    `,
  }), [])

  // 内部光晕 shader
  const innerGlowMat = useMemo(() => new THREE.ShaderMaterial({
    uniforms: {
      uTime: { value: 0 },
      uIntensity: { value: 0.5 },
      uHue: { value: 0.58 },
    },
    transparent: true,
    depthWrite: false,
    side: THREE.BackSide,
    blending: THREE.AdditiveBlending,
    vertexShader: /* glsl */`
      varying vec3 vNormal;
      varying vec3 vPos;
      void main() {
        vNormal = normalize(normalMatrix * normal);
        vPos = position;
        gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
      }
    `,
    fragmentShader: /* glsl */`
      uniform float uTime;
      uniform float uIntensity;
      uniform float uHue;
      varying vec3 vNormal;
      varying vec3 vPos;

      vec3 hsv2rgb(vec3 c) {
        vec4 K = vec4(1.0, 2.0/3.0, 1.0/3.0, 3.0);
        vec3 p = abs(fract(c.xxx + K.xyz) * 6.0 - K.www);
        return c.z * mix(K.xxx, clamp(p - K.xxx, 0.0, 1.0), c.y);
      }

      void main() {
        float intensity = pow(0.7 - dot(vNormal, vec3(0.0, 0.0, 1.0)), 2.0);
        vec3 col = hsv2rgb(vec3(uHue, 0.9, 0.8));
        gl_FragColor = vec4(col, intensity * uIntensity);
      }
    `,
  }), [])

  // 吸积盘 shader（水平光环，带旋转纹理）
  const accretionMat = useMemo(() => new THREE.ShaderMaterial({
    uniforms: {
      uTime: { value: 0 },
      uIntensity: { value: 0.3 },
      uHue: { value: 0.58 },
      uLow: { value: 0 },
    },
    transparent: true,
    depthWrite: false,
    side: THREE.DoubleSide,
    blending: THREE.AdditiveBlending,
    vertexShader: /* glsl */`
      varying vec2 vUv;
      varying vec3 vPos;
      void main() {
        vUv = uv;
        vPos = position;
        gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
      }
    `,
    fragmentShader: /* glsl */`
      uniform float uTime;
      uniform float uIntensity;
      uniform float uHue;
      uniform float uLow;
      varying vec2 vUv;
      varying vec3 vPos;

      vec3 hsv2rgb(vec3 c) {
        vec4 K = vec4(1.0, 2.0/3.0, 1.0/3.0, 3.0);
        vec3 p = abs(fract(c.xxx + K.xyz) * 6.0 - K.www);
        return c.z * mix(K.xxx, clamp(p - K.xxx, 0.0, 1.0), c.y);
      }

      // 简单噪声
      float hash(vec2 p) {
        return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
      }
      float noise(vec2 p) {
        vec2 i = floor(p);
        vec2 f = fract(p);
        float a = hash(i);
        float b = hash(i + vec2(1.0, 0.0));
        float c = hash(i + vec2(0.0, 1.0));
        float d = hash(i + vec2(1.0, 1.0));
        vec2 u = f * f * (3.0 - 2.0 * f);
        return mix(a, b, u.x) + (c - a) * u.y * (1.0 - u.x) + (d - b) * u.x * u.y;
      }

      void main() {
        float r = length(vPos.xy);
        float angle = atan(vPos.y, vPos.x);

        // 多层环结构：内圈（窄亮）+ 中圈（宽）+ 外圈（淡宽）
        // 内圈：r=1.5~2.5，亮
        float innerRing = smoothstep(1.2, 1.8, r) * smoothstep(2.8, 2.2, r);
        float innerBright = exp(-pow((r - 2.0) / 0.35, 2.0)) * 1.2;

        // 中圈：r=2.5~5.0，主环
        float midRing = smoothstep(2.0, 3.0, r) * smoothstep(6.0, 4.8, r);
        float midBright = exp(-pow((r - 4.0) / 1.2, 2.0)) * 0.9;

        // 外圈：r=5.0~6.5，淡
        float outerRing = smoothstep(4.5, 5.5, r) * smoothstep(6.5, 6.0, r);
        float outerBright = exp(-pow((r - 5.8) / 0.6, 2.0)) * 0.5;

        float ringMask = innerRing + midRing + outerRing;
        float radialBright = innerBright + midBright + outerBright;

        // 旋转的螺旋纹理（用笛卡尔坐标避免径向接缝）
        float ang = angle + uTime * 0.3;
        vec2 spiralUv = vec2(cos(ang) * r * 1.5 + uTime * 0.1, sin(ang) * r * 1.5 - uTime * 0.15);
        float spiralNoise = noise(spiralUv) * 0.6 + noise(spiralUv * 2.3) * 0.3 + noise(spiralUv * 5.0) * 0.1;
        float detailNoise = noise(vec2(vPos.x * 4.0 + uTime * 0.2, vPos.y * 4.0 - uTime * 0.15));

        // 组合纹理 — 更有结构感
        float texture = spiralNoise * 0.7 + detailNoise * 0.3;
        texture = mix(0.3, 1.0, texture);

        // 低频时纹理更活跃、更亮
        float activity = 0.5 + uLow * 1.4;

        float alpha = ringMask * radialBright * texture * uIntensity * activity;
        vec3 col = hsv2rgb(vec3(uHue, 0.9, 0.9));
        gl_FragColor = vec4(col, alpha);
      }
    `,
  }), [])

  // 表面闪烁粒子
  const sparklesData = useMemo(() => {
    const count = 80
    const positions = new Float32Array(count * 3)
    const sizes = new Float32Array(count)
    const phases = new Float32Array(count)

    for (let i = 0; i < count; i++) {
      // 球面均匀分布
      const theta = Math.random() * Math.PI * 2
      const phi = Math.acos(2 * Math.random() - 1)
      const r = 0.85 + Math.random() * 0.3
      positions[i * 3] = r * Math.sin(phi) * Math.cos(theta)
      positions[i * 3 + 1] = r * Math.sin(phi) * Math.sin(theta)
      positions[i * 3 + 2] = r * Math.cos(phi)
      sizes[i] = 0.04 + Math.random() * 0.06
      phases[i] = Math.random() * Math.PI * 2
    }

    return { positions, sizes, phases, count }
  }, [])

  const sparkleMat = useMemo(() => new THREE.ShaderMaterial({
    uniforms: {
      uTime: { value: 0 },
      uHi: { value: 0 },
      uBeat: { value: 0 },
    },
    transparent: true,
    depthWrite: false,
    blending: THREE.AdditiveBlending,
    vertexShader: /* glsl */`
      attribute float aSize;
      attribute float aPhase;
      uniform float uTime;
      uniform float uHi;
      uniform float uBeat;
      varying float vAlpha;
      void main() {
        float twinkle = sin(uTime * 3.0 + aPhase * 6.28) * 0.5 + 0.5;
        float size = aSize * (1.0 + uHi * 3.0 + uBeat * 2.0) * (0.6 + twinkle * 0.8);
        vAlpha = 0.3 + twinkle * 0.7 + uHi * 0.5 + uBeat * 0.5;

        vec4 mv = modelViewMatrix * vec4(position, 1.0);
        gl_Position = projectionMatrix * mv;
        gl_PointSize = size * (300.0 / -mv.z);
      }
    `,
    fragmentShader: /* glsl */`
      varying float vAlpha;
      uniform float uHi;
      void main() {
        vec2 uv = gl_PointCoord - 0.5;
        float d = length(uv);
        if (d > 0.5) discard;
        float glow = smoothstep(0.5, 0.0, d);
        vec3 col = vec3(0.7, 0.85, 1.0);
        gl_FragColor = vec4(col, glow * vAlpha);
      }
    `,
  }), [])

  const sparkleGeo = useMemo(() => {
    const g = new THREE.BufferGeometry()
    g.setAttribute('position', new THREE.BufferAttribute(sparklesData.positions, 3))
    g.setAttribute('aSize', new THREE.BufferAttribute(sparklesData.sizes, 1))
    g.setAttribute('aPhase', new THREE.BufferAttribute(sparklesData.phases, 1))
    return g
  }, [sparklesData])

  return (
    <group position={[0, 0.5, 0]}>
      {/* 中心晶核（八面体） */}
      <mesh ref={coreRef} geometry={coreGeo} castShadow>
        <meshPhysicalMaterial
          color="#1a2a5a"
          emissive="#2a4aaa"
          emissiveIntensity={0.3}
          metalness={0.85}
          roughness={0.12}
          clearcoat={1}
          clearcoatRoughness={0.05}
          flatShading
        />
      </mesh>

      {/* 核心 Fresnel 轮廓光 */}
      <mesh geometry={coreGeo} scale={[1.08, 1.08, 1.08]}>
        <FresnelMaterial />
      </mesh>

      {/* 三个环绕能量环（不同角度、不同大小） */}
      <group ref={ringsRef}>
        <mesh rotation={[0, 0, 0]}>
          <torusGeometry args={[1.5, 0.03, 8, 128]} />
          <primitive object={ringMat.clone()} attach="material" />
        </mesh>
        <mesh rotation={[Math.PI / 2, 0, 0]}>
          <torusGeometry args={[1.85, 0.025, 8, 128]} />
          <primitive object={ringMat.clone()} attach="material" />
        </mesh>
        <mesh rotation={[Math.PI / 3, Math.PI / 4, 0]}>
          <torusGeometry args={[2.2, 0.02, 8, 128]} />
          <primitive object={ringMat.clone()} attach="material" />
        </mesh>
      </group>

      {/* 内部光晕 */}
      {showHalo && (
        <mesh ref={innerGlowRef}>
          <sphereGeometry args={[1.0, 32, 32]} />
          <primitive object={innerGlowMat} attach="material" />
        </mesh>
      )}

      {/* 吸积盘（水平光环） */}
      {showHalo && (
        <mesh ref={accretionRef} position={[0, -2.2, 0]} rotation={[-Math.PI / 2, 0, 0]}>
          <circleGeometry args={[6.5, 128]} />
          <primitive object={accretionMat} attach="material" />
        </mesh>
      )}

      {/* 表面闪烁粒子 */}
      {showHalo && <points ref={sparklesRef} geometry={sparkleGeo} material={sparkleMat} />}

      {/* beat 脉冲光环（水平扩散） */}
      {showHalo && (
        <mesh ref={pulseRingRef} rotation={[-Math.PI / 2, 0, 0]}>
          <ringGeometry args={[0.9, 1.0, 64]} />
          <meshBasicMaterial
            color="#8ab4ff"
            transparent
            opacity={0}
            side={THREE.DoubleSide}
          blending={THREE.AdditiveBlending}
          depthWrite={false}
        />
        </mesh>
      )}

      {/* 漂浮光晕（柔和光斑，增加空间层次） */}
      {showHalo && (
        <>
          <FloatingGlow position={[5, 2, -8]} size={4} hue={0.58} speed={0.3} />
          <FloatingGlow position={[-7, -1, -5]} size={5.5} hue={0.72} speed={0.22} />
          <FloatingGlow position={[3, -2, -12]} size={7} hue={0.08} speed={0.18} />
          <FloatingGlow position={[-5, 3, -10]} size={4.5} hue={0.65} speed={0.28} />
          <FloatingGlow position={[8, 0, -3]} size={3} hue={0.55} speed={0.35} />
          <FloatingGlow position={[-9, -3, -8]} size={6} hue={0.78} speed={0.2} />
          <FloatingGlow position={[0, 4, -14]} size={5} hue={0.05} speed={0.15} />
          <FloatingGlow position={[-2, -4, -6]} size={3.5} hue={0.68} speed={0.32} />
        </>
      )}
    </group>
  )
}

// 漂浮光晕组件 — 柔和的光斑，缓慢漂移，始终面向相机
function FloatingGlow({ position, size, hue, speed }: { position: [number, number, number]; size: number; hue: number; speed: number }) {
  const meshRef = useRef<THREE.Mesh>(null)
  const phase = useMemo(() => Math.random() * Math.PI * 2, [])

  const mat = useMemo(() => new THREE.ShaderMaterial({
    transparent: true,
    depthWrite: false,
    blending: THREE.AdditiveBlending,
    side: THREE.DoubleSide,
    uniforms: { uHue: { value: hue }, uIntensity: { value: 0.12 } },
    vertexShader: /* glsl */`
      varying vec2 vUv;
      void main() {
        vUv = uv;
        gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
      }
    `,
    fragmentShader: /* glsl */`
      varying vec2 vUv;
      uniform float uHue;
      uniform float uIntensity;

      vec3 hsv2rgb(vec3 c) {
        vec4 K = vec4(1.0, 2.0/3.0, 1.0/3.0, 3.0);
        vec3 p = abs(fract(c.xxx + K.xyz) * 6.0 - K.www);
        return c.z * mix(K.xxx, clamp(p - K.xxx, 0.0, 1.0), c.y);
      }

      void main() {
        vec2 uv = vUv - 0.5;
        float d = length(uv);
        // 柔和的径向渐变，中心亮边缘淡
        float glow = exp(-d * 3.5) * 0.8 + exp(-d * 1.5) * 0.2;
        glow = smoothstep(0.5, 0.0, d) * glow;
        vec3 col = hsv2rgb(vec3(uHue, 0.8, 0.9));
        gl_FragColor = vec4(col, glow * uIntensity);
      }
    `,
  }), [hue])

  useFrame(({ clock, camera }) => {
    if (!meshRef.current) return
    const t = clock.getElapsedTime()
    // 漂浮：幅度增大，明显可见的运动
    meshRef.current.position.y = position[1] + Math.sin(t * speed + phase) * 2.5
    meshRef.current.position.z = position[2] + Math.cos(t * speed * 0.7 + phase) * 1.8
    meshRef.current.position.x = position[0] + Math.sin(t * speed * 0.5 + phase * 1.5) * 1.5
    // 始终面向相机
    meshRef.current.lookAt(camera.position)
    // 呼吸式缩放（大小变化）
    const breathScale = 1 + Math.sin(t * speed * 2 + phase) * 0.3
    meshRef.current.scale.setScalar(size * breathScale)
  })

  return (
    <mesh ref={meshRef} position={position}>
      <planeGeometry args={[1, 1]} />
      <primitive object={mat} attach="material" />
    </mesh>
  )
}
