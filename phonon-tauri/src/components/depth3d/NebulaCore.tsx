import { useEffect, useMemo, useRef } from 'react'
import { useFrame } from '@react-three/fiber'
import type { MutableRefObject } from 'react'
import * as THREE from 'three'
import type { SmoothedAudioData } from './audioBridge'

interface Props {
  audioRef: MutableRefObject<SmoothedAudioData>
  quality?: string
}

const PLANET_RADIUS = 4.5

function qualityPreset(quality?: string) {
  switch (quality) {
    case 'low':   return { planetSeg: 64,  cloudSeg: 48,  atmoSeg: 24,  nebulaN: 1000, asteroidN: 300, cloudN: 200, ringSeg: 128 }
    case 'mid':   return { planetSeg: 128, cloudSeg: 80,  atmoSeg: 40,  nebulaN: 1800, asteroidN: 500, cloudN: 350, ringSeg: 200 }
    case 'ultra': return { planetSeg: 256, cloudSeg: 192, atmoSeg: 96,  nebulaN: 3500, asteroidN: 1200, cloudN: 700, ringSeg: 480 }
    case 'high':
    default:      return { planetSeg: 192, cloudSeg: 128, atmoSeg: 64,  nebulaN: 2500, asteroidN: 800, cloudN: 500, ringSeg: 320 }
  }
}

// 通用噪声工具函数（字符串形式，便于 shader 内拼接）
const NOISE_UTILS = /* glsl */`
  float hash(vec3 p) {
    p = fract(p * 0.3183099 + 0.1);
    p *= 17.0;
    return fract(p.x * p.y * p.z * (p.x + p.y + p.z));
  }
  float noise(vec3 p) {
    vec3 i = floor(p);
    vec3 f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    return mix(
      mix(mix(hash(i), hash(i + vec3(1,0,0)), f.x),
          mix(hash(i + vec3(0,1,0)), hash(i + vec3(1,1,0)), f.x), f.y),
      mix(mix(hash(i + vec3(0,0,1)), hash(i + vec3(1,0,1)), f.x),
          mix(hash(i + vec3(0,1,1)), hash(i + vec3(1,1,1)), f.x), f.y), f.z);
  }
  float fbm(vec3 p, int oct) {
    float v = 0.0;
    float a = 0.5;
    for (int i = 0; i < 8; i++) {
      if (i >= oct) break;
      v += a * noise(p);
      p = p * 2.05 + vec3(1.7, 9.2, 3.3);
      a *= 0.5;
    }
    return v;
  }
  float ridge(vec3 p, int oct) {
    float v = 0.0;
    float a = 0.5;
    for (int i = 0; i < 6; i++) {
      if (i >= oct) break;
      float n = noise(p);
      v += a * abs(n * 2.0 - 1.0);
      p = p * 2.1 + vec3(5.3, 1.1, 7.7);
      a *= 0.55;
    }
    return v;
  }
`

// 星云主题：行星自转 + 双卫星 + 多环带 + 小行星带 + 星云环绕
export function NebulaCore({ audioRef, quality }: Props) {
  const groupRef = useRef<THREE.Group>(null)
  const planetRef = useRef<THREE.Mesh>(null)
  const cloudLayerRef = useRef<THREE.Mesh>(null)
  const atmoRef = useRef<THREE.Mesh>(null)
  const ringRef = useRef<THREE.Points>(null)
  const cloudRef = useRef<THREE.Points>(null)
  const moonRef = useRef<THREE.Mesh>(null)
  const moon2Ref = useRef<THREE.Mesh>(null)
  const planetRingRef = useRef<THREE.Mesh>(null)
  const asteroidRef = useRef<THREE.Points>(null)

  const q = qualityPreset(quality)

  const lowSmRef = useRef(0)
  const rmsSmRef = useRef(0)
  const hiSmRef = useRef(0)
  const beatPulseRef = useRef(0)
  const lastBeatRef = useRef(false)
  const beatCooldownRef = useRef(0)
  const startTimeRef = useRef(0)
  const initFadeRef = useRef(0) // 启动淡入：0→1，前 1.5 秒

  // ─── 行星表面 shader ─── 超精细：地形8层 + 海洋高光 + 云影 + 极光 + 火山 + 城市光
  const planetMat = useMemo(() => new THREE.ShaderMaterial({
    uniforms: {
      uTime: { value: 0 },
      uLow: { value: 0 },
      uHigh: { value: 0 },
      uLightDir: { value: new THREE.Vector3(0.85, 0.2, 0.5).normalize() },
      uCloudShadow: { value: 0 },
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
      uniform float uTime;
      uniform float uLow;
      uniform float uHigh;
      uniform vec3 uLightDir;
      uniform float uCloudShadow;
      varying vec3 vNormal;
      varying vec3 vPos;
      varying vec3 vWorldPos;

      ${NOISE_UTILS}

      void main() {
        vec3 p = normalize(vPos);

        // ── 地形高度（fbm + 山脊 + 细节） ──
        // 八度数经过性能预算削减：原始版本每像素 33+ 次 fbm，
        // 在 4K/dpr2 下帧时间超出 WebView2 合成容忍度 → 间歇输出
        // 纯黑帧（整窗闪屏）。削减八度后观感差异很小，帧率恢复正常。
        float continents = fbm(p * 1.2, 4) * 0.9;
        float mountains = ridge(p * 3.0, 3) * 0.5;
        float hills = fbm(p * 6.0, 3) * 0.25;
        float microDetail = fbm(p * 18.0, 2) * 0.08;
        // 注意：GLSL 不允许在初始化表达式中引用正在声明的变量。
        // 旧代码在 height 的初始化里引用了 height 自身（height * 0.0 + ...），
        // 导致整个片元着色器编译失败，星球退化成默认材质。
        float height = continents
          + mountains * smoothstep(0.42, 0.58, continents)
          + hills * smoothstep(0.5, 0.65, continents)
          + microDetail;

        // 海岸线精细度
        float coast = fbm(p * 8.0, 2) * 0.06;
        height += coast * smoothstep(0.44, 0.52, height);

        // ── 分层配色（10 层地形） ──
        vec3 deepOcean    = vec3(0.012, 0.03, 0.12);
        vec3 ocean        = vec3(0.03, 0.07, 0.25);
        vec3 shallowSea   = vec3(0.07, 0.14, 0.38);
        vec3 beach        = vec3(0.28, 0.25, 0.5);
        vec3 lowland      = vec3(0.16, 0.09, 0.28);
        vec3 forest       = vec3(0.12, 0.22, 0.18);
        vec3 highland     = vec3(0.32, 0.18, 0.48);
        vec3 mountain     = vec3(0.48, 0.33, 0.62);
        vec3 highMountain = vec3(0.6, 0.5, 0.72);
        vec3 snow         = vec3(0.62, 0.68, 0.82);

        vec3 col = deepOcean;
        col = mix(col, ocean,        smoothstep(0.33, 0.40, height));
        col = mix(col, shallowSea,   smoothstep(0.40, 0.45, height));
        col = mix(col, beach,        smoothstep(0.45, 0.475, height));
        col = mix(col, lowland,      smoothstep(0.475, 0.53, height));
        col = mix(col, forest,       smoothstep(0.53, 0.60, height) * 0.6);
        col = mix(col, highland,     smoothstep(0.58, 0.70, height));
        col = mix(col, mountain,     smoothstep(0.72, 0.82, height));
        col = mix(col, highMountain, smoothstep(0.80, 0.88, height));
        col = mix(col, snow,         smoothstep(0.86, 0.93, height));

        // 极地冰盖（随纬度变化，边缘有渐变）
        float polar = smoothstep(0.55, 0.88, abs(p.y));
        float polarEdge = smoothstep(0.55, 0.7, abs(p.y)) - smoothstep(0.75, 0.88, abs(p.y));
        col = mix(col, snow * 0.82, polar * 0.6);
        // 极地冰盖边缘的苔原色
        col = mix(col, vec3(0.3, 0.35, 0.45), polarEdge * smoothstep(0.48, 0.55, height) * 0.4);

        // ── 光照计算 ──
        vec3 N = normalize(vNormal);
        float NdotL = dot(N, uLightDir);
        float lit = smoothstep(-0.15, 0.4, NdotL);
        float halfLit = smoothstep(-0.25, 0.15, NdotL);

        // 黄昏色带（更宽更柔和）
        float terminator = smoothstep(-0.05, 0.12, NdotL) * (1.0 - smoothstep(0.12, 0.35, NdotL));
        vec3 sunset1 = vec3(1.0, 0.45, 0.18);
        vec3 sunset2 = vec3(0.9, 0.25, 0.5);
        vec3 sunset = mix(sunset2, sunset1, terminator) * terminator * 0.45;

        // ── 海洋镜面反射 ──
        float isOcean = 1.0 - smoothstep(0.44, 0.47, height);
        vec3 viewDir = normalize(cameraPosition - vWorldPos);
        vec3 halfVec = normalize(uLightDir + viewDir);
        float spec = pow(max(0.0, dot(N, halfVec)), 64.0);
        // 海浪微表面扰动高光
        float waveN = fbm(p * 25.0 + vec3(uTime * 0.02, 0, uTime * 0.015), 2);
        float specMask = spec * isOcean * lit * (0.6 + waveN * 0.4);
        vec3 oceanSpec = vec3(0.9, 0.95, 1.0) * specMask * 0.8;

        // ── 极光（夜面高纬度） ──
        float auroraLat = smoothstep(0.5, 0.8, abs(p.y));
        float auroraBand = sin(p.y * 12.0 + uTime * 0.3) * 0.5 + 0.5;
        float auroraN = fbm(p * 6.0 + vec3(0, uTime * 0.1, 0), 3);
        // 极光亮度跟高频（镲片/气声点亮夜面极光带）
        float aurora = auroraLat * auroraBand * auroraN * (1.0 - halfLit) * (0.55 + uHigh * 2.0) * 0.6;
        vec3 auroraCol1 = vec3(0.2, 1.0, 0.5);
        vec3 auroraCol2 = vec3(0.4, 0.6, 1.0);
        vec3 auroraCol3 = vec3(0.8, 0.3, 1.0);
        float auroraMix = fbm(p * 4.0 + vec3(uTime * 0.05), 3);
        vec3 auroraColor = mix(mix(auroraCol1, auroraCol2, auroraMix), auroraCol3, auroraMix * 0.3);
        vec3 auroraGlow = auroraColor * aurora * 0.5;

        // ── 火山热点（山脉区域，夜面可见） ──
        float volcanoMask = smoothstep(0.72, 0.8, height) * ridge(p * 5.0, 3) * 0.5;
        float volcanoN = fbm(p * 15.0, 2);
        float volcanoes = smoothstep(0.65, 0.78, volcanoN) * volcanoMask;
        vec3 volcanoGlow = vec3(1.0, 0.35, 0.08) * volcanoes * (1.0 - lit) * 0.4;

        // ── 夜面城市光 ──
        float cityN = fbm(p * 22.0, 3);
        float cities = smoothstep(0.68, 0.74, cityN) * (1.0 - halfLit);
        cities *= smoothstep(0.48, 0.58, height);
        // 大城市中心更亮
        float cityCenters = smoothstep(0.75, 0.82, cityN) * smoothstep(0.52, 0.62, height);
        vec3 cityGlow = vec3(1.0, 0.72, 0.3) * cities * 0.22;
        cityGlow += vec3(1.0, 0.85, 0.5) * cityCenters * 0.15 * (1.0 - halfLit);

        // ── 云层阴影（投射到地面） ──
        float shadow = 1.0 - uCloudShadow * 0.25 * lit;

        // 环境光（夜面也有微弱光）
        float ambient = 0.07 + uLow * 0.05;

        // 最终颜色合成
        vec3 finalCol = col * (ambient + lit * 0.93) * shadow;
        finalCol += oceanSpec;           // 海洋高光
        finalCol += sunset * isOcean * 0.6; // 黄昏海面反光
        finalCol += cityGlow;            // 城市灯光
        finalCol += auroraGlow;          // 极光
        finalCol += volcanoGlow;         // 火山

        // 低频发光（音乐律动，可感知的呼吸档）
        finalCol += col * uLow * 0.25;

        gl_FragColor = vec4(finalCol, 1.0);
      }
    `,
  }), [])

  // ─── 云层 shader ─── 三层云 + 阴影采样 + 更精细结构
  const cloudLayerMat = useMemo(() => new THREE.ShaderMaterial({
    transparent: true,
    depthWrite: false,
    uniforms: {
      uTime: { value: 0 },
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
      uniform float uTime;
      uniform float uLow;
      uniform vec3 uLightDir;
      varying vec3 vNormal;
      varying vec3 vPos;

      ${NOISE_UTILS}

      // 云的密度函数
      float cloudDensity(vec3 p) {
        // 主云系
        float c1 = fbm(p * 2.2 + vec3(uTime * 0.035, 0.0, 0.0), 4);
        // 碎云
        float c2 = fbm(p * 5.0 + vec3(-uTime * 0.07, uTime * 0.015, uTime * 0.02), 3) * 0.5;
        // 微结构
        float c3 = fbm(p * 12.0, 2) * 0.2;
        float c = c1 * 0.6 + c2 * 0.3 + c3 * 0.1;
        // 云带分布（沿纬度有疏密变化）
        float latBand = 0.7 + 0.3 * sin(p.y * 4.0 + 1.2);
        return c * latBand;
      }

      void main() {
        vec3 p = normalize(vPos);
        float clouds = cloudDensity(p);
        float alpha = smoothstep(0.48, 0.72, clouds);

        // 光照：模拟云的体积感
        float NdotL = dot(normalize(vNormal), uLightDir);
        float lit = smoothstep(-0.1, 0.35, NdotL);

        // 云的明暗过渡（边缘透光，底部暗）
        float cloudLight = 0.3 + lit * 0.7;
        // 边缘加亮（silver lining）
        float edge = smoothstep(0.48, 0.55, clouds) * (1.0 - smoothstep(0.65, 0.78, clouds));
        cloudLight += edge * lit * 0.35;

        vec3 cloudCol = vec3(0.92, 0.94, 1.0) * cloudLight;
        // 黄昏染色
        float terminator = smoothstep(-0.05, 0.15, NdotL) * (1.0 - smoothstep(0.15, 0.45, NdotL));
        cloudCol = mix(cloudCol, vec3(1.0, 0.55, 0.25), terminator * 0.5);
        // 极地偏冷色
        float polar = smoothstep(0.45, 0.85, abs(p.y));
        cloudCol = mix(cloudCol, vec3(0.72, 0.82, 1.0), polar * 0.35);
        // 夜面云暗化
        cloudCol *= 0.4 + lit * 0.6;

        gl_FragColor = vec4(cloudCol, alpha * (0.4 + uLow * 0.35));
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

  // ─── 行星环（多环带 + 卡西尼缝） ───
  const ringMat = useMemo(() => new THREE.ShaderMaterial({
    transparent: true,
    depthWrite: false,
    blending: THREE.AdditiveBlending,
    side: THREE.DoubleSide,
    uniforms: {
      uTime: { value: 0 },
      uLow: { value: 0 },
      uRms: { value: 0 },
      uLightDir: { value: new THREE.Vector3(0.85, 0.2, 0.5).normalize() },
    },
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
      uniform float uLow;
      uniform float uRms;
      uniform vec3 uLightDir;
      varying vec2 vUv;
      varying vec3 vPos;

      float hash(vec2 p) { return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
      float noise(vec2 p) {
        vec2 i = floor(p);
        vec2 f = fract(p);
        f = f * f * (3.0 - 2.0 * f);
        return mix(mix(hash(i), hash(i + vec2(1,0)), f.x),
                   mix(hash(i + vec2(0,1)), hash(i + vec2(1,1)), f.x), f.y);
      }
      float fbm2(vec2 p) {
        float v = 0.0;
        float a = 0.5;
        for (int i = 0; i < 5; i++) {
          v += a * noise(p);
          p = p * 2.1 + vec2(3.7, 1.9);
          a *= 0.5;
        }
        return v;
      }

      void main() {
        vec2 center = vUv - 0.5;
        float r = length(center) * 2.0;
        float angle = atan(center.y, center.x);

        // 三个主环带
        float ringA_inner = 0.52, ringA_outer = 0.68;  // 内亮环
        float cassini = 0.70;                          // 卡西尼缝
        float ringB_inner = 0.73, ringB_outer = 0.88;  // 外主环
        float ringC_inner = 0.90, ringC_outer = 0.98;  // 外暗环

        // 环带蒙版
        float ringA = smoothstep(ringA_inner - 0.02, ringA_inner, r) * smoothstep(ringA_outer + 0.02, ringA_outer, r);
        float ringB = smoothstep(ringB_inner - 0.02, ringB_inner, r) * smoothstep(ringB_outer + 0.02, ringB_outer, r);
        float ringC = smoothstep(ringC_inner - 0.01, ringC_inner, r) * smoothstep(ringC_outer + 0.01, ringC_outer, r);

        // 纹理：径向条纹 + 周向结构。
        // 角度必须周期化采样：atan 在 ±π 处跳变 2π，直接乘频率会让噪声
        // 在跳变处完全不连续，环面上留下一根贯穿的接缝亮线。
        vec2 ap = vec2(cos(angle), sin(angle));
        float radialN = fbm2(vec2(r * 40.0 + uTime * 0.4, (ap.x + ap.y) * 2.0));
        float angN = fbm2(ap * 6.0 + vec2(r * 3.0, -uTime * 0.25));
        float tex = radialN * 0.55 + angN * 0.45;

        // 每个环带的亮度分布
        float midA = (ringA_inner + ringA_outer) * 0.5;
        float thickA = ringA_outer - ringA_inner;
        float glowA = 1.0 - abs(r - midA) / thickA * 1.3;
        glowA = max(0.0, glowA);

        float midB = (ringB_inner + ringB_outer) * 0.5;
        float thickB = ringB_outer - ringB_inner;
        float glowB = 1.0 - abs(r - midB) / thickB * 1.1;
        glowB = max(0.0, glowB);

        float glowC = 1.0 - (r - ringC_inner) / (ringC_outer - ringC_inner);
        glowC = max(0.0, glowC) * 0.4;

        // 颜色：内圈偏紫粉，中间偏蓝紫，外圈偏冰蓝
        vec3 colA = vec3(0.9, 0.45, 0.95);
        vec3 colB = vec3(0.5, 0.55, 1.0);
        vec3 colC = vec3(0.6, 0.8, 1.0);

        // 环的光照（朝向光的一侧更亮）
        // 环位于 XZ 平面：方向向量与光源方向的水平分量都是 vec2
        // （旧代码 dot(vec3, uLightDir.xz) 类型不匹配，着色器编译失败）
        vec3 ringNormal = vec3(0.0, 1.0, 0.0);
        float lightAngle = dot(vec2(cos(angle), sin(angle)), uLightDir.xz);
        float ringLight = 0.6 + 0.4 * max(0.0, lightAngle);

        float alpha = 0.0;
        vec3 col = vec3(0.0);

        // A 环
        float aA = ringA * tex * glowA * 0.9;
        col += colA * aA;
        alpha += aA;

        // B 环（最亮）
        float aB = ringB * tex * glowB * 1.1;
        col += colB * aB;
        alpha += aB;

        // C 环（暗）
        float aC = ringC * tex * glowC * 0.5;
        col += colC * aC;
        alpha += aC;

        alpha *= ringLight;
        col *= ringLight;

        // 音乐响应（低频抬亮，可感知但不闪烁）
        alpha *= (0.5 + uLow * 0.35 + uRms * 0.15);

        gl_FragColor = vec4(col, alpha);
      }
    `,
  }), [])

  // ─── 星云粒子环 ───
  const nebulaGeo = useMemo(() => {
    const geo = new THREE.BufferGeometry()
    const count = q.nebulaN
    const positions = new Float32Array(count * 3)
    const sizes = new Float32Array(count)
    const colors = new Float32Array(count * 3)
    const speeds = new Float32Array(count)
    const radii = new Float32Array(count)
    const angles = new Float32Array(count)

    for (let i = 0; i < count; i++) {
      const r = 7.0 + Math.pow(Math.random(), 0.6) * 10.0
      const angle = Math.random() * Math.PI * 2
      const heightScale = (0.15 + Math.random() * 0.25) * (1 + (r - 7.0) * 0.05)

      positions[i * 3] = Math.cos(angle) * r
      positions[i * 3 + 1] = (Math.random() - 0.5) * heightScale
      positions[i * 3 + 2] = Math.sin(angle) * r

      sizes[i] = 0.08 + Math.random() * 0.22
      speeds[i] = 0.06 + Math.random() * 0.22
      radii[i] = r
      angles[i] = angle

      const hue = 0.82 - (r - 7.0) / 10.0 * 0.25
      const col = new THREE.Color().setHSL(hue, 0.65, 0.62)
      colors[i * 3] = col.r
      colors[i * 3 + 1] = col.g
      colors[i * 3 + 2] = col.b
    }

    geo.setAttribute('position', new THREE.BufferAttribute(positions, 3))
    geo.setAttribute('size', new THREE.BufferAttribute(sizes, 1))
    geo.setAttribute('color', new THREE.BufferAttribute(colors, 3))
    geo.setAttribute('speed', new THREE.BufferAttribute(speeds, 1))
    geo.setAttribute('radius', new THREE.BufferAttribute(radii, 1))
    geo.setAttribute('angle', new THREE.BufferAttribute(angles, 1))
    return geo
  }, [q.nebulaN])

  const nebulaMat = useMemo(() => new THREE.ShaderMaterial({
    transparent: true,
    depthWrite: false,
    blending: THREE.AdditiveBlending,
    uniforms: {
      uTime: { value: 0 },
      uIntensity: { value: 0.5 },
      uLow: { value: 0 },
      uPixelRatio: { value: Math.min(window.devicePixelRatio, 2) },
    },
    vertexShader: /* glsl */`
      attribute float size;
      attribute vec3 color;
      attribute float speed;
      attribute float radius;
      attribute float angle;
      uniform float uTime;
      uniform float uIntensity;
      uniform float uPixelRatio;
      varying vec3 vColor;
      varying float vAlpha;

      void main() {
        vColor = color;
        float angSpeed = speed / sqrt(radius) * 0.08;
        float currentAngle = angle + uTime * angSpeed;
        float wobble = sin(uTime * speed * 1.3 + angle * 3.0) * 0.08;
        vec3 pos = vec3(
          cos(currentAngle) * radius,
          position.y + wobble,
          sin(currentAngle) * radius
        );
        vec4 mvPosition = modelViewMatrix * vec4(pos, 1.0);
        gl_PointSize = size * uIntensity * 80.0 * uPixelRatio / -mvPosition.z;
        vAlpha = uIntensity * (0.28 + speed * 0.35);
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
        float alpha = smoothstep(0.5, 0.05, d) * vAlpha;
        gl_FragColor = vec4(vColor, alpha);
      }
    `,
  }), [])

  // ─── 小行星带 ───
  const asteroidGeo = useMemo(() => {
    const geo = new THREE.BufferGeometry()
    const count = q.asteroidN
    const positions = new Float32Array(count * 3)
    const sizes = new Float32Array(count)
    const colors = new Float32Array(count * 3)
    const speeds = new Float32Array(count)
    const radii = new Float32Array(count)
    const angles = new Float32Array(count)
    const rotations = new Float32Array(count)

    for (let i = 0; i < count; i++) {
      const r = 12.0 + Math.pow(Math.random(), 0.7) * 8.0
      const angle = Math.random() * Math.PI * 2
      const heightScale = 0.2 + Math.random() * 0.35

      positions[i * 3] = Math.cos(angle) * r
      positions[i * 3 + 1] = (Math.random() - 0.5) * heightScale
      positions[i * 3 + 2] = Math.sin(angle) * r

      sizes[i] = 0.03 + Math.random() * 0.08
      speeds[i] = 0.03 + Math.random() * 0.1
      radii[i] = r
      angles[i] = angle
      rotations[i] = Math.random() * Math.PI * 2

      // 岩石色
      const shade = 0.3 + Math.random() * 0.4
      const col = new THREE.Color().setHSL(0.08 + Math.random() * 0.05, 0.2, shade)
      colors[i * 3] = col.r
      colors[i * 3 + 1] = col.g
      colors[i * 3 + 2] = col.b
    }

    geo.setAttribute('position', new THREE.BufferAttribute(positions, 3))
    geo.setAttribute('size', new THREE.BufferAttribute(sizes, 1))
    geo.setAttribute('color', new THREE.BufferAttribute(colors, 3))
    geo.setAttribute('speed', new THREE.BufferAttribute(speeds, 1))
    geo.setAttribute('radius', new THREE.BufferAttribute(radii, 1))
    geo.setAttribute('angle', new THREE.BufferAttribute(angles, 1))
    geo.setAttribute('rotation', new THREE.BufferAttribute(rotations, 1))
    return geo
  }, [q.asteroidN])

  const asteroidMat = useMemo(() => new THREE.ShaderMaterial({
    transparent: false,
    uniforms: {
      uTime: { value: 0 },
      uPixelRatio: { value: Math.min(window.devicePixelRatio, 2) },
      uLightDir: { value: new THREE.Vector3(0.85, 0.2, 0.5).normalize() },
    },
    vertexShader: /* glsl */`
      attribute float size;
      attribute vec3 color;
      attribute float speed;
      attribute float radius;
      attribute float angle;
      attribute float rotation;
      uniform float uTime;
      uniform float uPixelRatio;
      uniform vec3 uLightDir;
      varying vec3 vColor;
      varying float vLit;

      void main() {
        vColor = color;
        float angSpeed = speed / sqrt(radius) * 0.06;
        float currentAngle = angle + uTime * angSpeed;
        float tilt = sin(uTime * speed * 0.8 + rotation) * 0.1;
        vec3 pos = vec3(
          cos(currentAngle) * radius,
          position.y + tilt,
          sin(currentAngle) * radius
        );
        // 简易光照
        vec3 normal = normalize(pos);
        vLit = 0.2 + 0.8 * max(0.0, dot(normal, uLightDir));
        vec4 mvPosition = modelViewMatrix * vec4(pos, 1.0);
        gl_PointSize = size * 30.0 * uPixelRatio / -mvPosition.z;
        gl_Position = projectionMatrix * mvPosition;
      }
    `,
    fragmentShader: /* glsl */`
      varying vec3 vColor;
      varying float vLit;
      void main() {
        vec2 uv = gl_PointCoord - 0.5;
        float d = length(uv);
        if (d > 0.5) discard;
        // 简单的球形明暗
        float sphere = 1.0 - d * 1.5;
        sphere = max(0.1, sphere);
        gl_FragColor = vec4(vColor * vLit * sphere, 1.0);
      }
    `,
  }), [])

  // ─── 远处弥漫星云 ───
  const cloudGeo = useMemo(() => {
    const geo = new THREE.BufferGeometry()
    const count = q.cloudN
    const positions = new Float32Array(count * 3)
    const sizes = new Float32Array(count)
    const colors = new Float32Array(count * 3)
    const speeds = new Float32Array(count)

    for (let i = 0; i < count; i++) {
      const r = 15 + Math.random() * 20
      const theta = Math.random() * Math.PI * 2
      const phi = Math.acos(2 * Math.random() - 1)
      positions[i * 3] = r * Math.sin(phi) * Math.cos(theta)
      positions[i * 3 + 1] = r * Math.sin(phi) * Math.sin(theta) * 0.5
      positions[i * 3 + 2] = r * Math.cos(phi)

      sizes[i] = 0.35 + Math.random() * 0.7
      speeds[i] = 0.015 + Math.random() * 0.06

      const hue = 0.6 + Math.random() * 0.3
      const col = new THREE.Color().setHSL(hue, 0.55, 0.55)
      colors[i * 3] = col.r
      colors[i * 3 + 1] = col.g
      colors[i * 3 + 2] = col.b
    }

    geo.setAttribute('position', new THREE.BufferAttribute(positions, 3))
    geo.setAttribute('size', new THREE.BufferAttribute(sizes, 1))
    geo.setAttribute('color', new THREE.BufferAttribute(colors, 3))
    geo.setAttribute('speed', new THREE.BufferAttribute(speeds, 1))
    return geo
  }, [q.cloudN])

  const cloudMat = useMemo(() => new THREE.ShaderMaterial({
    transparent: true,
    depthWrite: false,
    blending: THREE.AdditiveBlending,
    uniforms: {
      uTime: { value: 0 },
      uIntensity: { value: 0.35 },
      uPixelRatio: { value: Math.min(window.devicePixelRatio, 2) },
    },
    vertexShader: /* glsl */`
      attribute float size;
      attribute vec3 color;
      attribute float speed;
      uniform float uTime;
      uniform float uIntensity;
      uniform float uPixelRatio;
      varying vec3 vColor;
      varying float vAlpha;

      void main() {
        vColor = color;
        vec3 pos = position;
        pos.x += sin(uTime * speed + position.z * 0.4) * 0.3;
        pos.y += cos(uTime * speed * 0.6 + position.x * 0.25) * 0.2;
        vec4 mvPosition = modelViewMatrix * vec4(pos, 1.0);
        gl_PointSize = size * uIntensity * 32.0 * uPixelRatio / -mvPosition.z;
        vAlpha = uIntensity * (0.08 + speed * 0.9);
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
        float alpha = smoothstep(0.5, 0.0, d);
        alpha = pow(alpha, 1.6) * vAlpha;
        gl_FragColor = vec4(vColor, alpha);
      }
    `,
  }), [])

  // ─── 卫星 1（主卫星，岩石质地，陨石坑） ───
  const moonMat = useMemo(() => new THREE.ShaderMaterial({
    uniforms: {
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
      uniform vec3 uLightDir;
      varying vec3 vNormal;
      varying vec3 vPos;

      ${NOISE_UTILS}

      void main() {
        vec3 p = normalize(vPos);

        // 陨石坑：多层噪声形成圆形凹陷
        float craterField = fbm(p * 6.0, 5);
        float largeCraters = ridge(p * 3.0, 4) * 0.6;
        float smallCraters = fbm(p * 15.0, 4) * 0.3;
        float craters = largeCraters + smallCraters * smoothstep(0.4, 0.6, craterField);

        // 高地 vs 平原
        float highland = fbm(p * 2.0, 4);
        float mare = 1.0 - smoothstep(0.45, 0.55, highland);

        vec3 highlandCol = vec3(0.45, 0.42, 0.5);
        vec3 mareCol = vec3(0.28, 0.26, 0.34);
        vec3 col = mix(highlandCol, mareCol, mare * 0.7);

        // 陨石坑边缘亮、底部暗
        float craterEdge = smoothstep(0.5, 0.65, craters) - smoothstep(0.7, 0.85, craters);
        float craterFloor = smoothstep(0.65, 0.8, craters);
        col += craterEdge * 0.15;
        col -= craterFloor * 0.1;

        // 光照
        float NdotL = dot(normalize(vNormal), uLightDir);
        float lit = smoothstep(-0.2, 0.35, NdotL);

        // 黄昏色
        float terminator = smoothstep(-0.05, 0.15, NdotL) * (1.0 - smoothstep(0.15, 0.4, NdotL));
        col = mix(col, vec3(0.8, 0.4, 0.2), terminator * 0.3);

        vec3 finalCol = col * (0.12 + lit * 0.88);
        gl_FragColor = vec4(finalCol, 1.0);
      }
    `,
  }), [])

  // ─── 卫星 2（冰卫星，更小更远） ───
  const moon2Mat = useMemo(() => new THREE.ShaderMaterial({
    uniforms: {
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
      uniform vec3 uLightDir;
      varying vec3 vNormal;
      varying vec3 vPos;

      ${NOISE_UTILS}

      void main() {
        vec3 p = normalize(vPos);

        // 冰面纹理：裂缝 + 冰丘
        float ice = fbm(p * 5.0, 5);
        float cracks = ridge(p * 8.0, 4) * 0.5;
        float detail = fbm(p * 20.0, 4) * 0.2;
        float surf = ice + cracks * 0.4 + detail;

        vec3 iceWhite = vec3(0.85, 0.9, 0.98);
        vec3 iceBlue = vec3(0.5, 0.65, 0.85);
        vec3 deepIce = vec3(0.3, 0.4, 0.65);

        vec3 col = mix(iceBlue, iceWhite, smoothstep(0.4, 0.6, surf));
        col = mix(col, deepIce, smoothstep(0.7, 0.85, cracks));

        // 极地更亮
        float polar = smoothstep(0.5, 0.85, abs(p.y));
        col = mix(col, iceWhite * 0.95, polar * 0.5);

        // 光照
        float NdotL = dot(normalize(vNormal), uLightDir);
        float lit = smoothstep(-0.2, 0.35, NdotL);

        // 冰面高光
        vec3 viewDir = normalize(cameraPosition - vPos);
        vec3 halfVec = normalize(uLightDir + viewDir);
        float spec = pow(max(0.0, dot(normalize(vNormal), halfVec)), 32.0);
        col += vec3(0.9, 0.95, 1.0) * spec * lit * 0.3;

        vec3 finalCol = col * (0.15 + lit * 0.85);
        gl_FragColor = vec4(finalCol, 1.0);
      }
    `,
  }), [])

  // 质量预设切换会重建几何体：主动 dispose 旧的，避免 GPU 缓冲滞留
  useEffect(() => () => {
    nebulaGeo.dispose()
    asteroidGeo.dispose()
    cloudGeo.dispose()
  }, [nebulaGeo, asteroidGeo, cloudGeo])

  useFrame(({ clock, viewport }, delta) => {
    const d = audioRef.current
    const t = clock.elapsedTime
    // 记录启动时间
    if (startTimeRef.current === 0) startTimeRef.current = t
    const elapsed = t - startTimeRef.current
    // 启动淡入：音频响应前 ~1.1s 从 0 渐升到 1，避免进入瞬间闪烁
    // （delta 驱动，帧率变化不影响时长）
    initFadeRef.current = Math.min(1, initFadeRef.current + delta * 0.9)
    const fade = initFadeRef.current

    // 粒子尺寸跟随画布实际 dpr：governor 降档（画布 dpr 变小）时
    // gl_PointSize 若仍按设备 dpr 计，所有点会突然放大 25%
    const dpr = viewport.dpr || 1

    lowSmRef.current = lerp(lowSmRef.current, d.lowFreqAvg * fade, 0.08)
    rmsSmRef.current = lerp(rmsSmRef.current, d.rms * fade, 0.06)
    hiSmRef.current = lerp(hiSmRef.current, d.highFreqAvg * fade, 0.05)

    // beat 冷却：避免连续触发导致闪烁
    beatCooldownRef.current = Math.max(0, beatCooldownRef.current - 1)
    // 启动前 0.8 秒完全屏蔽 beat
    if (elapsed > 0.8 && d.beat && !lastBeatRef.current && beatCooldownRef.current <= 0) {
      beatPulseRef.current = 0.18 * fade
      beatCooldownRef.current = 8 // 至少 8 帧冷却（约 130ms @60fps）
    }
    beatPulseRef.current *= 0.68
    lastBeatRef.current = d.beat

    const low = lowSmRef.current
    const rms = rmsSmRef.current
    const pulse = beatPulseRef.current

    // 计算云阴影（极弱，避免行星表面闪烁）
    const cloudShadow = low * 0.03 + rms * 0.02

    // 行星自转
    if (planetRef.current) {
      planetRef.current.rotation.y = t * 0.04
      const scale = 1 + low * 0.015 + pulse * 0.01
      planetRef.current.scale.setScalar(scale)
      const mat = planetRef.current.material as THREE.ShaderMaterial
      mat.uniforms.uTime.value = t
      mat.uniforms.uLow.value = low
      mat.uniforms.uHigh.value = hiSmRef.current
      mat.uniforms.uCloudShadow.value = cloudShadow
    }

    // 云层
    if (cloudLayerRef.current) {
      cloudLayerRef.current.rotation.y = t * 0.055
      const mat = cloudLayerRef.current.material as THREE.ShaderMaterial
      mat.uniforms.uTime.value = t
      mat.uniforms.uLow.value = low
    }

    // 大气（合并壳）：半径取两壳中点，呼吸幅度加大到可感知档
    if (atmoRef.current) {
      atmoRef.current.rotation.y = -t * 0.025
      const scale = 1.065 + low * 0.012 + pulse * 0.009
      atmoRef.current.scale.setScalar(scale)
      const mat = atmoRef.current.material as THREE.ShaderMaterial
      mat.uniforms.uIntensity.value = 0.4 + low * 0.3 + rms * 0.1
      mat.uniforms.uLow.value = low
    }

    // 行星环
    if (planetRingRef.current) {
      planetRingRef.current.rotation.z = Math.sin(t * 0.08) * 0.04
      const mat = planetRingRef.current.material as THREE.ShaderMaterial
      mat.uniforms.uTime.value = t
      mat.uniforms.uLow.value = low
      mat.uniforms.uRms.value = rms
    }

    // 星云粒子环
    if (ringRef.current) {
      ringRef.current.rotation.x = 0.12 + Math.sin(t * 0.06) * 0.02
      ringRef.current.rotation.z = Math.sin(t * 0.04) * 0.012
      const mat = ringRef.current.material as THREE.ShaderMaterial
      mat.uniforms.uTime.value = t
      mat.uniforms.uIntensity.value = 0.35 + low * 0.3 + rms * 0.15 + pulse * 0.25
      mat.uniforms.uLow.value = low
      mat.uniforms.uPixelRatio.value = dpr
    }

    // 小行星带
    if (asteroidRef.current) {
      asteroidRef.current.rotation.x = -0.08
      asteroidRef.current.rotation.z = t * 0.005
      const mat = asteroidRef.current.material as THREE.ShaderMaterial
      mat.uniforms.uTime.value = t
      mat.uniforms.uPixelRatio.value = dpr
    }

    // 远处弥漫云（弱音频响应，避免全屏闪烁）
    if (cloudRef.current) {
      cloudRef.current.rotation.y = t * 0.008
      const mat = cloudRef.current.material as THREE.ShaderMaterial
      mat.uniforms.uTime.value = t
      mat.uniforms.uIntensity.value = 0.2 + hiSmRef.current * 0.12 + low * 0.08
      mat.uniforms.uPixelRatio.value = dpr
    }

    // 卫星1：绕行星公转（轨道 12.5 > 环外缘 11.7，避免每圈穿环）
    if (moonRef.current) {
      const moonDist = 12.5
      const moonSpeed = 0.28
      const moonAngle = t * moonSpeed
      const moonY = Math.sin(t * 0.18) * 1.0
      moonRef.current.position.set(
        Math.cos(moonAngle) * moonDist,
        moonY,
        Math.sin(moonAngle) * moonDist
      )
      moonRef.current.rotation.y = t * 0.4
    }

    // 卫星2（冰卫星）：更远，倾斜轨道（与卫星1拉开距离）
    if (moon2Ref.current) {
      const moon2Dist = 15.0
      const moon2Speed = 0.18
      const moon2Angle = t * moon2Speed + 1.5
      // 倾斜轨道
      const incline = 0.4
      const moon2Y = Math.sin(moon2Angle) * moon2Dist * Math.sin(incline)
      const moon2X = Math.cos(moon2Angle) * moon2Dist
      const moon2Z = Math.sin(moon2Angle) * moon2Dist * Math.cos(incline)
      moon2Ref.current.position.set(moon2X, moon2Y, moon2Z)
      moon2Ref.current.rotation.y = t * 0.3
    }
  })

  return (
    <group ref={groupRef} position={[0, -0.5, 0]}>
      {/* 远处弥漫星云（最底层） */}
      <points ref={cloudRef} geometry={cloudGeo}>
        <primitive object={cloudMat} attach="material" />
      </points>

      {/* 小行星带 */}
      <points ref={asteroidRef} geometry={asteroidGeo}>
        <primitive object={asteroidMat} attach="material" />
      </points>

      {/* 星云粒子环 */}
      <points ref={ringRef} geometry={nebulaGeo}>
        <primitive object={nebulaMat} attach="material" />
      </points>

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

      {/* 行星环（多环带）—— 倾角加大到 ~14°，配合抬高的机位呈现土星式椭圆 */}
      <mesh ref={planetRingRef} rotation={[0.24, 0, 0.05]}>
        <ringGeometry args={[PLANET_RADIUS * 1.3, PLANET_RADIUS * 2.6, q.ringSeg]} />
        <primitive object={ringMat} attach="material" />
      </mesh>

      {/* 大气（内外辉光合一） */}
      <mesh ref={atmoRef}>
        <sphereGeometry args={[PLANET_RADIUS, q.atmoSeg, q.atmoSeg]} />
        <primitive object={atmoMat} attach="material" />
      </mesh>

      {/* 卫星1（岩石卫星） */}
      <mesh ref={moonRef}>
        <sphereGeometry args={[0.65, Math.floor(q.cloudSeg * 0.5), Math.floor(q.cloudSeg * 0.5)]} />
        <primitive object={moonMat} attach="material" />
      </mesh>

      {/* 卫星2（冰卫星） */}
      <mesh ref={moon2Ref}>
        <sphereGeometry args={[0.35, Math.floor(q.cloudSeg * 0.4), Math.floor(q.cloudSeg * 0.4)]} />
        <primitive object={moon2Mat} attach="material" />
      </mesh>
    </group>
  )
}

function lerp(a: number, b: number, t: number) { return a + (b - a) * t }
