//! Direct2D + DirectComposition rendering canvas.
//!
//! `Canvas` renders into a composition swapchain (per-pixel alpha via DWM),
//! which is what unlocks the visual upgrade over GDI: true soft drop
//! shadows, antialiased rounded corners that blend against the desktop, and
//! DirectWrite text. The window must be created with
//! `WS_EX_NOREDIRECTIONBITMAP`.
//!
//! Device model: `begin()/end()` wrap each frame; if EndDraw reports
//! `D2DERR_RECREATE_TARGET` the internal resources are dropped and
//! [`Canvas::needs_recreate`] turns true — the caller re-runs
//! [`Canvas::new`] before the next frame.

use windows::core::{w, Interface};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::DirectComposition::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::System::Com::CoInitializeEx;

use crate::dpi::{get_dpi, s};
use crate::gdi::make_rect;

// ── Text flags (mirror the DT_* constants the UI was written against) ──

pub mod tf {
    pub const LEFT: u32 = 0x0001;
    pub const CENTER: u32 = 0x0002;
    pub const RIGHT: u32 = 0x0004;
    pub const TOP: u32 = 0x0008;
    pub const VCENTER: u32 = 0x0010;
    pub const BOTTOM: u32 = 0x0020;
    pub const SINGLELINE: u32 = 0x0040;
    pub const WORDBREAK: u32 = 0x0080;
    pub const END_ELLIPSIS: u32 = 0x0100;
    pub const CALCRECT: u32 = 0x0200; // measure only — don't draw
}

// ── Color conversion (COLORREF 0x00BBGGRR → float RGBA) ──

#[inline]
pub fn color_f(c: u32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: (c & 0xFF) as f32 / 255.0,
        g: ((c >> 8) & 0xFF) as f32 / 255.0,
        b: ((c >> 16) & 0xFF) as f32 / 255.0,
        a: 1.0,
    }
}

#[inline]
fn color_fa(c: u32, alpha: f32) -> D2D1_COLOR_F {
    let mut f = color_f(c);
    f.a = alpha;
    f
}

#[inline]
fn rect_f(rc: &RECT) -> D2D_RECT_F {
    D2D_RECT_F {
        left: rc.left as f32,
        top: rc.top as f32,
        right: rc.right as f32,
        bottom: rc.bottom as f32,
    }
}

// ── Font style (replaces the GDI HFONT set from the old Theme) ──

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FontStyle {
    Big,
    Title,
    Body,
    BodyBold,
    Small,
    Micro,
}

impl FontStyle {
    fn spec(self) -> (i32, u32) {
        // (point size, weight) — mirrors the old make_font calls.
        match self {
            FontStyle::Big => (20, 700),
            FontStyle::Title => (13, 600),
            FontStyle::Body => (11, 400),
            FontStyle::BodyBold => (11, 600),
            FontStyle::Small => (9, 400),
            FontStyle::Micro => (8, 400),
        }
    }
}

// ── Canvas ──

/// Direct2D render context for one window, presented through
/// DirectComposition (per-pixel alpha).
pub struct Canvas {
    dwrite: IDWriteFactory,
    context: ID2D1DeviceContext,
    swapchain: IDXGISwapChain1,
    dcomp_device: IDCompositionDevice,
    _dcomp_target: IDCompositionTarget,
    brush: ID2D1SolidColorBrush,
    /// Cached text formats, one per font style.
    formats: [Option<IDWriteTextFormat>; 6],
    pub width: i32,
    pub height: i32,
}

impl Canvas {
    /// Create the D3D/D2D/DComp chain for `hwnd`. The window must have
    /// `WS_EX_NOREDIRECTIONBITMAP`.
    pub fn new(hwnd: HWND, width: i32, height: i32) -> windows::core::Result<Self> {
        unsafe {
            // D2D needs COM on the UI thread; tolerate an already-initialized
            // apartment with a different model.
            let _ = CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);

            // D3D11 device with BGRA support (hardware, WARP fallback).
            let mut d3d: Option<ID3D11Device> = None;
            let flags = D3D11_CREATE_DEVICE_BGRA_SUPPORT;
            if D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                None,
                flags,
                None,
                D3D11_SDK_VERSION,
                Some(&mut d3d),
                None,
                None,
            )
            .is_err()
            {
                D3D11CreateDevice(
                    None,
                    D3D_DRIVER_TYPE_WARP,
                    None,
                    flags,
                    None,
                    D3D11_SDK_VERSION,
                    Some(&mut d3d),
                    None,
                    None,
                )?;
            }
            let d3d = d3d.expect("D3D11CreateDevice produced no device");

            let dxgi_device: IDXGIDevice = d3d
                .cast()
                .inspect_err(|e| log::error!("D2D: cast IDXGIDevice: {e}"))?;
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            // NOTE: IDXGIDevice's parent is the *adapter*, not the factory —
            // GetParent::<IDXGIFactory2>() fails with E_NOINTERFACE. Create
            // the factory directly instead.
            let dxgi_factory: IDXGIFactory2 = CreateDXGIFactory1()
                .inspect_err(|e| log::error!("D2D: CreateDXGIFactory1: {e}"))?;

            // D2D device + context.
            let factory: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)
                .inspect_err(|e| log::error!("D2D: create factory: {e}"))?;
            let factory1: ID2D1Factory1 = factory
                .cast()
                .inspect_err(|e| log::error!("D2D: cast ID2D1Factory1: {e}"))?;
            let d2d_device: ID2D1Device = factory1
                .CreateDevice(&dxgi_device)
                .inspect_err(|e| log::error!("D2D: CreateDevice: {e}"))?;
            let context: ID2D1DeviceContext = d2d_device
                .CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)
                .inspect_err(|e| log::error!("D2D: CreateDeviceContext: {e}"))?;

            // Composition swapchain with premultiplied alpha.
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: width.max(1) as u32,
                Height: height.max(1) as u32,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                Stereo: false.into(),
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                Scaling: DXGI_SCALING_STRETCH,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
                Flags: 0,
            };
            let swapchain = dxgi_factory
                .CreateSwapChainForComposition(&d3d, &desc, None)
                .inspect_err(|e| log::error!("D2D: CreateSwapChainForComposition: {e}"))?;

            // DirectComposition target wired to the swapchain.
            let dcomp_device: IDCompositionDevice = DCompositionCreateDevice(&dxgi_device)
                .inspect_err(|e| log::error!("D2D: DCompositionCreateDevice: {e}"))?;
            let dcomp_target = dcomp_device
                .CreateTargetForHwnd(hwnd, true)
                .inspect_err(|e| log::error!("D2D: CreateTargetForHwnd: {e}"))?;
            let dcomp_visual = dcomp_device
                .CreateVisual()
                .inspect_err(|e| log::error!("D2D: CreateVisual: {e}"))?;
            dcomp_visual
                .SetContent(&swapchain)
                .inspect_err(|e| log::error!("D2D: visual.SetContent: {e}"))?;
            dcomp_target
                .SetRoot(&dcomp_visual)
                .inspect_err(|e| log::error!("D2D: target.SetRoot: {e}"))?;
            dcomp_device.Commit()?;

            let brush = context.CreateSolidColorBrush(&color_f(0x00FFFFFF), None)?;

            let canvas = Self {
                dwrite,
                context,
                swapchain,
                dcomp_device,
                _dcomp_target: dcomp_target,
                brush,
                formats: [None, None, None, None, None, None],
                width,
                height,
            };
            canvas.bind_bitmap()?;
            Ok(canvas)
        }
    }

    /// Bind the swapchain's back buffer as the D2D render target.
    fn bind_bitmap(&self) -> windows::core::Result<()> {
        unsafe {
            let surface: IDXGISurface = self.swapchain.GetBuffer(0)?;
            let props = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0,
                dpiY: 96.0,
                // TARGET is what makes the bitmap usable by SetTarget —
                // without it every draw went nowhere and the window stayed
                // fully transparent. CANNOT_DRAW is the standard pairing
                // for a pure render-target surface.
                bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                colorContext: std::mem::ManuallyDrop::new(None),
            };
            let bitmap = self
                .context
                .CreateBitmapFromDxgiSurface(&surface, Some(&props))?;
            let image: ID2D1Image = bitmap.into();
            self.context.SetTarget(Some(&image));
        }
        Ok(())
    }

    /// Resize the swapchain and rebind the back buffer (WM_SIZE).
    pub fn resize(&mut self, width: i32, height: i32) {
        self.width = width;
        self.height = height;
        unsafe {
            self.context.SetTarget(None);
            if let Err(e) = self.swapchain.ResizeBuffers(
                0,
                width.max(1) as u32,
                height.max(1) as u32,
                DXGI_FORMAT_UNKNOWN,
                DXGI_SWAP_CHAIN_FLAG(0),
            ) {
                log::warn!("ResizeBuffers failed: {}", e);
                return;
            }
        }
        if self.bind_bitmap().is_err() {
            log::warn!("Failed to rebind back buffer after resize");
        }
    }

    /// True if the D3D/D2D resources were dropped (device lost) and must be
    /// recreated on the next paint.
    pub fn needs_recreate(&self) -> bool {
        false
    }

    /// Begin a frame: clear to fully transparent (DWM blends the silhouette
    /// against the desktop).
    pub fn begin(&mut self) {
        unsafe {
            self.context.BeginDraw();
            self.context.Clear(Some(&D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            }));
        }
    }

    /// End a frame: flush D2D, present the swapchain, commit the composition.
    pub fn end(&mut self) {
        unsafe {
            if let Err(e) = self.context.EndDraw(None, None) {
                log::warn!("D2D EndDraw failed: {}", e);
            }
            let _ = self.swapchain.Present(1, DXGI_PRESENT(0));
            let _ = self.dcomp_device.Commit();
        }
    }

    fn rt(&self) -> &ID2D1DeviceContext {
        &self.context
    }

    // ── Brushes ──

    fn set_brush(&self, color: u32) {
        unsafe {
            self.brush.SetColor(&color_f(color));
        }
    }

    fn set_brush_a(&self, color: u32, alpha: f32) {
        unsafe {
            self.brush.SetColor(&color_fa(color, alpha));
        }
    }

    // ── Primitives ──

    pub fn fill_rect(&mut self, rc: &RECT, color: u32) {
        unsafe {
            self.set_brush(color);
            self.rt().FillRectangle(&rect_f(rc), &self.brush);
        }
    }

    pub fn fill_rect_alpha(&mut self, rc: &RECT, color: u32, alpha: f32) {
        unsafe {
            self.set_brush_a(color, alpha);
            self.rt().FillRectangle(&rect_f(rc), &self.brush);
        }
    }

    pub fn fill_round(&mut self, rc: &RECT, radius: i32, color: u32) {
        unsafe {
            self.set_brush(color);
            self.rt().FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: rect_f(rc),
                    radiusX: radius as f32,
                    radiusY: radius as f32,
                },
                &self.brush,
            );
        }
    }

    pub fn fill_round_alpha(&mut self, rc: &RECT, radius: i32, color: u32, alpha: f32) {
        unsafe {
            self.set_brush_a(color, alpha);
            self.rt().FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: rect_f(rc),
                    radiusX: radius as f32,
                    radiusY: radius as f32,
                },
                &self.brush,
            );
        }
    }

    pub fn stroke_round(&mut self, rc: &RECT, radius: i32, color: u32, width: f32) {
        unsafe {
            self.set_brush(color);
            self.rt().DrawRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: rect_f(rc),
                    radiusX: radius as f32,
                    radiusY: radius as f32,
                },
                &self.brush,
                width,
                None,
            );
        }
    }

    pub fn fill_ellipse(&mut self, cx: i32, cy: i32, rx: i32, ry: i32, color: u32) {
        unsafe {
            self.set_brush(color);
            self.rt().FillEllipse(
                &D2D1_ELLIPSE {
                    point: D2D_POINT_2F {
                        x: cx as f32,
                        y: cy as f32,
                    },
                    radiusX: rx as f32,
                    radiusY: ry as f32,
                },
                &self.brush,
            );
        }
    }

    pub fn stroke_ellipse(&mut self, cx: i32, cy: i32, rx: i32, ry: i32, color: u32, width: f32) {
        unsafe {
            self.set_brush(color);
            self.rt().DrawEllipse(
                &D2D1_ELLIPSE {
                    point: D2D_POINT_2F {
                        x: cx as f32,
                        y: cy as f32,
                    },
                    radiusX: rx as f32,
                    radiusY: ry as f32,
                },
                &self.brush,
                width,
                None,
            );
        }
    }

    pub fn line(&mut self, x1: i32, y1: i32, x2: i32, y2: i32, color: u32, width: f32) {
        unsafe {
            self.set_brush(color);
            self.rt().DrawLine(
                D2D_POINT_2F {
                    x: x1 as f32,
                    y: y1 as f32,
                },
                D2D_POINT_2F {
                    x: x2 as f32,
                    y: y2 as f32,
                },
                &self.brush,
                width,
                None,
            );
        }
    }

    /// Vertical linear gradient over `rc`.
    pub fn fill_gradient_v(&mut self, rc: &RECT, top: u32, bottom: u32) {
        self.fill_gradient(rc, top, bottom, false);
    }

    /// Horizontal linear gradient over `rc`.
    pub fn fill_gradient_h(&mut self, rc: &RECT, left: u32, right: u32) {
        self.fill_gradient(rc, left, right, true);
    }

    fn fill_gradient(&mut self, rc: &RECT, c1: u32, c2: u32, horizontal: bool) {
        let (start, end) = if horizontal {
            (
                D2D_POINT_2F {
                    x: rc.left as f32,
                    y: rc.top as f32,
                },
                D2D_POINT_2F {
                    x: rc.right as f32,
                    y: rc.top as f32,
                },
            )
        } else {
            (
                D2D_POINT_2F {
                    x: rc.left as f32,
                    y: rc.top as f32,
                },
                D2D_POINT_2F {
                    x: rc.left as f32,
                    y: rc.bottom as f32,
                },
            )
        };
        unsafe {
            let stops = [
                D2D1_GRADIENT_STOP {
                    position: 0.0,
                    color: color_f(c1),
                },
                D2D1_GRADIENT_STOP {
                    position: 1.0,
                    color: color_f(c2),
                },
            ];
            let Ok(collection) = self.rt().CreateGradientStopCollection(
                &stops,
                D2D1_COLOR_SPACE_SRGB,
                D2D1_COLOR_SPACE_SRGB,
                D2D1_BUFFER_PRECISION(0),
                D2D1_EXTEND_MODE_CLAMP,
                D2D1_COLOR_INTERPOLATION_MODE(1),
            ) else {
                return;
            };
            let Ok(grad) = self.rt().CreateLinearGradientBrush(
                &D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                    startPoint: start,
                    endPoint: end,
                },
                None,
                &collection,
            ) else {
                return;
            };
            self.rt().FillRectangle(&rect_f(rc), &grad);
        }
    }

    /// Rounded-rect filled with a linear gradient (D2D brushes live in
    /// render-target space, so the gradient aligns with the rect exactly).
    pub fn fill_round_gradient(
        &mut self,
        rc: &RECT,
        radius: i32,
        c1: u32,
        c2: u32,
        horizontal: bool,
    ) {
        let (start, end) = if horizontal {
            (
                D2D_POINT_2F {
                    x: rc.left as f32,
                    y: rc.top as f32,
                },
                D2D_POINT_2F {
                    x: rc.right as f32,
                    y: rc.top as f32,
                },
            )
        } else {
            (
                D2D_POINT_2F {
                    x: rc.left as f32,
                    y: rc.top as f32,
                },
                D2D_POINT_2F {
                    x: rc.left as f32,
                    y: rc.bottom as f32,
                },
            )
        };
        unsafe {
            let stops = [
                D2D1_GRADIENT_STOP {
                    position: 0.0,
                    color: color_f(c1),
                },
                D2D1_GRADIENT_STOP {
                    position: 1.0,
                    color: color_f(c2),
                },
            ];
            let Ok(collection) = self.rt().CreateGradientStopCollection(
                &stops,
                D2D1_COLOR_SPACE_SRGB,
                D2D1_COLOR_SPACE_SRGB,
                D2D1_BUFFER_PRECISION(0),
                D2D1_EXTEND_MODE_CLAMP,
                D2D1_COLOR_INTERPOLATION_MODE(1),
            ) else {
                return;
            };
            let Ok(grad) = self.rt().CreateLinearGradientBrush(
                &D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                    startPoint: start,
                    endPoint: end,
                },
                None,
                &collection,
            ) else {
                return;
            };
            self.rt().FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: rect_f(rc),
                    radiusX: radius as f32,
                    radiusY: radius as f32,
                },
                &grad,
            );
        }
    }

    /// Soft drop shadow: layered rounded rects with black at decreasing
    /// alpha — now a true alpha shadow over the desktop (DComp).
    pub fn draw_soft_shadow(&mut self, rc: &RECT, radius: i32, layers: i32, expand: i32) {
        for i in 0..layers {
            let e = expand - i * (expand / layers.max(1));
            let a = 0.10 - i as f32 * (0.10 / layers as f32);
            let gr = make_rect(rc.left - e, rc.top - e, rc.right + e, rc.bottom + e);
            self.fill_round_alpha(&gr, radius + e, 0x00000000, a.max(0.02));
        }
    }

    /// Clip all drawing to `rc` until the returned guard is dropped.
    pub fn push_clip(&mut self, rc: &RECT) {
        unsafe {
            self.rt()
                .PushAxisAlignedClip(&rect_f(rc), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        }
    }

    pub fn pop_clip(&mut self) {
        unsafe {
            self.rt().PopAxisAlignedClip();
        }
    }

    // ── Text ──

    fn format(&mut self, style: FontStyle) -> IDWriteTextFormat {
        let idx = style as usize;
        if let Some(f) = &self.formats[idx] {
            return f.clone();
        }
        let (pt, weight) = style.spec();
        let dpi = get_dpi();
        let size = (pt * dpi / 72) as f32; // same pixel height the GDI font used
        let format = unsafe {
            self.dwrite
                .CreateTextFormat(
                    w!("Segoe UI"),
                    None,
                    DWRITE_FONT_WEIGHT(weight as u16 as i32),
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    size,
                    w!(""),
                )
                .expect("CreateTextFormat")
        };
        self.formats[idx] = Some(format.clone());
        format
    }

    fn build_layout(
        &mut self,
        text: &str,
        rc: &RECT,
        style: FontStyle,
        flags: u32,
    ) -> (IDWriteTextLayout, DWRITE_TEXT_METRICS) {
        let format = self.format(style);
        let wtext: Vec<u16> = text.encode_utf16().collect();
        let w = (rc.right - rc.left).max(1) as f32;
        let h = (rc.bottom - rc.top).max(1) as f32;
        unsafe {
            let layout = self
                .dwrite
                .CreateTextLayout(&wtext, &format, w, h)
                .expect("CreateTextLayout");

            layout
                .SetTextAlignment(match flags & (tf::LEFT | tf::CENTER | tf::RIGHT) {
                    tf::CENTER => DWRITE_TEXT_ALIGNMENT_CENTER,
                    tf::RIGHT => DWRITE_TEXT_ALIGNMENT_TRAILING,
                    _ => DWRITE_TEXT_ALIGNMENT_LEADING,
                })
                .ok();

            let single = flags & tf::SINGLELINE != 0;
            layout
                .SetWordWrapping(if single || flags & tf::WORDBREAK == 0 {
                    DWRITE_WORD_WRAPPING_NO_WRAP
                } else {
                    DWRITE_WORD_WRAPPING_WRAP
                })
                .ok();

            if flags & tf::END_ELLIPSIS != 0 {
                let _ = layout.SetTrimming(
                    &DWRITE_TRIMMING {
                        granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                        delimiter: 0,
                        delimiterCount: 0,
                    },
                    None,
                );
            }

            layout
                .SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR)
                .ok();

            let mut metrics = DWRITE_TEXT_METRICS::default();
            layout.GetMetrics(&mut metrics).ok();
            (layout, metrics)
        }
    }

    /// Draw text into `rc`. Returns the rendered text height in pixels.
    pub fn text(&mut self, text: &str, rc: &RECT, style: FontStyle, color: u32, flags: u32) -> i32 {
        if text.is_empty() {
            return 0;
        }
        let (layout, metrics) = self.build_layout(text, rc, style, flags);
        if flags & tf::CALCRECT != 0 {
            return metrics.height as i32;
        }

        // Vertical placement from the measured height.
        let box_h = (rc.bottom - rc.top) as f32;
        let top = if flags & tf::VCENTER != 0 {
            rc.top as f32 + (box_h - metrics.height).max(0.0) / 2.0
        } else if flags & tf::BOTTOM != 0 {
            (rc.bottom as f32 - metrics.height).max(rc.top as f32)
        } else {
            rc.top as f32
        };

        unsafe {
            self.set_brush(color);
            self.rt().DrawTextLayout(
                D2D_POINT_2F {
                    x: rc.left as f32,
                    y: top,
                },
                &layout,
                &self.brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }
        metrics.height as i32
    }

    /// Measure text for the given rect width (DT_CALCRECT analog).
    pub fn measure_text(
        &mut self,
        text: &str,
        width: i32,
        style: FontStyle,
        wrap: bool,
    ) -> (i32, i32) {
        let rc = RECT {
            left: 0,
            top: 0,
            right: width.max(1),
            bottom: i32::MAX / 2,
        };
        let mut flags = tf::CALCRECT;
        if wrap {
            flags |= tf::WORDBREAK;
        }
        let (_layout, metrics) = self.build_layout(text, &rc, style, flags);
        (metrics.width as i32, metrics.height as i32)
    }

    /// Scale a logical pixel length (re-exported convenience for pages).
    pub fn scale(len: i32) -> i32 {
        s(len)
    }
}
