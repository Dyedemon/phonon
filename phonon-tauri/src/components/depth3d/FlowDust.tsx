// 流场星尘 — 沿涡旋流场运动的光点粒子
// 有秩序、有轨迹感，不是散乱的。onset 时从中心迸发闪光粒子
import { useMemo, useRef, useEffect } from 'react'
import { useFrame, useThree } from '@react-three/fiber'
import * as THREE from 'three'
import type { MutableRefObject } from 'react'
import type { SmoothedAudioData } from './audioBridge'
import { lerp } from './audioBridge'

interface Props {
  audioRef: MutableRefObject<SmoothedAudioData>
  nebulaMode?: boolean
}

const PARTICLE_COUNT = 700
const BURST_COUNT = 40

// 2D 简化噪声（用于流场）
function noise2D(x: number, y: number): number {
  const n = Math.sin(x * 12.9898 + y * 78.233) * 43758.5453
  return n - Math.floor(n)
}

export function FlowDust({ audioRef, nebulaMode = false }: Props) {
  const pointsRef = useRef<THREE.Points>(null)
  const burstPointsRef = useRef<THREE.Points>(null)
  const { size } = useThree()

  const velocitiesRef = useRef<Float32Array>(new Float32Array(PARTICLE_COUNT * 3))
  const burstVelocitiesRef = useRef<Float32Array>(new Float32Array(BURST_COUNT * 3))
  const burstLifesRef = useRef<Float32Array>(new Float32Array(BURST_COUNT))
  const burstActiveRef = useRef(0)

  const lowSmRef = useRef(0)
  const hiSmRef = useRef(0)

  // 星云模式切换
  useEffect(() => {
    if (pointsRef.current) {
      const mat = pointsRef.current.material as THREE.ShaderMaterial
      mat.uniforms.uNebula.value = nebulaMode ? 1.0 : 0.0
    }
  }, [nebulaMode])

  useFrame(({ clock }) => {
    const d = audioRef.current
    const t = clock.elapsedTime
    const low = lowSmRef.current = lerp(lowSmRef.current, d.lowFreqAvg, 0.06)
    const hi = hiSmRef.current = lerp(hiSmRef.current, d.highFreqAvg, 0.08)

    // ===== 主粒子群（流场运动）=====
    if (pointsRef.current) {
      const geo = pointsRef.current.geometry
      const posAttr = geo.attributes.position as THREE.BufferAttribute
      const pos = posAttr.array as Float32Array
      const vel = velocitiesRef.current

      const flowStrength = 0.008 + low * 0.015

      for (let i = 0; i < PARTICLE_COUNT; i++) {
        const i3 = i * 3
        const x = pos[i3]
        const y = pos[i3 + 1]
        const z = pos[i3 + 2]

        // 简化的 3D 涡旋流场：
        // 1. 绕 Y 轴旋转
        const angle = Math.atan2(z, x)

        // 2. 用伪噪声制造流动扰动
        const nx = noise2D(x * 0.1 + t * 0.05, y * 0.1)
        const ny = noise2D(y * 0.1 + t * 0.03, z * 0.1)
        const nz = noise2D(z * 0.1 + t * 0.04, x * 0.1)

        // 切向速度（绕中心旋转）
        const tangentSpeed = 0.02 + low * 0.03
        vel[i3] += (-Math.sin(angle) * tangentSpeed + (nx - 0.5) * flowStrength) * 0.1
        vel[i3 + 1] += ((ny - 0.5) * flowStrength + 0.005 * low) * 0.1
        vel[i3 + 2] += (Math.cos(angle) * tangentSpeed + (nz - 0.5) * flowStrength) * 0.1

        // 阻尼
        vel[i3] *= 0.96
        vel[i3 + 1] *= 0.96
        vel[i3 + 2] *= 0.96

        // 更新位置
        pos[i3] += vel[i3]
        pos[i3 + 1] += vel[i3 + 1]
        pos[i3 + 2] += vel[i3 + 2]

        // 边界处理：粒子飘太远就重新拉回中心附近
        const dist = Math.sqrt(x * x + y * y + z * z)
        if (dist > 25) {
          const scale = 8 / dist
          pos[i3] *= scale
          pos[i3 + 1] *= scale
          pos[i3 + 2] *= scale
          vel[i3] *= 0.1
          vel[i3 + 1] *= 0.1
          vel[i3 + 2] *= 0.1
        }
      }

      posAttr.needsUpdate = true

      // 同步速度到 attribute（用于拖尾方向计算）
      const velAttr = geo.attributes.aVelocity as THREE.BufferAttribute
      const velArr = velAttr.array as Float32Array
      velArr.set(vel)
      velAttr.needsUpdate = true

      // 更新 shader uniform
      const mat = pointsRef.current.material as THREE.ShaderMaterial
      mat.uniforms.uTime.value = t
      mat.uniforms.uLow.value = low
      mat.uniforms.uHi.value = hi
      mat.uniforms.uOnset.value = d.onset
      mat.uniforms.uViewport.value.set(size.width, size.height)
    }

    // ===== 爆发粒子（onset 触发）=====
    if (d.onset > 0.3 && burstActiveRef.current < BURST_COUNT - 5) {
      // 生成一批新粒子
      const spawnCount = Math.min(5, BURST_COUNT - burstActiveRef.current)
      for (let b = 0; b < spawnCount; b++) {
        const idx = burstActiveRef.current
        if (idx >= BURST_COUNT) break

        // 从中心附近随机位置出发
        const theta = Math.random() * Math.PI * 2
        const phi = Math.acos(2 * Math.random() - 1)
        const startR = 1 + Math.random() * 0.5

        const burstGeo = burstPointsRef.current?.geometry
        if (!burstGeo) break
        const bpAttr = burstGeo.attributes.position as THREE.BufferAttribute
        const bp = bpAttr.array as Float32Array

        bp[idx * 3] = Math.sin(phi) * Math.cos(theta) * startR - 0.5
        bp[idx * 3 + 1] = Math.sin(phi) * Math.sin(theta) * startR
        bp[idx * 3 + 2] = Math.cos(phi) * startR

        // 速度：向外飞散
        const speed = 0.15 + Math.random() * 0.15 + d.onset * 0.2
        burstVelocitiesRef.current[idx * 3] = Math.sin(phi) * Math.cos(theta) * speed
        burstVelocitiesRef.current[idx * 3 + 1] = Math.sin(phi) * Math.sin(theta) * speed + 0.05
        burstVelocitiesRef.current[idx * 3 + 2] = Math.cos(phi) * speed

        burstLifesRef.current[idx] = 1.0
        burstActiveRef.current++
        bpAttr.needsUpdate = true
      }
    }

    // 更新爆发粒子
    if (burstPointsRef.current && burstActiveRef.current > 0) {
      const geo = burstPointsRef.current.geometry
      const posAttr = geo.attributes.position as THREE.BufferAttribute
      const pos = posAttr.array as Float32Array
      const lifeAttr = geo.attributes.aLife as THREE.BufferAttribute
      const lives = lifeAttr.array as Float32Array

      for (let i = 0; i < burstActiveRef.current; i++) {
        const i3 = i * 3
        pos[i3] += burstVelocitiesRef.current[i3]
        pos[i3 + 1] += burstVelocitiesRef.current[i3 + 1]
        pos[i3 + 2] += burstVelocitiesRef.current[i3 + 2]

        // 重力 + 减速
        burstVelocitiesRef.current[i3 + 1] -= 0.002
        burstVelocitiesRef.current[i3] *= 0.98
        burstVelocitiesRef.current[i3 + 1] *= 0.98
        burstVelocitiesRef.current[i3 + 2] *= 0.98

        burstLifesRef.current[i] -= 0.015
        lives[i] = Math.max(0, burstLifesRef.current[i])
      }

      // 清理死掉的粒子（用最后一个填充）
      while (burstActiveRef.current > 0 && burstLifesRef.current[burstActiveRef.current - 1] <= 0) {
        burstActiveRef.current--
      }

      posAttr.needsUpdate = true
      lifeAttr.needsUpdate = true
    }
  })

  // 主粒子几何体
  const mainGeo = useMemo(() => {
    const positions = new Float32Array(PARTICLE_COUNT * 3)
    const sizes = new Float32Array(PARTICLE_COUNT)
    const phases = new Float32Array(PARTICLE_COUNT)
    const velocities = new Float32Array(PARTICLE_COUNT * 3)

    for (let i = 0; i < PARTICLE_COUNT; i++) {
      // 球壳分布，中心密外围疏
      const theta = Math.random() * Math.PI * 2
      const phi = Math.acos(2 * Math.random() - 1)
      const r = 4 + Math.pow(Math.random(), 0.6) * 16

      positions[i * 3] = Math.sin(phi) * Math.cos(theta) * r
      positions[i * 3 + 1] = Math.sin(phi) * Math.sin(theta) * r - 2
      positions[i * 3 + 2] = Math.cos(phi) * r

      sizes[i] = 0.3 + Math.random() * 0.45
      phases[i] = Math.random() * Math.PI * 2
      // 初始速度为 0，useFrame 里更新
      velocities[i * 3] = 0
      velocities[i * 3 + 1] = 0
      velocities[i * 3 + 2] = 0
    }

    const g = new THREE.BufferGeometry()
    g.setAttribute('position', new THREE.BufferAttribute(positions, 3))
    g.setAttribute('aSize', new THREE.BufferAttribute(sizes, 1))
    g.setAttribute('aPhase', new THREE.BufferAttribute(phases, 1))
    g.setAttribute('aVelocity', new THREE.BufferAttribute(velocities, 3))
    return g
  }, [])

  const mainMat = useMemo(() => new THREE.ShaderMaterial({
    uniforms: {
      uTime: { value: 0 },
      uLow: { value: 0 },
      uHi: { value: 0 },
      uOnset: { value: 0 },
      uNebula: { value: 0.0 },
      uViewport: { value: new THREE.Vector2(1, 1) },
    },
    transparent: true,
    depthWrite: false,
    blending: THREE.AdditiveBlending,
    vertexShader: /* glsl */`
      attribute float aSize;
      attribute float aPhase;
      attribute vec3 aVelocity;
      uniform float uTime;
      uniform float uLow;
      uniform float uHi;
      uniform float uOnset;
      uniform vec2 uViewport;
      varying float vAlpha;
      varying float vDist;
      varying vec2 vVelDir; // 屏幕空间速度方向
      varying float vSpeed; // 速度大小

      void main() {
        // 闪烁
        float twinkle = sin(uTime * 1.5 + aPhase * 6.28) * 0.3 + 0.7;

        // 尺寸随音乐变化
        float size = aSize * (0.8 + uLow * 0.5 + uHi * 0.8 + uOnset * 0.6);

        // 透明度：远距离更淡
        vec4 mv = modelViewMatrix * vec4(position, 1.0);
        vDist = -mv.z;
        float distFade = 1.0 - smoothstep(12.0, 35.0, vDist);

        vAlpha = (0.3 + twinkle * 0.4 + uHi * 0.25) * distFade;

        // 计算屏幕空间速度方向
        vec4 mvVel = modelViewMatrix * vec4(position + aVelocity * 30.0, 1.0);
        vec4 projPos = projectionMatrix * mv;
        vec4 projVel = projectionMatrix * mvVel;

        // 转换到屏幕空间（像素）
        vec2 screenPos = projPos.xy / projPos.w * uViewport * 0.5;
        vec2 screenVelPos = projVel.xy / projVel.w * uViewport * 0.5;
        vec2 velScreen = screenVelPos - screenPos;

        vSpeed = length(velScreen);
        vVelDir = vSpeed > 0.001 ? normalize(velScreen) : vec2(1.0, 0.0);

        gl_Position = projPos;
        // 点大小：速度越快拖尾越长，点的尺寸也要够大
        float tailLength = min(vSpeed * 0.08 + size * 6.0, size * 15.0);
        gl_PointSize = max(size * 6.0, tailLength);
      }
    `,
    fragmentShader: /* glsl */`
      varying float vAlpha;
      varying float vDist;
      varying vec2 vVelDir;
      varying float vSpeed;
      uniform float uLow;
      uniform float uNebula;

      void main() {
        vec2 uv = gl_PointCoord - 0.5;

        // 旋转到速度方向
        vec2 dir = vVelDir;
        mat2 rot = mat2(dir.x, -dir.y, dir.y, dir.x);
        vec2 rotated = rot * uv;

        // 沿速度方向（x轴）拉伸，做成拖尾形状
        // 头部（+x方向）圆钝，尾部（-x方向）尖锐
        float tailSharpness = 0.5 + min(vSpeed * 0.005, 1.0) * 0.8;

        // x轴：头部 0.5，尾部占更多
        float xNorm = rotated.x + 0.5; // 0~1，0=尾，1=头

        // 沿y轴的宽度：头部宽，尾部尖
        float widthAtX;
        if (xNorm > 0.4) {
          // 头部：半圆形
          float headX = (xNorm - 0.4) / 0.6; // 0~1
          widthAtX = sqrt(1.0 - headX * headX) * 0.5;
        } else {
          // 尾部：线性收窄
          widthAtX = xNorm / 0.4 * 0.5 * (1.0 - tailSharpness * 0.6);
        }

        // 超出形状的丢弃
        if (abs(rotated.y) > widthAtX) discard;
        if (rotated.x < -0.5) discard;

        // 透明度渐变：头部亮，尾部淡
        float headGlow = smoothstep(0.0, 1.0, xNorm);
        float tailFade = pow(xNorm, 0.7);

        float alpha = tailFade * vAlpha;
        // 头部加一点高光
        alpha += headGlow * vAlpha * 0.3 * min(vSpeed * 0.01, 1.0);

        // 颜色：冷白偏蓝，低频时偏暖紫；星云模式用紫蓝粉
        vec3 colDefault = mix(vec3(0.7, 0.8, 1.0), vec3(0.85, 0.75, 1.0), uLow);
        vec3 colNebula = mix(vec3(0.6, 0.5, 1.0), vec3(1.0, 0.6, 0.9), uLow);
        vec3 col = mix(colDefault, colNebula, uNebula);

        gl_FragColor = vec4(col, alpha);
      }
    `,
  }), [])

  // 爆发粒子几何体
  const burstGeo = useMemo(() => {
    const positions = new Float32Array(BURST_COUNT * 3)
    const sizes = new Float32Array(BURST_COUNT)
    const lives = new Float32Array(BURST_COUNT)

    for (let i = 0; i < BURST_COUNT; i++) {
      sizes[i] = 0.8 + Math.random() * 1
      lives[i] = 0
    }

    const g = new THREE.BufferGeometry()
    g.setAttribute('position', new THREE.BufferAttribute(positions, 3))
    g.setAttribute('aSize', new THREE.BufferAttribute(sizes, 1))
    g.setAttribute('aLife', new THREE.BufferAttribute(lives, 1))
    return g
  }, [])

  const burstMat = useMemo(() => new THREE.ShaderMaterial({
    transparent: true,
    depthWrite: false,
    blending: THREE.AdditiveBlending,
    vertexShader: /* glsl */`
      attribute float aSize;
      attribute float aLife;
      varying float vLife;

      void main() {
        vLife = aLife;
        vec4 mv = modelViewMatrix * vec4(position, 1.0);
        gl_Position = projectionMatrix * mv;
        gl_PointSize = aSize * aLife * (250.0 / -mv.z);
      }
    `,
    fragmentShader: /* glsl */`
      varying float vLife;
      void main() {
        vec2 uv = gl_PointCoord - 0.5;
        float d = length(uv);
        if (d > 0.5) discard;
        float glow = smoothstep(0.5, 0.0, d);
        vec3 col = vec3(1.0, 0.95, 0.85);
        gl_FragColor = vec4(col, glow * vLife * 0.9);
      }
    `,
  }), [])

  return (
    <group position={[0, 0, 0]}>
      {/* 主流场粒子 */}
      <points ref={pointsRef} geometry={mainGeo} material={mainMat} />
      {/* 爆发粒子 */}
      <points ref={burstPointsRef} geometry={burstGeo} material={burstMat} />
    </group>
  )
}
