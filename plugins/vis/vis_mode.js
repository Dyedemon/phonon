/**
 * Phonon 视觉升华插件(自包含实体)
 *
 * 之前本文件只是占位:「提升质感」的全部效果寄宿在应用的 App.css 里。
 * 现在效果归位 —— 插件自带全部 CSS 与音频联动逻辑,对外契约:
 *
 *   styles        插件启用时由宿主注入 <style>:按钮/面板 UI + 质感模式
 *                 全部规则 + 氛围光斑层(见文件末尾区块)
 *   activate()    插件启用时调用:创建光斑层、挂音频事件监听
 *   deactivate()  插件禁用时调用:移除光斑层、清 CSS 变量与监听
 *   frame(now)    质感模式激活期间由宿主 rAF 驱动:读频段数据,
 *                 写 --vis-bass/--vis-mid/--vis-high/--vis-energy/--vis-beat,
 *                 驱动光斑呼吸与节拍脉冲
 *   init/render/resize/destroy
 *                 可视化页(2d 脚本)兼容接口,保留占位实现
 *
 * 音频数据来源(window 全局,由应用的 spectrumBus 镜像):
 *   window.__phononSpectrumBands   32 个对数频段 0..1(SpectrumData,~40Hz)
 *   'audio-features' CustomEvent   引擎 DSP 特征(beat / onset / rms 等)
 */

var STYLES = `
/* ═══════════════════════════════════════════════════
   Visualization Mode Button & Global Visual Schemes
   高质量实现 — 按钮 / 设置面板 / 提升质感 / 5维空间
   ═══════════════════════════════════════════════════ */

/* 旋转角度自定义属性 */
@property --vis-angle {
  syntax: '<angle>';
  initial-value: 0deg;
  inherits: false;
}

/* ─── 按钮本体（长椭圆形 pill） ─── */
.vis-mode-btn {
  position: relative;
  display: inline-flex;
  align-items: center;
  gap: 7px;
  height: 32px;
  padding: 0 16px 0 14px;
  border: none;
  border-radius: 999px;
  background:
    linear-gradient(135deg, rgba(6, 182, 212, 0.06), rgba(99, 102, 241, 0.06)),
    var(--bg-card);
  color: var(--text-bright);
  font-size: var(--text-sm);
  font-weight: 600;
  letter-spacing: 0.5px;
  cursor: pointer;
  overflow: hidden;
  isolation: isolate;
  flex-shrink: 0;
  transition:
    transform 0.45s cubic-bezier(0.34, 1.56, 0.64, 1),
    box-shadow 0.45s ease,
    color 0.35s ease,
    background 0.35s ease,
    filter 0.35s ease;
}

/* 内部径向光泽（顶部高光） */
.vis-mode-gloss {
  position: absolute;
  inset: 0;
  border-radius: 999px;
  background: linear-gradient(
    180deg,
    rgba(255, 255, 255, 0.08) 0%,
    transparent 50%
  );
  pointer-events: none;
  z-index: 1;
  transition: opacity 0.4s ease;
  opacity: 0.6;
}

/* 旋转锥形渐变描边 */
.vis-mode-btn::before {
  content: '';
  position: absolute;
  inset: 0;
  border-radius: 999px;
  padding: 1.5px;
  background: conic-gradient(
    from var(--vis-angle),
    var(--accent),
    var(--accent2),
    var(--accent2-hover),
    var(--accent-hover),
    var(--accent)
  );
  -webkit-mask: linear-gradient(#000 0 0) content-box, linear-gradient(#000 0 0);
  -webkit-mask-composite: xor;
          mask-composite: exclude;
  animation: vis-mode-spin 6s linear infinite;
  opacity: 0.45;
  transition: opacity 0.4s ease, animation-duration 0.4s ease;
  pointer-events: none;
  z-index: 2;
}

/* 流光扫过 */
.vis-mode-btn::after {
  content: '';
  position: absolute;
  top: 0; left: -130%;
  width: 65%; height: 100%;
  background: linear-gradient(
    90deg,
    transparent,
    rgba(255, 255, 255, 0.12) 40%,
    rgba(255, 255, 255, 0.22) 50%,
    rgba(255, 255, 255, 0.12) 60%,
    transparent
  );
  transform: skewX(-20deg);
  transition: left 0.8s cubic-bezier(0.22, 0.61, 0.36, 1);
  pointer-events: none;
  z-index: 1;
}

/* ─── 悬停态 ─── */
.vis-mode-btn.hovered,
.vis-mode-btn:hover {
  transform: scale(1.05) translateY(-1px);
  color: #fff;
  background:
    linear-gradient(135deg, rgba(6, 182, 212, 0.16), rgba(99, 102, 241, 0.16)),
    var(--bg-card);
  box-shadow:
    0 0 20px var(--accent-glow),
    0 0 40px rgba(99, 102, 241, 0.2),
    0 4px 12px rgba(0, 0, 0, 0.4),
    inset 0 1px 0 rgba(255, 255, 255, 0.08);
  filter: brightness(1.08);
}
.vis-mode-btn.hovered::before,
.vis-mode-btn:hover::before { opacity: 0.9; animation-duration: 2s; }
.vis-mode-btn.hovered::after,
.vis-mode-btn:hover::after { left: 140%; }
.vis-mode-btn.hovered .vis-mode-gloss,
.vis-mode-btn:hover .vis-mode-gloss { opacity: 1; }

/* ─── 激活态 ─── */
.vis-mode-btn.active {
  color: #fff;
  background:
    linear-gradient(135deg, rgba(6, 182, 212, 0.22), rgba(99, 102, 241, 0.22)),
    var(--bg-card);
  box-shadow:
    0 0 16px var(--accent-glow),
    0 0 32px rgba(99, 102, 241, 0.25),
    inset 0 0 14px rgba(99, 102, 241, 0.12),
    inset 0 1px 0 rgba(255, 255, 255, 0.06);
  animation: vis-mode-breathe 4s ease-in-out infinite;
}
.vis-mode-btn.active::before { opacity: 0.85; animation-duration: 4s; }
.vis-mode-btn.active .vis-mode-gloss { opacity: 0.8; }

/* 呼吸效果 */
@keyframes vis-mode-breathe {
  0%, 100% {
    box-shadow:
      0 0 12px var(--accent-glow),
      0 0 24px rgba(99, 102, 241, 0.18),
      inset 0 0 10px rgba(99, 102, 241, 0.08),
      inset 0 1px 0 rgba(255, 255, 255, 0.06);
  }
  50% {
    box-shadow:
      0 0 28px var(--accent-glow),
      0 0 56px rgba(99, 102, 241, 0.42),
      inset 0 0 24px rgba(99, 102, 241, 0.22),
      inset 0 1px 0 rgba(255, 255, 255, 0.1);
  }
}

/* ─── 漂浮粒子 ─── */
.vis-mode-particles {
  position: absolute; inset: 0;
  border-radius: 999px;
  pointer-events: none;
  z-index: 1;
  overflow: hidden;
  opacity: 0;
  transition: opacity 0.6s ease;
}
.vis-mode-btn.active .vis-mode-particles { opacity: 1; }
.vis-mode-particles > span {
  position: absolute;
  width: 2.5px; height: 2.5px;
  border-radius: 50%;
  background: radial-gradient(
    circle,
    rgba(255, 255, 255, 0.95) 0%,
    rgba(120, 180, 255, 0.5) 40%,
    transparent 75%
  );
  box-shadow: 0 0 4px rgba(120, 180, 255, 0.4);
  animation: vis-mode-float 4.5s ease-in-out infinite;
  left: calc((var(--i) / var(--n)) * 100%);
  bottom: -15%;
  animation-delay: calc(var(--i) * 0.35s);
}
.vis-mode-particles > span:nth-child(odd) { animation-duration: 5.5s; }
.vis-mode-particles > span:nth-child(3n) { animation-duration: 3.8s; }
.vis-mode-particles > span:nth-child(4n) { animation-duration: 6.2s; }
@keyframes vis-mode-float {
  0%   { transform: translateY(0) scale(0.5); opacity: 0; }
  15%  { opacity: 0.9; }
  85%  { opacity: 0.7; }
  100% { transform: translateY(-44px) scale(1.2); opacity: 0; }
}

/* ─── 均衡器指示器（5根光柱） ─── */
.vis-mode-eq {
  display: inline-flex;
  align-items: flex-end;
  gap: 2px;
  height: 14px;
  position: relative; z-index: 2;
}
.vis-mode-eq > span {
  display: block;
  width: 2.5px;
  border-radius: 2px;
  background: linear-gradient(180deg, var(--accent2), var(--accent));
  transform-origin: bottom center;
  transform: scaleY(0.2);
  transition: transform 0.25s ease, opacity 0.3s ease;
  opacity: 0.6;
}
.vis-mode-eq.on > span {
  animation: vis-mode-eq 1.1s ease-in-out infinite;
  opacity: 1;
}
.vis-mode-eq > span:nth-child(1) { height: 50%; animation-delay: 0s; }
.vis-mode-eq > span:nth-child(2) { height: 85%; animation-delay: 0.15s; }
.vis-mode-eq > span:nth-child(3) { height: 40%; animation-delay: 0.3s; }
.vis-mode-eq > span:nth-child(4) { height: 70%; animation-delay: 0.45s; }
.vis-mode-eq > span:nth-child(5) { height: 60%; animation-delay: 0.6s; }

.vis-mode-btn.hovered .vis-mode-eq.on > span,
.vis-mode-btn:hover .vis-mode-eq.on > span,
.vis-mode-btn.active .vis-mode-eq.on > span {
  animation-duration: 0.5s;
}

.vis-mode-label {
  position: relative; z-index: 2;
  white-space: nowrap;
  text-shadow: 0 0 8px rgba(0, 0, 0, 0.3);
}

/* 齿轮图标 */
.vis-mode-gear {
  position: relative; z-index: 2;
  margin-left: 2px;
  transition: transform 0.6s cubic-bezier(0.34, 1.56, 0.64, 1), opacity 0.3s ease;
  opacity: 0.8;
  filter: drop-shadow(0 0 3px var(--accent-glow));
}
.vis-mode-btn.active.hovered .vis-mode-gear,
.vis-mode-btn.active:hover .vis-mode-gear {
  transform: rotate(180deg);
  opacity: 1;
}

@keyframes vis-mode-spin {
  to { --vis-angle: 360deg; }
}
@keyframes vis-mode-eq {
  0%, 100% { transform: scaleY(0.2); }
  50%      { transform: scaleY(1); }
}

/* ═══════════════════════════════════════════════════
   设置面板
   ═══════════════════════════════════════════════════ */
.vis-mode-panel {
  position: absolute;
  top: calc(100% + 10px);
  right: 0;
  width: 280px;
  max-height: calc(100vh - 80px);
  overflow-y: auto;
  overflow-x: hidden;
  scrollbar-gutter: stable;
  padding: 0;
  border-radius: 14px;
  background: rgba(12, 14, 26, 0.92);
  backdrop-filter: blur(24px) saturate(180%);
  -webkit-backdrop-filter: blur(24px) saturate(180%);
  border: 1px solid rgba(120, 150, 255, 0.2);
  box-shadow:
    0 24px 60px rgba(0, 0, 0, 0.6),
    0 8px 24px rgba(0, 0, 0, 0.4),
    0 0 40px rgba(99, 102, 241, 0.15),
    inset 0 1px 0 rgba(255, 255, 255, 0.06);
  z-index: 10000;
  animation: vis-mode-panel-in 0.28s cubic-bezier(0.22, 1, 0.36, 1);
  transform-origin: top right;
  overflow: hidden;
}

/* 面板顶部辉光 */
.vis-mode-panel-glow {
  position: absolute;
  top: -50%; left: 0; right: 0;
  height: 100%;
  background: radial-gradient(
    ellipse at center top,
    rgba(99, 102, 241, 0.12),
    transparent 70%
  );
  pointer-events: none;
  z-index: 0;
}

@keyframes vis-mode-panel-in {
  from { opacity: 0; transform: translateY(-8px) scale(0.94); }
  to   { opacity: 1; transform: translateY(0) scale(1); }
}

.vis-mode-panel-title {
  position: relative; z-index: 1;
  padding: 12px 14px 10px;
  border-bottom: 1px solid rgba(120, 150, 255, 0.1);
}
.vis-mode-panel-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
}
.vis-mode-panel-head-text {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}
.vis-mode-panel-name {
  font-size: var(--text-sm);
  font-weight: 700;
  color: rgba(220, 230, 255, 0.95);
  letter-spacing: 0.8px;
}
.vis-mode-panel-sub {
  font-size: var(--text-xs);
  color: rgba(160, 180, 220, 0.5);
  letter-spacing: 0.3px;
  display: none;
}

/* 开启/关闭开关（iOS 风格 toggle） */
.vis-mode-toggle {
  position: relative;
  flex-shrink: 0;
  width: 44px;
  height: 24px;
  border-radius: 999px;
  border: 1px solid rgba(120, 150, 255, 0.25);
  background: rgba(255, 255, 255, 0.04);
  cursor: pointer;
  padding: 0;
  transition: background 0.3s ease, border-color 0.3s ease, box-shadow 0.3s ease;
}
.vis-mode-toggle.on {
  background: linear-gradient(135deg, var(--accent), var(--accent2));
  border-color: transparent;
  box-shadow: 0 0 12px var(--accent-glow), inset 0 1px 0 rgba(255, 255, 255, 0.2);
}
.vis-mode-toggle-knob {
  position: absolute;
  top: 50%;
  left: 2px;
  width: 18px;
  height: 18px;
  border-radius: 50%;
  background: #fff;
  box-shadow: 0 1px 4px rgba(0, 0, 0, 0.3);
  transform: translateY(-50%);
  transition: left 0.3s cubic-bezier(0.34, 1.56, 0.64, 1);
}
.vis-mode-toggle.on .vis-mode-toggle-knob {
  left: 22px;
}

.vis-mode-panel-list {
  position: relative; z-index: 1;
  padding: 6px;
}

.vis-mode-opt {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  padding: 9px 10px;
  margin: 2px 0;
  border: 1px solid transparent;
  border-radius: 10px;
  background: transparent;
  color: var(--text);
  cursor: pointer;
  text-align: left;
  transition:
    background 0.25s ease,
    border-color 0.25s ease,
    transform 0.25s cubic-bezier(0.22, 1, 0.36, 1);
  position: relative;
  overflow: hidden;
}
.vis-mode-opt::before {
  content: '';
  position: absolute;
  inset: 0;
  background: linear-gradient(90deg, transparent, rgba(99, 102, 241, 0.06), transparent);
  transform: translateX(-100%);
  transition: transform 0.5s ease;
}
.vis-mode-opt:hover {
  background: rgba(99, 102, 241, 0.08);
  transform: translateX(3px);
  border-color: rgba(99, 102, 241, 0.15);
}
.vis-mode-opt:hover::before {
  transform: translateX(100%);
}
.vis-mode-opt.sel {
  background: rgba(99, 102, 241, 0.12);
  border-color: rgba(99, 102, 241, 0.35);
  box-shadow:
    inset 0 0 12px rgba(99, 102, 241, 0.08),
    0 0 12px rgba(99, 102, 241, 0.1);
}

/* 单选圆点 */
.vis-mode-opt-radio {
  flex-shrink: 0;
  width: 18px;
  height: 18px;
  border-radius: 50%;
  border: 2px solid rgba(160, 180, 220, 0.35);
  position: relative;
  transition: border-color 0.25s ease;
}
.vis-mode-opt.sel .vis-mode-opt-radio {
  border-color: var(--accent);
}
.vis-mode-opt-radio::after {
  content: '';
  position: absolute;
  inset: 3px;
  border-radius: 50%;
  background: var(--accent);
  transform: scale(0);
  transition: transform 0.3s cubic-bezier(0.34, 1.56, 0.64, 1);
  box-shadow: 0 0 10px var(--accent-glow);
}
.vis-mode-opt.sel .vis-mode-opt-radio::after {
  transform: scale(1);
}

.vis-mode-opt-body {
  display: flex;
  flex-direction: column;
  gap: 0;
  min-width: 0;
  flex: 1;
}
.vis-mode-opt-label {
  font-size: var(--text-sm);
  font-weight: 600;
  color: var(--text-bright);
  letter-spacing: 0.2px;
}
.vis-mode-opt-desc {
  font-size: var(--text-xs);
  color: rgba(160, 180, 220, 0.5);
  line-height: 1.45;
  letter-spacing: 0.1px;
  display: none;
}
.vis-mode-opt.sel .vis-mode-opt-label {
  color: #fff;
}
.vis-mode-opt.sel .vis-mode-opt-desc {
  color: rgba(180, 200, 255, 0.65);
}

/* 选项图标 */
.vis-mode-opt-icon {
  flex-shrink: 0;
  opacity: 0.35;
  transition: opacity 0.25s ease, filter 0.25s ease;
  color: var(--text-dim);
}
.vis-mode-opt:hover .vis-mode-opt-icon {
  opacity: 0.6;
}
.vis-mode-opt.sel .vis-mode-opt-icon {
  opacity: 1;
  color: var(--accent);
  filter: drop-shadow(0 0 6px var(--accent-glow));
}

/* ═══════════════════════════════════════════════════
   全局视觉方案 A：提升质感（Quality）
   玻璃拟态 · 景深阴影 · 精致边框 · 光斑 · 噪点
   ═══════════════════════════════════════════════════ */
html[data-vis-active='true'][data-vis-mode='quality'],
html[data-vis-active='true'][data-vis-mode='quality'] body,
html[data-vis-active='true'][data-vis-mode='quality'] #root {
  /* 静态渐变光斑已迁移到插件自有的 .vis-ambient-layer(随主题色/音频呼吸) */
  background: var(--bg);
}

/* 噪点纹理叠加层（提升真实感） */
html[data-vis-active='true'][data-vis-mode='quality'] #root::before {
  content: '';
  position: fixed;
  inset: 0;
  pointer-events: none;
  z-index: 0;
  opacity: 0.048;
  background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='200' height='200'%3E%3Cfilter id='n'%3E%3CfeTurbulence type='fractalNoise' baseFrequency='0.9' numOctaves='3'/%3E%3C/filter%3E%3Crect width='200' height='200' filter='url(%23n)'/%3E%3C/svg%3E");
}

html[data-vis-active='true'][data-vis-mode='quality'] .card {
  background:
    linear-gradient(135deg, rgba(255, 255, 255, 0.08), rgba(255, 255, 255, 0.015)),
    color-mix(in srgb, var(--bg-card) 28%, transparent);
  border: 1px solid rgba(180, 210, 255, 0.22);
  backdrop-filter: blur(10px) brightness(1.12) saturate(140%);
  -webkit-backdrop-filter: blur(10px) brightness(1.12) saturate(140%);
  box-shadow:
    0 6px 18px rgba(0, 0, 0, 0.22),
    0 2px 6px rgba(0, 0, 0, 0.12),
    0 0 0 1px rgba(255, 255, 255, 0.03),
    inset 0 1px 0 rgba(255, 255, 255, 0.18),
    inset 0 -1px 0 rgba(0, 0, 0, 0.12),
    inset 0 0 40px rgba(99, 102, 241, 0.04);
}
html[data-vis-active='true'][data-vis-mode='quality'] .tab {
  background: transparent;
  border: none;
  border-radius: 9px 9px 0 0;
  margin: 0 2px;
  transition: color 0.2s ease, background 0.2s ease, text-shadow 0.2s ease;
}
html[data-vis-active='true'][data-vis-mode='quality'] .tab:hover {
  background: rgba(255, 255, 255, 0.06);
}
html[data-vis-active='true'][data-vis-mode='quality'] .tab.active {
  background: linear-gradient(180deg, rgba(99, 102, 241, 0.2), transparent 75%);
  box-shadow:
    inset 0 -2px 0 var(--accent),
    0 -4px 20px rgba(99, 102, 241, 0.12);
  text-shadow: 0 0 10px var(--accent-glow);
}
html[data-vis-active='true'][data-vis-mode='quality'] .btn {
  border-radius: 10px;
  transition: all 0.25s ease;
}
html[data-vis-active='true'][data-vis-mode='quality'] .btn-primary {
  background:
    linear-gradient(135deg,
      color-mix(in srgb, var(--accent) 90%, #fff),
      color-mix(in srgb, var(--accent2) 90%, #fff));
  border: 1px solid rgba(255, 255, 255, 0.2);
  box-shadow:
    0 3px 14px var(--accent-glow),
    0 2px 6px rgba(0, 0, 0, 0.2),
    inset 0 1px 0 rgba(255, 255, 255, 0.28),
    inset 0 -1px 0 rgba(0, 0, 0, 0.14),
    inset 0 0 20px rgba(255, 255, 255, 0.04);
}
html[data-vis-active='true'][data-vis-mode='quality'] .btn-primary:hover {
  filter: brightness(1.06);
  box-shadow:
    0 5px 20px var(--accent-glow),
    0 3px 10px rgba(0, 0, 0, 0.24),
    inset 0 1px 0 rgba(255, 255, 255, 0.32),
    inset 0 -1px 0 rgba(0, 0, 0, 0.14),
    0 0 0 1px rgba(255, 255, 255, 0.05);
}
html[data-vis-active='true'][data-vis-mode='quality'] .list-item {
  border-radius: 12px;
  margin: 5px 0;
  background: rgba(255, 255, 255, 0.03);
  border: 1px solid rgba(180, 210, 255, 0.14) !important;
  box-shadow:
    0 2px 6px rgba(0, 0, 0, 0.14),
    0 1px 2px rgba(0, 0, 0, 0.08),
    inset 0 1px 0 rgba(255, 255, 255, 0.06);
  transition: all 0.22s ease;
}
html[data-vis-active='true'][data-vis-mode='quality'] .list-item:hover {
  background: rgba(255, 255, 255, 0.06);
  border-color: rgba(180, 210, 255, 0.26) !important;
  transform: translateY(-1px);
  box-shadow:
    0 4px 12px rgba(0, 0, 0, 0.18),
    0 1px 3px rgba(0, 0, 0, 0.1),
    inset 0 1px 0 rgba(255, 255, 255, 0.08),
    0 0 0 1px rgba(255, 255, 255, 0.02);
}
html[data-vis-active='true'][data-vis-mode='quality'] .titlebar {
  background: color-mix(in srgb, var(--bg-card) 30%, transparent);
  backdrop-filter: blur(12px) brightness(1.1) saturate(135%);
  -webkit-backdrop-filter: blur(12px) brightness(1.1) saturate(135%);
  border-bottom: 1px solid rgba(180, 210, 255, 0.16);
  box-shadow: 0 1px 6px rgba(0, 0, 0, 0.1);
}
html[data-vis-active='true'][data-vis-mode='quality'] .toolbar {
  background: rgba(255, 255, 255, 0.03);
  border-radius: 12px;
  border: 1px solid rgba(180, 210, 255, 0.12);
  box-shadow: 0 1px 5px rgba(0, 0, 0, 0.12), inset 0 1px 0 rgba(255, 255, 255, 0.06);
  padding: 10px 16px !important;
  gap: 16px !important;
}
/* quality 模式下：toolbar 内的刷新/操作按钮与左侧元素拉开距离，避免视觉拥挤 */
html[data-vis-active='true'][data-vis-mode='quality'] .toolbar > .btn:last-child,
html[data-vis-active='true'][data-vis-mode='quality'] .toolbar > button:last-child {
  margin-left: 4px;
}

/* ─────────────── quality 模式：扩大质感覆盖面 ─────────────── */
html[data-vis-active='true'][data-vis-mode='quality'] .tabs {
  background:
    linear-gradient(180deg, rgba(255, 255, 255, 0.02), transparent 55%),
    color-mix(in srgb, var(--bg-card) 28%, transparent);
  backdrop-filter: blur(10px) brightness(1.08) saturate(130%);
  -webkit-backdrop-filter: blur(10px) brightness(1.08) saturate(130%);
  border-bottom: 1px solid rgba(180, 210, 255, 0.16);
  box-shadow: 0 1px 5px rgba(0, 0, 0, 0.08);
}
html[data-vis-active='true'][data-vis-mode='quality'] .content {
  background: transparent;
}
/* 通用输入框 */
html[data-vis-active='true'][data-vis-mode='quality'] input[type='text'],
html[data-vis-active='true'][data-vis-mode='quality'] input[type='number'],
html[data-vis-active='true'][data-vis-mode='quality'] input[type='search'],
html[data-vis-active='true'][data-vis-mode='quality'] input[type='password'],
html[data-vis-active='true'][data-vis-mode='quality'] input[type='url'],
html[data-vis-active='true'][data-vis-mode='quality'] input[type='file'],
html[data-vis-active='true'][data-vis-mode='quality'] textarea,
html[data-vis-active='true'][data-vis-mode='quality'] select {
  border-radius: 10px;
  background: rgba(255, 255, 255, 0.05);
  border: 1px solid rgba(180, 210, 255, 0.18);
  box-shadow:
    inset 0 1px 0 rgba(255, 255, 255, 0.07),
    0 3px 10px rgba(0, 0, 0, 0.22),
    inset 0 0 0 1px rgba(255, 255, 255, 0.02);
  transition: all 0.22s cubic-bezier(0.25, 0.8, 0.25, 1);
}
/* outline 按钮（.btn-outline / 次要按钮） */
html[data-vis-active='true'][data-vis-mode='quality'] .btn-outline,
html[data-vis-active='true'][data-vis-mode='quality'] .btn-secondary {
  border-radius: 10px;
  border: 1px solid rgba(180, 210, 255, 0.26);
  background: rgba(255, 255, 255, 0.04);
  box-shadow:
    inset 0 1px 0 rgba(255, 255, 255, 0.08),
    0 2px 8px rgba(0, 0, 0, 0.18);
  transition: all 0.22s cubic-bezier(0.25, 0.8, 0.25, 1);
}
/* 滚动条（轻微润色） */
html[data-vis-active='true'][data-vis-mode='quality'] * {
  scrollbar-width: thin;
  scrollbar-color: rgba(120, 160, 255, 0.32) transparent;
}
html[data-vis-active='true'][data-vis-mode='quality'] ::-webkit-scrollbar {
  width: 10px;
  height: 10px;
}
html[data-vis-active='true'][data-vis-mode='quality'] ::-webkit-scrollbar-thumb {
  background: linear-gradient(180deg,
    rgba(120, 160, 255, 0.32),
    rgba(120, 160, 255, 0.2));
  border-radius: 999px;
  border: 2px solid transparent;
  background-clip: padding-box;
  box-shadow: inset 0 0 6px rgba(120, 160, 255, 0.1);
}
html[data-vis-active='true'][data-vis-mode='quality'] ::-webkit-scrollbar-thumb:hover {
  background: linear-gradient(180deg,
    rgba(120, 160, 255, 0.5),
    rgba(120, 160, 255, 0.32));
  background-clip: padding-box;
  border: 2px solid transparent;
  box-shadow: inset 0 0 10px rgba(120, 160, 255, 0.2);
}
html[data-vis-active='true'][data-vis-mode='quality'] .title,
html[data-vis-active='true'][data-vis-mode='quality'] h1,
html[data-vis-active='true'][data-vis-mode='quality'] h2,
html[data-vis-active='true'][data-vis-mode='quality'] h3 {
  color: var(--text-bright);
  text-shadow:
    0 1px 0 rgba(255, 255, 255, 0.04),
    0 0 20px rgba(99, 102, 241, 0.08);
  letter-spacing: 0.1px;
}
html[data-vis-active='true'][data-vis-mode='quality'] .subtitle,
html[data-vis-active='true'][data-vis-mode='quality'] .text-dim {
  color: color-mix(in srgb, var(--text-dim) 92%, transparent);
}
html[data-vis-active='true'][data-vis-mode='quality'] .divider,
html[data-vis-active='true'][data-vis-mode='quality'] hr {
  border-color: rgba(180, 210, 255, 0.14);
}
html[data-vis-active='true'][data-vis-mode='quality'] .empty-state {
  border-radius: 16px;
  background: rgba(255, 255, 255, 0.035);
  border: 1px dashed rgba(180, 210, 255, 0.2);
  box-shadow: 0 1px 6px rgba(0, 0, 0, 0.12);
}
/* 通用模态 / 面板类（常见扩展样式） */
html[data-vis-active='true'][data-vis-mode='quality'] .modal,
html[data-vis-active='true'][data-vis-mode='quality'] .dialog,
html[data-vis-active='true'][data-vis-mode='quality'] .popover,
html[data-vis-active='true'][data-vis-mode='quality'] .sheet {
  border-radius: 18px;
  background:
    linear-gradient(135deg, rgba(255, 255, 255, 0.06), rgba(255, 255, 255, 0.01)),
    color-mix(in srgb, var(--bg-card) 25%, transparent);
  border: 1px solid rgba(180, 210, 255, 0.24);
  backdrop-filter: blur(14px) brightness(1.1) saturate(135%);
  -webkit-backdrop-filter: blur(14px) brightness(1.1) saturate(135%);
  box-shadow:
    0 10px 30px rgba(0, 0, 0, 0.3),
    0 3px 10px rgba(0, 0, 0, 0.16),
    0 0 0 1px rgba(255, 255, 255, 0.03),
    inset 0 1px 0 rgba(255, 255, 255, 0.14),
    inset 0 -1px 0 rgba(0, 0, 0, 0.1),
    inset 0 0 60px rgba(99, 102, 241, 0.05);
}
/* 切换开关 / Slider / 徽章（类名扩展） */
html[data-vis-active='true'][data-vis-mode='quality'] .switch,
html[data-vis-active='true'][data-vis-mode='quality'] .toggle {
  background: rgba(255, 255, 255, 0.04);
}
html[data-vis-active='true'][data-vis-mode='quality'] .badge,
html[data-vis-active='true'][data-vis-mode='quality'] .chip {
  border-radius: 999px;
  background: rgba(120, 160, 255, 0.14);
  border: 1px solid rgba(120, 160, 255, 0.28);
  box-shadow:
    inset 0 1px 0 rgba(255, 255, 255, 0.08),
    0 1px 4px rgba(0, 0, 0, 0.16);
}

/* ─────────────── quality 模式：全面提升交互反馈 ─────────────── */
/* btn-primary：按下收缩 */
html[data-vis-active='true'][data-vis-mode='quality'] .btn-primary:active {
  transform: scale(0.98);
  filter: brightness(0.98);
}
/* 次要按钮（outline/secondary）：悬停亮边 + 辉光 */
html[data-vis-active='true'][data-vis-mode='quality'] .btn-outline:hover,
html[data-vis-active='true'][data-vis-mode='quality'] .btn-secondary:hover {
  background: rgba(120, 160, 255, 0.14);
  border-color: rgba(120, 160, 255, 0.48);
  box-shadow:
    0 0 14px rgba(99, 102, 241, 0.14),
    inset 0 1px 0 rgba(255, 255, 255, 0.12),
    0 2px 6px rgba(0, 0, 0, 0.14);
}
html[data-vis-active='true'][data-vis-mode='quality'] .btn-outline:active,
html[data-vis-active='true'][data-vis-mode='quality'] .btn-secondary:active {
  transform: scale(0.985);
}
/* 普通 .btn：按下收缩 */
html[data-vis-active='true'][data-vis-mode='quality'] .btn:active {
  transform: scale(0.985);
}
/* list-item：悬停已在上方加了 translateY(-1px)，这里补 active 回弹 */
html[data-vis-active='true'][data-vis-mode='quality'] .list-item:active {
  transform: translateY(0) scale(0.996);
}
/* 输入框：聚焦光晕 */
html[data-vis-active='true'][data-vis-mode='quality'] input:focus,
html[data-vis-active='true'][data-vis-mode='quality'] textarea:focus,
html[data-vis-active='true'][data-vis-mode='quality'] select:focus {
  outline: none;
  border-color: color-mix(in srgb, var(--accent) 80%, transparent);
  box-shadow:
    0 0 0 3px color-mix(in srgb, var(--accent) 24%, transparent),
    inset 0 1px 0 rgba(255, 255, 255, 0.08),
    0 2px 6px rgba(0, 0, 0, 0.14),
    0 0 12px color-mix(in srgb, var(--accent) 10%, transparent);
  background: rgba(255, 255, 255, 0.07);
}
/* 卡片：hover 微浮动（用于可交互的卡片块） */
html[data-vis-active='true'][data-vis-mode='quality'] .card-interactive:hover,
html[data-vis-active='true'][data-vis-mode='quality'] .card.clickable:hover,
html[data-vis-active='true'][data-vis-mode='quality'] .card[role='button']:hover {
  transform: translateY(-2px);
  border-color: rgba(180, 210, 255, 0.32);
  box-shadow:
    0 8px 22px rgba(0, 0, 0, 0.22),
    0 3px 8px rgba(0, 0, 0, 0.14),
    inset 0 1px 0 rgba(255, 255, 255, 0.18),
    inset 0 -1px 0 rgba(0, 0, 0, 0.1),
    0 0 0 1px rgba(255, 255, 255, 0.04),
    0 0 16px rgba(99, 102, 241, 0.06);
}
/* 链接 / 文本按钮：悬停文字辉光 */
html[data-vis-active='true'][data-vis-mode='quality'] a,
html[data-vis-active='true'][data-vis-mode='quality'] .link,
html[data-vis-active='true'][data-vis-mode='quality'] .text-link {
  transition: color 0.22s ease, text-shadow 0.22s ease;
}
html[data-vis-active='true'][data-vis-mode='quality'] a:hover,
html[data-vis-active='true'][data-vis-mode='quality'] .link:hover,
html[data-vis-active='true'][data-vis-mode='quality'] .text-link:hover {
  text-shadow: 0 0 10px color-mix(in srgb, var(--accent) 35%, transparent);
}

/* focus-visible 键盘焦点环（全局统一高质量） */
html[data-vis-active='true'][data-vis-mode='quality'] *:focus-visible {
  outline: none;
  box-shadow:
    0 0 0 2px var(--bg),
    0 0 0 4px color-mix(in srgb, var(--accent) 65%, transparent),
    0 0 16px color-mix(in srgb, var(--accent) 25%, transparent);
  border-radius: 6px;
}

/* ─────────────── quality 模式：补全覆盖区域 ─────────────── */

/* 窗口控制按钮（最小化/最大化/关闭）：通透玻璃 + 微妙交互 */
html[data-vis-active='true'][data-vis-mode='quality'] .titlebar-btn {
  background: transparent;
  transition: background 0.15s ease, color 0.15s ease, box-shadow 0.15s ease;
  border-radius: 6px;
}
html[data-vis-active='true'][data-vis-mode='quality'] .titlebar-btn:hover {
  background: rgba(255, 255, 255, 0.08);
  color: var(--text-bright);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.08);
}
html[data-vis-active='true'][data-vis-mode='quality'] .titlebar-btn:active {
  background: rgba(255, 255, 255, 0.04);
}
html[data-vis-active='true'][data-vis-mode='quality'] .titlebar-btn.close:hover {
  background: rgba(232, 17, 35, 0.9);
  color: #fff;
  box-shadow: 0 0 16px rgba(232, 17, 35, 0.35);
}

/* 播放器底栏：通透玻璃 */
html[data-vis-active='true'][data-vis-mode='quality'] .player-bar {
  background:
    linear-gradient(0deg, rgba(99, 102, 241, 0.04), transparent 60%),
    color-mix(in srgb, var(--bg-card) 25%, transparent);
  border-top: 1px solid rgba(180, 210, 255, 0.16);
  backdrop-filter: blur(12px) brightness(1.1) saturate(135%);
  -webkit-backdrop-filter: blur(12px) brightness(1.1) saturate(135%);
  box-shadow: 0 -1px 6px rgba(0, 0, 0, 0.08);
}

/* 侧边栏：通透玻璃 */
html[data-vis-active='true'][data-vis-mode='quality'] .sidebar,
html[data-vis-active='true'][data-vis-mode='quality'] .player-main {
  background:
    linear-gradient(180deg, rgba(255, 255, 255, 0.02), transparent 50%),
    color-mix(in srgb, var(--bg-card) 22%, transparent);
  backdrop-filter: blur(10px) brightness(1.08) saturate(128%);
  -webkit-backdrop-filter: blur(10px) brightness(1.08) saturate(128%);
  border-right: 1px solid rgba(180, 210, 255, 0.12);
}

/* 播放控制按钮（mode-btn）：悬停辉光 + 按下收缩 */
html[data-vis-active='true'][data-vis-mode='quality'] .mode-btn {
  border-radius: 10px;
  transition: color 0.2s ease, background 0.2s ease, transform 0.15s ease, box-shadow 0.2s ease;
}
html[data-vis-active='true'][data-vis-mode='quality'] .mode-btn:hover {
  background: rgba(255, 255, 255, 0.07);
  color: var(--accent);
  box-shadow: 0 0 14px rgba(99, 102, 241, 0.15);
}
html[data-vis-active='true'][data-vis-mode='quality'] .mode-btn:active {
  transform: scale(0.92);
}

/* 设备项：通透玻璃质感 */
html[data-vis-active='true'][data-vis-mode='quality'] .device-item {
  background: rgba(255, 255, 255, 0.035);
  border: 1px solid rgba(180, 210, 255, 0.14);
  border-radius: 12px;
  box-shadow: 0 1px 4px rgba(0, 0, 0, 0.1), inset 0 1px 0 rgba(255, 255, 255, 0.05);
  transition: all 0.22s ease;
}
html[data-vis-active='true'][data-vis-mode='quality'] .device-item:hover {
  background: rgba(255, 255, 255, 0.06);
  border-color: rgba(180, 210, 255, 0.26);
  transform: translateY(-1px);
  box-shadow:
    0 3px 10px rgba(0, 0, 0, 0.16),
    inset 0 1px 0 rgba(255, 255, 255, 0.08),
    0 0 0 1px rgba(255, 255, 255, 0.02);
}
html[data-vis-active='true'][data-vis-mode='quality'] .device-item.active {
  background:
    linear-gradient(135deg, rgba(99, 102, 241, 0.14), rgba(6, 182, 212, 0.06)),
    rgba(99, 102, 241, 0.08);
  border-color: var(--accent);
  box-shadow:
    0 0 14px var(--accent-glow),
    0 2px 8px rgba(0, 0, 0, 0.14),
    inset 0 1px 0 rgba(255, 255, 255, 0.1),
    inset 0 0 20px rgba(99, 102, 241, 0.06);
}
html[data-vis-active='true'][data-vis-mode='quality'] .device-item .device-icon {
  background: rgba(255, 255, 255, 0.05);
  border: 1px solid rgba(180, 210, 255, 0.16);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.06);
}

/* DSP 处理器项 */
html[data-vis-active='true'][data-vis-mode='quality'] .processor-item {
  background: rgba(255, 255, 255, 0.035);
  border: 1px solid rgba(180, 210, 255, 0.14);
  border-radius: 12px;
  box-shadow: 0 1px 4px rgba(0, 0, 0, 0.1), inset 0 1px 0 rgba(255, 255, 255, 0.05);
  transition: all 0.22s ease;
}
html[data-vis-active='true'][data-vis-mode='quality'] .processor-item:hover {
  background: rgba(255, 255, 255, 0.06);
  border-color: rgba(180, 210, 255, 0.26);
  box-shadow:
    0 2px 8px rgba(0, 0, 0, 0.14),
    inset 0 1px 0 rgba(255, 255, 255, 0.08);
}

/* 可折叠头部 */
html[data-vis-active='true'][data-vis-mode='quality'] .collapsible-header {
  border-radius: 10px;
  transition: background 0.2s ease;
}
html[data-vis-active='true'][data-vis-mode='quality'] .collapsible-header:hover {
  background: rgba(255, 255, 255, 0.06);
}

/* 分割条（split-bar）：悬停辉光 */
html[data-vis-active='true'][data-vis-mode='quality'] .split-bar.vertical::after,
html[data-vis-active='true'][data-vis-mode='quality'] .split-bar.horizontal::after {
  background: rgba(120, 160, 255, 0.35);
}
html[data-vis-active='true'][data-vis-mode='quality'] .split-bar.vertical:hover::after,
html[data-vis-active='true'][data-vis-mode='quality'] .split-bar.horizontal:hover::after {
  background: var(--accent);
  box-shadow: 0 0 12px var(--accent-glow);
}

/* 进度条 / 音量滑块 */
html[data-vis-active='true'][data-vis-mode='quality'] input[type='range'] {
  -webkit-appearance: none;
  appearance: none;
  height: 6px;
  border-radius: 999px;
  background: rgba(255, 255, 255, 0.1);
  box-shadow: inset 0 1px 2px rgba(0, 0, 0, 0.2);
}
html[data-vis-active='true'][data-vis-mode='quality'] input[type='range']::-webkit-slider-thumb {
  -webkit-appearance: none;
  width: 16px;
  height: 16px;
  border-radius: 50%;
  background: linear-gradient(135deg, var(--accent), var(--accent2));
  border: 2px solid rgba(255, 255, 255, 0.2);
  box-shadow:
    0 0 12px var(--accent-glow),
    0 3px 8px rgba(0, 0, 0, 0.35),
    inset 0 1px 0 rgba(255, 255, 255, 0.25);
  cursor: pointer;
  transition: transform 0.15s ease, box-shadow 0.15s ease;
}
html[data-vis-active='true'][data-vis-mode='quality'] input[type='range']::-webkit-slider-thumb:hover {
  transform: scale(1.25);
  box-shadow:
    0 0 20px var(--accent-glow),
    0 4px 12px rgba(0, 0, 0, 0.4),
    inset 0 1px 0 rgba(255, 255, 255, 0.3);
}
html[data-vis-active='true'][data-vis-mode='quality'] input[type='range']::-webkit-slider-thumb:active {
  transform: scale(0.92);
}

/* 通用 ghost 按钮 */
html[data-vis-active='true'][data-vis-mode='quality'] .btn-ghost {
  border-radius: 10px;
  background: rgba(255, 255, 255, 0.03);
  transition: all 0.2s ease;
}
html[data-vis-active='true'][data-vis-mode='quality'] .btn-ghost:hover {
  background: rgba(255, 255, 255, 0.07);
}
html[data-vis-active='true'][data-vis-mode='quality'] .btn-ghost:active {
  transform: scale(0.96);
}

/* 模态对话框 */
html[data-vis-active='true'][data-vis-mode='quality'] .modal-overlay {
  background: rgba(0, 0, 0, 0.55);
  backdrop-filter: blur(10px);
  -webkit-backdrop-filter: blur(10px);
}
html[data-vis-active='true'][data-vis-mode='quality'] .modal-dialog {
  background:
    linear-gradient(135deg, rgba(255, 255, 255, 0.07), rgba(255, 255, 255, 0.015)),
    color-mix(in srgb, var(--bg-card) 30%, transparent);
  border: 1px solid rgba(180, 210, 255, 0.22);
  backdrop-filter: blur(16px) brightness(1.12) saturate(140%);
  -webkit-backdrop-filter: blur(16px) brightness(1.12) saturate(140%);
  box-shadow:
    0 12px 36px rgba(0, 0, 0, 0.32),
    0 4px 12px rgba(0, 0, 0, 0.18),
    0 0 0 1px rgba(255, 255, 255, 0.03),
    inset 0 1px 0 rgba(255, 255, 255, 0.16),
    inset 0 -1px 0 rgba(0, 0, 0, 0.1),
    inset 0 0 60px rgba(99, 102, 241, 0.06);
}
html[data-vis-active='true'][data-vis-mode='quality'] .modal-close-btn {
  border-radius: 8px;
  transition: all 0.15s ease;
}
html[data-vis-active='true'][data-vis-mode='quality'] .modal-close-btn:hover {
  background: rgba(255, 255, 255, 0.1);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.08);
}

/* 设置页面二级导航栏（通用/外观/音频/频谱/关于） */
html[data-vis-active='true'][data-vis-mode='quality'] .settings-tabs {
  background:
    linear-gradient(180deg, rgba(255, 255, 255, 0.025), transparent 70%),
    color-mix(in srgb, var(--bg) 78%, transparent) !important;
  backdrop-filter: blur(12px) brightness(1.08) saturate(130%);
  -webkit-backdrop-filter: blur(12px) brightness(1.08) saturate(130%);
  border-bottom: 1px solid rgba(180, 210, 255, 0.14) !important;
  box-shadow: 0 2px 10px rgba(0, 0, 0, 0.12);
}
html[data-vis-active='true'][data-vis-mode='quality'] .settings-tab {
  position: relative;
  transition: color 0.2s ease, background 0.2s ease, text-shadow 0.2s ease;
  border-radius: 8px 8px 0 0;
  margin: 0 2px;
}
html[data-vis-active='true'][data-vis-mode='quality'] .settings-tab:hover {
  background: rgba(255, 255, 255, 0.05);
  color: var(--text-bright) !important;
}
html[data-vis-active='true'][data-vis-mode='quality'] .settings-tab.active {
  background: linear-gradient(180deg, rgba(99, 102, 241, 0.16), transparent 75%);
  text-shadow: 0 0 10px var(--accent-glow);
  box-shadow: 0 -2px 14px rgba(99, 102, 241, 0.1);
}

/* 设置页面卡片 */
html[data-vis-active='true'][data-vis-mode='quality'] .settings-section,
html[data-vis-active='true'][data-vis-mode='quality'] .settings-card,
html[data-vis-active='true'][data-vis-mode='quality'] .settings-group {
  background: rgba(255, 255, 255, 0.03);
  border: 1px solid rgba(180, 210, 255, 0.14);
  border-radius: 14px;
  box-shadow:
    0 2px 8px rgba(0, 0, 0, 0.12),
    inset 0 1px 0 rgba(255, 255, 255, 0.06);
}
/* quality 模式下：进一步收缩 toast/menu/cover/tooltip 等外围投影 */
html[data-vis-active='true'][data-vis-mode='quality'] .toast {
  box-shadow: 0 2px 10px rgba(0, 0, 0, 0.28) !important;
}
html[data-vis-active='true'][data-vis-mode='quality'] .context-menu {
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.32) !important;
}
html[data-vis-active='true'][data-vis-mode='quality'] .player-cover {
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.22) !important;
}
html[data-vis-active='true'][data-vis-mode='quality'] .player-cover-large {
  box-shadow: 0 3px 14px rgba(0, 0, 0, 0.28) !important;
}
html[data-vis-active='true'][data-vis-mode='quality'] .progress-tooltip {
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.24) !important;
}

/* ─────────────── 氛围光斑层(插件自有 DOM,随音频呼吸) ───────────────
   activate() 在 #root 首子节点注入 <div class="vis-ambient-layer">(与噪点
   层同栈位,内容之下)。五个光斑的颜色全部由 --accent / --accent2 派生,
   跟随用户配色方案;frame() 写入的 --vis-* 变量驱动呼吸与节拍脉冲;
   漂移用极慢 keyframes(47~101s),只动 transform/opacity,合成器友好。

   静音可见性:光斑浓度按"无音乐也有质感"设计——基础不透明度较高,
   并以整层极缓慢的 transform 呼吸(17s 一个来回)赋予明暗起伏,音乐响起时
   --vis-energy / --vis-bass 等再叠加鼓点响应。 */
.vis-ambient-layer {
  position: fixed;
  inset: 0;
  z-index: 0;
  overflow: hidden;
  pointer-events: none;
  opacity: 0;
  transition: opacity 0.6s ease;
}
html[data-vis-active='true'][data-vis-mode='quality'] .vis-ambient-layer {
  opacity: 1;
}
/* 音频能量决定整体浓度（覆盖上面的 opacity: 1，必须排在其后） */
html[data-vis-active='true'][data-vis-mode='quality'] .vis-ambient-layer {
  opacity: calc(0.92 + var(--vis-energy, 0) * 0.08);
  /* 常驻呼吸：整层极缓慢地推近/回落——静音时界面依然"活着"。
     用 transform 而非透明度/自定义属性动画：transform 走合成器，
     不会每帧重绘这张全屏渐变，保持 GPU 低占用。 */
  animation: vis-idle-breath 17s ease-in-out infinite alternate;
  will-change: transform;
}
@keyframes vis-idle-breath {
  from { transform: scale(1) translate3d(0, 0, 0); }
  to   { transform: scale(1.05) translate3d(0, -1.4vmax, 0); }
}
.vis-ambient-layer .vis-blob {
  position: absolute;
  border-radius: 50%;
  will-change: transform, opacity;
}
.vis-ambient-layer .b1 {
  width: 58vmax; height: 58vmax; left: -14vmax; top: -20vmax;
  background: radial-gradient(circle,
    color-mix(in srgb, var(--accent, #06b6d4) 30%, transparent), transparent 66%);
  animation: vis-drift-1 41s ease-in-out infinite alternate;
}
.vis-ambient-layer .b2 {
  width: 52vmax; height: 52vmax; right: -16vmax; top: -14vmax;
  background: radial-gradient(circle,
    color-mix(in srgb, var(--accent2, #6366f1) 32%, transparent), transparent 64%);
  animation: vis-drift-2 53s ease-in-out infinite alternate;
}
.vis-ambient-layer .b3 {
  width: 46vmax; height: 46vmax; left: 28%; bottom: -22vmax;
  background: radial-gradient(circle,
    color-mix(in srgb, var(--accent2, #6366f1) 24%, transparent), transparent 68%);
  animation: vis-drift-3 61s ease-in-out infinite alternate;
}
.vis-ambient-layer .b4 {
  width: 34vmax; height: 34vmax; right: 6%; bottom: -8vmax;
  background: radial-gradient(circle,
    color-mix(in srgb, var(--accent, #06b6d4) 20%, transparent), transparent 70%);
  animation: vis-drift-2 71s ease-in-out infinite alternate-reverse;
}
.vis-ambient-layer .b5 {
  width: 26vmax; height: 26vmax; left: -6vmax; bottom: 6%;
  background: radial-gradient(circle,
    color-mix(in srgb, var(--accent2, #6366f1) 18%, transparent), transparent 72%);
  animation: vis-drift-1 79s ease-in-out infinite alternate-reverse;
}
/* 音频呼吸:scale 独立属性不与漂移 keyframes 的 transform 冲突。
   bass 撑大低频光斑,mid/high 分频驱动中高频光斑,beat 叠加快速衰减脉冲。 */
html[data-vis-active='true'][data-vis-mode='quality'] .vis-ambient-layer .b1 {
  scale: calc(1 + var(--vis-bass, 0) * 0.14 + var(--vis-beat, 0) * 0.08);
  opacity: calc(0.85 + var(--vis-bass, 0) * 0.15);
}
html[data-vis-active='true'][data-vis-mode='quality'] .vis-ambient-layer .b2 {
  scale: calc(1 + var(--vis-mid, 0) * 0.10);
}
html[data-vis-active='true'][data-vis-mode='quality'] .vis-ambient-layer .b3 {
  scale: calc(1 + var(--vis-mid, 0) * 0.08 + var(--vis-beat, 0) * 0.04);
}
html[data-vis-active='true'][data-vis-mode='quality'] .vis-ambient-layer .b4 {
  scale: calc(1 + var(--vis-high, 0) * 0.12);
  opacity: calc(0.82 + var(--vis-high, 0) * 0.18);
}
html[data-vis-active='true'][data-vis-mode='quality'] .vis-ambient-layer .b5 {
  scale: calc(1 + var(--vis-high, 0) * 0.16);
  opacity: calc(0.8 + var(--vis-high, 0) * 0.2);
}
@keyframes vis-drift-1 {
  from { transform: translate3d(0, 0, 0) scale(1); }
  to   { transform: translate3d(15vmax, 10vmax, 0) scale(1.22); }
}
@keyframes vis-drift-2 {
  from { transform: translate3d(0, 0, 0) scale(1); }
  to   { transform: translate3d(-13vmax, 14vmax, 0) scale(1.24); }
}
@keyframes vis-drift-3 {
  from { transform: translate3d(0, 0, 0) scale(1); }
  to   { transform: translate3d(10vmax, -12vmax, 0) scale(1.18); }
}
/* 模式未激活时暂停漂移动画并释放合成层（opacity 渐隐过渡不受影响）。
   will-change 常驻会把 5 个大光斑 + 全屏层钉在合成器显存里（约数十
   MB）——休息态（插件启用但质感模式关闭）没必要付这笔钱；激活时
   下方规则会重新提升。 */
html:not([data-vis-active='true'][data-vis-mode='quality']) .vis-ambient-layer .vis-blob,
html:not([data-vis-active='true'][data-vis-mode='quality']) .vis-ambient-layer {
  animation-play-state: paused;
  will-change: auto;
}
@media (prefers-reduced-motion: reduce) {
  .vis-ambient-layer .vis-blob,
  .vis-ambient-layer {
    animation: none;
  }
}
`;

// 惰性获取:脚本在宿主 webview 中求值时 document 必然存在,
// 不在求值期就解引用,也便于无 DOM 环境下的冒烟测试
function docEl() { return document.documentElement }

// ── 音频联动状态 ─────────────────────────────────────────
// env 平滑:起音快(0.38)、释放慢(0.08),对鼓点跟手、收尾柔和
var env = { bass: 0, mid: 0, high: 0, energy: 0, beat: 0 }
var features = null          // 最近一次 audio-features(用引擎算好的 beat)
var lastAudioAt = 0          // 任一音频流最后到达时刻(停播后判定 stale 归零)
var lastWriteAt = 0
var layer = null

function clamp01(v) { return v < 0 ? 0 : v > 1 ? 1 : v }

function bandAvg(bands, from, to) {
  var sum = 0
  var n = 0
  for (var i = from; i < to && i < bands.length; i++) {
    sum += bands[i]
    n++
  }
  return n ? sum / n : 0
}

function follow(cur, target) {
  return cur + (target - cur) * (target > cur ? 0.38 : 0.08)
}

function onAudioEvent(e) {
  lastAudioAt = performance.now()
  features = (e && e.detail) || null
  if (features && features.beat) env.beat = 1
}

function onBandsEvent() {
  lastAudioAt = performance.now()
}

function ensureLayer() {
  if (layer || !document.body) return
  layer = document.createElement('div')
  layer.className = 'vis-ambient-layer'
  layer.setAttribute('aria-hidden', 'true')
  for (var i = 1; i <= 5; i++) {
    var b = document.createElement('div')
    b.className = 'vis-blob b' + i
    layer.appendChild(b)
  }
  var root = document.getElementById('root') || document.body
  // 首子节点:与 #root::before 噪点层同栈位,始终衬在内容之下
  root.insertBefore(layer, root.firstChild)
}

function removeLayer() {
  if (layer && layer.parentNode) layer.parentNode.removeChild(layer)
  layer = null
  var names = ['--vis-bass', '--vis-mid', '--vis-high', '--vis-energy', '--vis-beat']
  for (var i = 0; i < names.length; i++) docEl().style.removeProperty(names[i])
}

function frame(now) {
  if (!layer) return
  var raw = window.__phononSpectrumBands
  var bands = raw && raw.length ? raw : []
  var stale = now - lastAudioAt > 300
  var tBass = 0
  var tMid = 0
  var tHigh = 0
  if (!stale && bands.length) {
    tBass = clamp01(bandAvg(bands, 0, 6) * 1.15)
    tMid = clamp01(bandAvg(bands, 6, 18))
    tHigh = clamp01(bandAvg(bands, 18, 32) * 1.25)
  }
  var tEnergy = clamp01(tBass * 0.45 + tMid * 0.35 + tHigh * 0.2)
  env.bass = follow(env.bass, tBass)
  env.mid = follow(env.mid, tMid)
  env.high = follow(env.high, tHigh)
  env.energy = follow(env.energy, tEnergy)
  env.beat *= 0.9
  if (env.beat < 0.01) env.beat = 0
  if (now - lastWriteAt < 25) return // ~40Hz 写入足够,省掉多余 style churn
  lastWriteAt = now
  var s = docEl().style
  s.setProperty('--vis-bass', env.bass.toFixed(3))
  s.setProperty('--vis-mid', env.mid.toFixed(3))
  s.setProperty('--vis-high', env.high.toFixed(3))
  s.setProperty('--vis-energy', env.energy.toFixed(3))
  s.setProperty('--vis-beat', env.beat.toFixed(3))
}

var active = false // activate/deactivate 必须成对生效,防重复挂监听器

return {
  name: '视觉升华插件',
  type: '2d',
  styles: STYLES,
  activate: function () {
    if (active) return
    active = true
    ensureLayer()
    lastAudioAt = performance.now()
    window.addEventListener('audio-features', onAudioEvent)
    window.addEventListener('spectrum-bands', onBandsEvent)
  },
  deactivate: function () {
    if (!active) return
    active = false
    window.removeEventListener('audio-features', onAudioEvent)
    window.removeEventListener('spectrum-bands', onBandsEvent)
    removeLayer()
  },
  frame: function (now) { frame(now) },
  init: function (canvas, w, h) {
    var ctx = canvas.getContext('2d');
    if (ctx) {
      ctx.fillStyle = '#05060a';
      ctx.fillRect(0, 0, w, h);
    }
  },
  render: function (canvas, features, w, h, time) {
    var ctx = canvas.getContext('2d');
    if (!ctx) return;
    // 极简占位:视觉增强通过 styles/ambient 层生效,这里不绘制特效
    ctx.fillStyle = 'rgba(5, 6, 11, 0.1)';
    ctx.fillRect(0, 0, w, h);
    ctx.fillStyle = 'rgba(140, 160, 200, 0.35)';
    ctx.font = '500 14px system-ui, sans-serif';
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    ctx.fillText('可视化模式已激活', w / 2, h / 2 - 8);
    ctx.fillStyle = 'rgba(140, 160, 200, 0.2)';
    ctx.font = '11px system-ui, sans-serif';
    ctx.fillText('请在顶部导航栏的按钮中切换模式', w / 2, h / 2 + 14);
  },
  resize: function (canvas, w, h) {},
  destroy: function () {
    deactivate()
  },
};
