import { useMemo, useRef } from 'react'
import { useFrame, useThree } from '@react-three/fiber'
import type { MutableRefObject } from 'react'
import * as THREE from 'three'
import { bakePlanetMaps, NOISE_UTILS, ALBEDO_PLACEHOLDER, CLOUD_PLACEHOLDER } from './planetBake'
import type { SmoothedAudioData } from './audioBridge'

interface Props {
  audioRef: MutableRefObject<SmoothedAudioData>
  quality?: string
}

const PLANET_RADIUS = 4.5

function qualityPreset(quality?: string) {
  switch (quality) {
    case 'low':   return { planetSeg: 64,  cloudSeg: 48,  atmoSeg: 24, ringSeg: 128 }
    case 'mid':   return { planetSeg: 128, cloudSeg: 80,  atmoSeg: 40, ringSeg: 200 }
    case 'ultra': return { planetSeg: 256, cloudSeg: 192, atmoSeg: 96, ringSeg: 480 }
    case 'high':
    default:      return { planetSeg: 192, cloudSeg: 128, atmoSeg: 64, ringSeg: 320 }
  }
}

// 星云主题：行星自转 + 单环带（无卫星/无粒子环——场景即控制台界面）
export function NebulaCore({ audioRef, quality }: Props) {
  const groupRef = useRef<THREE.Group>(null)
  const planetRef = useRef<THREE.Mesh>(null)
  const cloudLayerRef = useRef<THREE.Mesh>(null)
  const atmoRef = useRef<THREE.Mesh>(null)
  const planetRingRef = useRef<THREE.Mesh>(null)

  const q = qualityPreset(quality)
  const gl = useThree((s) => s.gl)

  const lowSmRef = useRef(0)
  const rmsSmRef = useRef(0)
  const hiSmRef = useRef(0)
  const beatPulseRef = useRef(0)
  const lastBeatRef = useRef(false)
  const beatCooldownRef = useRef(0)
  const startTimeRef = useRef(0)
  const initFadeRef = useRef(0) // 启动淡入：0→1，约 1.1 秒
  const ringWaveRef = useRef(1) // 环带节拍波进度：1 = 空闲，beat 时归 0

  // ─── 行星表面 shader（烘焙版） ─── 反照率/夜面发光来自烘焙纹理；
  // 运行时只做光照、极光（瘦身）、海面高光、环影与移动云影——
  // 每像素噪声成本从 ~30 八度降到 ~3（烘焙时八度加倍，细节免费提升）
  const planetMat = useMemo(() => new THREE.ShaderMaterial({
    uniforms: {
      uTime: { value: 0 },
      uLow: { value: 0 },
      uHigh: { value: 0 },
      uLightDir: { value: new THREE.Vector3(0.85, 0.2, 0.5).normalize() },
      uAlbedo: { value: ALBEDO_PLACEHOLDER },
      uCloudTex: { value: CLOUD_PLACEHOLDER },
    },
    vertexShader: /* glsl */`
      varying vec3 vNormal;
      varying vec3 vPos;
      varying vec3 vWorldPos;
      void main() {
        vNormal = normalize(normalMatrix * normal);
        vPos = position;
        vec4 wp = modelMatrix * vec4(position, 1.0);
        vWorldPos = wp.xyz;
        gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
      }
    `,
    fragmentShader: /* glsl */`
      uniform sampler2D uAlbedo;
      uniform sampler2D uCloudTex;
      uniform float uTime;
      uniform float uLow;
      uniform float uHigh;
      uniform vec3 uLightDir;
      varying vec3 vNormal;
      varying vec3 vPos;
      varying vec3 vWorldPos;

      ${NOISE_UTILS}

      void main() {
        vec3 dir = normalize(vPos);
        // 等距圆柱 UV：与烘焙贴图的展开映射互为逆变换
        vec2 uvE = vec2(atan(dir.z, dir.x) / 6.28318530718 + 0.5, asin(clamp(dir.y, -1.0, 1.0)) / 3.14159265359 + 0.5);
        vec4 alb = texture2D(uAlbedo, uvE);
        // 云 + 海洋掩码（漂移速率 = 云层自转差 + 纹理独立漂移）
        vec2 cmUv = vec2(uvE.x + uTime * 0.0104, uvE.y);
        vec2 cm = texture2D(uCloudTex, fract(cmUv)).rg;
        float isOcean = cm.g;

        vec3 N = normalize(vNormal);
        float NdotL = dot(N, uLightDir);
        float lit = smoothstep(-0.15, 0.4, NdotL);
        float halfLit = smoothstep(-0.25, 0.15, NdotL);

        // 黄昏色带（更宽更柔和）
        float terminator = smoothstep(-0.05, 0.12, NdotL) * (1.0 - smoothstep(0.12, 0.35, NdotL));
        vec3 sunset1 = vec3(1.0, 0.45, 0.18);
        vec3 sunset2 = vec3(0.9, 0.25, 0.5);
        vec3 sunset = mix(sunset2, sunset1, terminator) * terminator * 0.45;

        // 海洋镜面反射（掩码来自烘焙）
        vec3 viewDir = normalize(cameraPosition - vWorldPos);
        vec3 halfVec = normalize(uLightDir + viewDir);
        float spec = pow(max(0.0, dot(N, halfVec)), 64.0);
        float waveN = fbm(dir * 25.0 + vec3(uTime * 0.02, 0, uTime * 0.015), 2);
        float specMask = spec * isOcean * lit * (0.6 + waveN * 0.4);
        vec3 oceanSpec = vec3(0.9, 0.95, 1.0) * specMask * 0.8;

        // 极光（夜面高纬度，瘦身：单八度噪声 + 复用）
        float auroraLat = smoothstep(0.5, 0.8, abs(dir.y));
        float auroraBand = sin(dir.y * 12.0 + uTime * 0.3) * 0.5 + 0.5;
        float auroraN = fbm(dir * 6.0 + vec3(0, uTime * 0.1, 0), 1);
        float aurora = auroraLat * auroraBand * auroraN * (1.0 - halfLit) * (0.55 + uHigh * 2.0) * 0.6;
        vec3 auroraColor = mix(vec3(0.2, 1.0, 0.5), vec3(0.4, 0.6, 1.0), auroraN);
        vec3 auroraGlow = auroraColor * aurora * 0.5;

        // 环带投影：几何对齐时的细暗弧（环带 6.5~8.55）
        vec3 pg = vWorldPos + vec3(0.0, 0.5, 0.0);
        float tR = -pg.y / uLightDir.y;
        float ringShadow = 0.0;
        if (tR > 0.0) {
          vec3 cR = pg + uLightDir * tR;
          float rc = length(cR.xz);
          ringShadow = smoothstep(6.3, 7.0, rc) * (1.0 - smoothstep(7.9, 8.9, rc)) * 0.55;
        }
        float litShaded = lit * (1.0 - ringShadow);

        // 真移动云影：云密度作为遮光量（随漂移移动）
        float cloudShadow = 1.0 - smoothstep(0.35, 0.85, cm.r) * 0.32 * lit;

        float ambient = 0.07 + uLow * 0.05;

        vec3 finalCol = alb.rgb * (ambient + litShaded * 0.93) * cloudShadow;
        finalCol += oceanSpec;
        finalCol += sunset * isOcean * 0.6;
        // 夜面发光（城市/火山——烘焙 a 通道，暖色）
        finalCol += vec3(1.0, 0.62, 0.25) * alb.a * (1.0 - halfLit) * 0.85;
        finalCol += auroraGlow;
        finalCol += alb.rgb * uLow * 0.35;

        gl_FragColor = vec4(finalCol, 1.0);
      }
    `,
  }), [])

  // ─── 云层 shader（烘焙版） ─── 密度来自烘焙纹理 + 独立慢漂移
  const cloudLayerMat = useMemo(() => new THREE.ShaderMaterial({
    transparent: true,
    depthWrite: false,
    uniforms: {
      uTime: { value: 0 },
      uLow: { value: 0 },
      uLightDir: { value: new THREE.Vector3(0.85, 0.2, 0.5).normalize() },
      uCloudTex: { value: CLOUD_PLACEHOLDER },
    },
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
      uniform sampler2D uCloudTex;
      uniform float uTime;
      uniform float uLow;
      uniform vec3 uLightDir;
      varying vec3 vNormal;
      varying vec3 vPos;
      void main() {
        vec3 p = normalize(vPos);
        vec2 uvE = vec2(atan(p.z, p.x) / 6.28318530718 + 0.5, asin(clamp(p.y, -1.0, 1.0)) / 3.14159265359 + 0.5);
        float density = texture2D(uCloudTex, vec2(uvE.x + uTime * 0.008, uvE.y)).r;
        float alpha = smoothstep(0.52, 0.75, density);

        float NdotL = dot(normalize(vNormal), uLightDir);
        float lit = smoothstep(-0.1, 0.35, NdotL);

        float cloudLight = 0.3 + lit * 0.7;
        float edge = smoothstep(0.48, 0.55, density) * (1.0 - smoothstep(0.65, 0.78, density));
        cloudLight += edge * lit * 0.35;

        vec3 cloudCol = vec3(0.92, 0.94, 1.0) * cloudLight;
        float terminator = smoothstep(-0.05, 0.15, NdotL) * (1.0 - smoothstep(0.15, 0.45, NdotL));
        cloudCol = mix(cloudCol, vec3(1.0, 0.55, 0.25), terminator * 0.5);
        float polar = smoothstep(0.45, 0.85, abs(p.y));
        cloudCol = mix(cloudCol, vec3(0.72, 0.82, 1.0), polar * 0.35);
        cloudCol *= 0.4 + lit * 0.6;

        gl_FragColor = vec4(cloudCol, alpha * (0.3 + uLow * 0.35));
      }
    `,
  }), [])

  // ─── 大气 ─── 内外辉光合一（省一层全盘透明 pass）
  // 原内外两层 BackSide 壳（1.01R 紧贴亮环 + 1.12R 宽软外辉）改为在同一张壳上
  // 用两个不同幂次的菲涅尔项复现：f1 紧（pow 2.8）、f2 宽（pow 1.6）。
  const atmoMat = useMemo(() => new THREE.ShaderMaterial({
    transparent: true,
    depthWrite: false,
    blending: THREE.AdditiveBlending,
    side: THREE.BackSide,
    uniforms: {
      uIntensity: { value: 0.5 },
      uLow: { value: 0 },
      uLightDir: { value: new THREE.Vector3(0.85, 0.2, 0.5).normalize() },
    },
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
      uniform float uIntensity;
      uniform float uLow;
      uniform vec3 uLightDir;
      varying vec3 vNormal;
      varying vec3 vPos;
      void main() {
        vec3 viewDir = normalize(cameraPosition - vPos);
        float ndv = abs(dot(normalize(vNormal), viewDir));
        float f1 = pow(1.0 - ndv, 2.8);  // 贴边亮环
        float f2 = pow(1.0 - ndv, 2.2);  // 宽软外辉（收紧，避免大范围泛光）
        // 光面大气更亮（类似日出日落的辉光）
        float lightSide = max(0.0, dot(normalize(vPos), uLightDir));
        float rimGlow = pow(lightSide, 2.0) * 0.5;
        vec3 innerCol = mix(vec3(0.35, 0.55, 1.0), vec3(0.65, 0.35, 0.95), uLow * 0.7);
        innerCol = mix(innerCol, vec3(1.0, 0.5, 0.2), rimGlow * 0.6);
        vec3 outerCol = mix(vec3(0.2, 0.4, 0.9), vec3(0.5, 0.2, 0.85), uLow * 0.5);
        float a1 = f1 * (0.3 + uIntensity * 0.4 + rimGlow * 0.25);
        float a2 = f2 * (0.04 + uIntensity * 0.08);
        float a = min(a1 + a2, 1.0);
        vec3 col = (innerCol * a1 + outerCol * a2) / max(a1 + a2, 1e-4);
        gl_FragColor = vec4(col, a);
      }
    `,
  }), [])


  useFrame(({ clock }, delta) => {
    const d = audioRef.current
    const t = clock.elapsedTime
    // 记录启动时间
    if (startTimeRef.current === 0) startTimeRef.current = t
    const elapsed = t - startTimeRef.current
    // 启动淡入：音频响应前 ~1.1s 从 0 渐升到 1，避免进入瞬间闪烁
    // （delta 驱动，帧率变化不影响时长）
    initFadeRef.current = Math.min(1, initFadeRef.current + delta * 0.9)
    const fade = initFadeRef.current


    lowSmRef.current = lerp(lowSmRef.current, d.lowFreqAvg * fade, 0.08)
    rmsSmRef.current = lerp(rmsSmRef.current, d.rms * fade, 0.06)
    hiSmRef.current = lerp(hiSmRef.current, d.highFreqAvg * fade, 0.05)

    // beat 冷却：避免连续触发导致闪烁
    beatCooldownRef.current = Math.max(0, beatCooldownRef.current - 1)
    // 启动前 0.8 秒完全屏蔽 beat
    if (elapsed > 0.8 && d.beat && !lastBeatRef.current && beatCooldownRef.current <= 0) {
      beatPulseRef.current = 0.18 * fade
      // 波必须走完才允许下一道——重低音时 beat 间隔(~0.5s)短于波行进时间
      // (1.2s)，连续重置会让波永远卡在内缘重启，表现为内缘频闪（用户反馈）
      if (ringWaveRef.current >= 0.999) ringWaveRef.current = 0
      beatCooldownRef.current = 8 // 至少 8 帧冷却（约 130ms @60fps）
    }
    beatPulseRef.current *= 0.68
    lastBeatRef.current = d.beat

    const low = lowSmRef.current
    const rms = rmsSmRef.current
    const pulse = beatPulseRef.current

    // 计算云阴影（极弱，避免行星表面闪烁）


    // 行星自转
    if (planetRef.current) {
      planetRef.current.rotation.y = t * 0.04
      // 呼吸（低频）+ 节拍踢一脚：±4.5% 的 scale 脉冲是可感知的下限
      const scale = 1 + low * 0.035 + pulse * 0.045
      planetRef.current.scale.setScalar(scale)
      const mat = planetRef.current.material as THREE.ShaderMaterial
      // 烘焙贴图：首次调用触发一次 GPU 烘焙并缓存（入场渐显掩盖）
      const baked = bakePlanetMaps(gl)
      mat.uniforms.uAlbedo.value = baked.albedo
      mat.uniforms.uCloudTex.value = baked.cloud
      ;(window as any).__planetUniforms = mat.uniforms
      mat.uniforms.uTime.value = t
      mat.uniforms.uLow.value = low
      mat.uniforms.uHigh.value = hiSmRef.current

    }

    // 云层
    if (cloudLayerRef.current) {
      cloudLayerRef.current.rotation.y = t * 0.055
      const mat = cloudLayerRef.current.material as THREE.ShaderMaterial
      mat.uniforms.uCloudTex.value = bakePlanetMaps(gl).cloud
      mat.uniforms.uTime.value = t
      mat.uniforms.uLow.value = low
    }

    // 大气（合并壳）：半径取两壳中点，呼吸幅度加大到可感知档
    if (atmoRef.current) {
      atmoRef.current.rotation.y = -t * 0.025
      const scale = 1.065 + low * 0.012 + pulse * 0.009
      atmoRef.current.scale.setScalar(scale)
      // 大气辉光：低频响应加档——行星的"呼吸"要能与环带的反应抗衡
      const mat = atmoRef.current.material as THREE.ShaderMaterial
      mat.uniforms.uIntensity.value = 0.4 + low * 0.5 + rms * 0.15 + pulse * 0.35
      mat.uniforms.uLow.value = low
    }

    // 行星环
    if (planetRingRef.current) {
      planetRingRef.current.rotation.z = Math.sin(t * 0.08) * 0.04
      const mat = planetRingRef.current.material as THREE.ShaderMaterial
      mat.uniforms.uTime.value = t
      mat.uniforms.uLow.value = low
      mat.uniforms.uPulse.value = pulse
      ringWaveRef.current = Math.min(1, ringWaveRef.current + delta / 1.2)
      mat.uniforms.uWave.value = ringWaveRef.current
    }

  })

  return (
    <group ref={groupRef} position={[0, -0.5, 0]}>
      {/* 行星 */}
      <mesh ref={planetRef}>
        <sphereGeometry args={[PLANET_RADIUS, q.planetSeg, q.planetSeg]} />
        <primitive object={planetMat} attach="material" />
      </mesh>

      {/* 云层 */}
      <mesh ref={cloudLayerRef}>
        <sphereGeometry args={[PLANET_RADIUS * 1.01, q.cloudSeg, q.cloudSeg]} />
        <primitive object={cloudLayerMat} attach="material" />
      </mesh>

      {/* 大气（内外辉光合一） */}
      <mesh ref={atmoRef}>
        <sphereGeometry args={[PLANET_RADIUS, q.atmoSeg, q.atmoSeg]} />
        <primitive object={atmoMat} attach="material" />
      </mesh>
    </group>
  )
}

function lerp(a: number, b: number, t: number) { return a + (b - a) * t }
