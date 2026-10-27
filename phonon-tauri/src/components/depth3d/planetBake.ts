// 行星地表/云层烘焙：把每帧 ~30 个噪声八度的静态部分一次性烘到纹理。
// 烘焙用全屏 quad（等距圆柱展开：uv → 球面方向 p），噪声代码与运行时
// 原版完全一致——烘焙结果与逐帧计算视觉等价，帧成本降为 1~2 次采样。
// 水平方向数学上无缝（lon ±π 处 p 连续）；极点收敛到一点。
import * as THREE from 'three'

export const NOISE_UTILS = /* glsl */`
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

const QUAD_VERT = /* glsl */`
  varying vec2 vUvB;
  void main() {
    vUvB = uv;
    gl_Position = vec4(uv * 2.0 - 1.0, 0.0, 1.0);
  }
`

// 地表烘焙：rgb = 反照率（地形配色 + 极地冰盖），a = 夜面发光强度（城市/火山）
const PLANET_BAKE_FRAG = /* glsl */`
  precision highp float;
  varying vec2 vUvB;
  ${NOISE_UTILS}
  void main() {
    float lon = (vUvB.x - 0.5) * 6.28318530718;
    float lat = (vUvB.y - 0.5) * 3.14159265359;
    vec3 p = vec3(cos(lat) * cos(lon), sin(lat), cos(lat) * sin(lon));

    // ── 地形高度（烘焙版：八度比逐帧版高一档，细节免费） ──
    float continents = fbm(p * 1.2, 5) * 0.9;
    float mountains = ridge(p * 3.0, 5) * 0.5;
    float hills = fbm(p * 6.0, 4) * 0.25;
    float microDetail = fbm(p * 18.0, 3) * 0.08;
    float height = continents
      + mountains * smoothstep(0.42, 0.58, continents)
      + hills * smoothstep(0.5, 0.65, continents)
      + microDetail;
    float coast = fbm(p * 8.0, 3) * 0.06;
    height += coast * smoothstep(0.44, 0.52, height);

    // ── 分层配色（与逐帧版逐行一致） ──
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

    float polar = smoothstep(0.55, 0.88, abs(p.y));
    float polarEdge = smoothstep(0.55, 0.7, abs(p.y)) - smoothstep(0.75, 0.88, abs(p.y));
    col = mix(col, snow * 0.82, polar * 0.6);
    col = mix(col, vec3(0.3, 0.35, 0.45), polarEdge * smoothstep(0.48, 0.55, height) * 0.4);

    // ── 夜面发光强度 → a 通道（城市/火山；动态夜面系数由运行时乘） ──
    float cityN = fbm(p * 22.0, 4);
    float cities = smoothstep(0.68, 0.74, cityN) * smoothstep(0.48, 0.58, height);
    float cityCenters = smoothstep(0.75, 0.82, cityN) * smoothstep(0.52, 0.62, height);
    float volcanoMask = smoothstep(0.72, 0.8, height) * ridge(p * 5.0, 4) * 0.5;
    float volcanoN = fbm(p * 15.0, 2);
    float volcanoes = smoothstep(0.65, 0.78, volcanoN) * volcanoMask;
    float emissive = cities * 0.22 + cityCenters * 0.15 + volcanoes * 0.4;

    gl_FragColor = vec4(col, clamp(emissive, 0.0, 1.0));
  }
`

// 云层烘焙：R = 云密度（原始值，smoothstep 由运行时做），G = 海洋掩码
const CLOUD_BAKE_FRAG = /* glsl */`
  precision highp float;
  varying vec2 vUvB;
  ${NOISE_UTILS}
  void main() {
    float lon = (vUvB.x - 0.5) * 6.28318530718;
    float lat = (vUvB.y - 0.5) * 3.14159265359;
    vec3 p = vec3(cos(lat) * cos(lon), sin(lat), cos(lat) * sin(lon));

    float continents = fbm(p * 1.2, 5) * 0.9;
    float mountains = ridge(p * 3.0, 5) * 0.5;
    float hills = fbm(p * 6.0, 4) * 0.25;
    float microDetail = fbm(p * 18.0, 3) * 0.08;
    float height = continents
      + mountains * smoothstep(0.42, 0.58, continents)
      + hills * smoothstep(0.5, 0.65, continents)
      + microDetail;

    float c1 = fbm(p * 2.2, 4);
    float c2 = fbm(p * 5.0, 3) * 0.5;
    float c3 = fbm(p * 12.0, 2) * 0.2;
    float c = c1 * 0.6 + c2 * 0.3 + c3 * 0.1;
    float latBand = 0.7 + 0.3 * sin(p.y * 4.0 + 1.2);
    float density = clamp(c * latBand, 0.0, 1.0);
    float oceanMask = 1.0 - smoothstep(0.44, 0.47, height);

    gl_FragColor = vec4(density, oceanMask, 0.0, 1.0);
  }
`

export interface PlanetBakedMaps {
  albedo: THREE.Texture
  cloud: THREE.Texture
  dispose: () => void
}

let cache: PlanetBakedMaps | null = null

/** 烘焙一次（每渲染器生命周期）；后续直接复用 */
export function bakePlanetMaps(gl: THREE.WebGLRenderer): PlanetBakedMaps {
  if (cache) return cache
  try {
  ;(window as any).__bakeInfo = { stage: 'start' }

  const makeRT = (w: number, h: number) => {
    const rt = new THREE.WebGLRenderTarget(w, h, {
      type: THREE.HalfFloatType,
      depthBuffer: false,
      generateMipmaps: true,
      minFilter: THREE.LinearMipmapLinearFilter,
      magFilter: THREE.LinearFilter,
      wrapS: THREE.RepeatWrapping,
      wrapT: THREE.ClampToEdgeWrapping,
    })
    return rt
  }

  const mkQuad = (frag: string) => {
    const mat = new THREE.ShaderMaterial({
      vertexShader: QUAD_VERT,
      fragmentShader: frag,
      depthTest: false,
      depthWrite: false,
    })
    const mesh = new THREE.Mesh(new THREE.PlaneGeometry(2, 2), mat)
    const scene = new THREE.Scene()
    scene.add(mesh)
    return { scene, cam: new THREE.Camera(), mat, mesh }
  }

  const prevRT = gl.getRenderTarget()

  // 1) 地表反照率 1024×512
  const albedoRT = makeRT(1024, 512)
  const planet = mkQuad(PLANET_BAKE_FRAG)
  gl.setRenderTarget(albedoRT)
  gl.render(planet.scene, planet.cam)

  // 2) 云密度 + 海洋掩码 512×256
  const cloudRT = makeRT(512, 256)
  const clouds = mkQuad(CLOUD_BAKE_FRAG)
  gl.setRenderTarget(cloudRT)
  gl.render(clouds.scene, clouds.cam)

  gl.setRenderTarget(prevRT)

  planet.mat.dispose()
  planet.mesh.geometry.dispose()
  clouds.mat.dispose()
  clouds.mesh.geometry.dispose()

  ;(window as any).__bakeInfo = { stage: 'done', w: albedoRT.width, h: albedoRT.height }
  ;(window as any).__bakeDebug = { albedoRT, cloudRT, gl }
  cache = {
    albedo: albedoRT.texture,
    cloud: cloudRT.texture,
    dispose: () => {
      albedoRT.dispose()
      cloudRT.dispose()
      cache = null
    },
  }
  return cache
  } catch (err: any) {
    ;(window as any).__bakeInfo = { stage: 'error', message: String(err && err.message || err) }
    throw err
  }
}

// 烘焙完成前的占位纹理（1×1）：中性反照率 + 空云
export const ALBEDO_PLACEHOLDER = (() => {
  const t = new THREE.DataTexture(new Uint8Array([120, 120, 130, 0]), 1, 1)
  t.needsUpdate = true
  return t
})()
export const CLOUD_PLACEHOLDER = (() => {
  const t = new THREE.DataTexture(new Uint8Array([0, 0, 0, 255]), 1, 1)
  t.needsUpdate = true
  return t
})()
