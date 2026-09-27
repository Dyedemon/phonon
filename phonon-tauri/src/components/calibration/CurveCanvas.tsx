import React, { useRef, useEffect } from 'react';

interface CurveData {
  frequencies: number[];
  db: number[];
}

interface CurveCanvasProps {
  measured: CurveData | null;
  target: CurveData;
  fitted: CurveData | null;
  residual: CurveData | null;
  width?: number;
  height?: number;
}

/**
 * Canvas-based frequency response curve visualizer.
 *
 * Draws up to 4 curves on a log-frequency x-axis (20 Hz – 20 kHz)
 * and linear dB y-axis. Includes grid lines and axis labels.
 */
const CurveCanvas: React.FC<CurveCanvasProps> = ({
  measured,
  target,
  fitted,
  residual,
  width = 720,
  height = 320,
}) => {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    const dpr = window.devicePixelRatio || 1;
    canvas.width = width * dpr;
    canvas.height = height * dpr;
    canvas.style.width = `${width}px`;
    canvas.style.height = `${height}px`;
    ctx.scale(dpr, dpr);

    // Margins
    const ml = 50; // left margin (y-axis labels)
    const mr = 20; // right margin
    const mt = 20; // top margin
    const mb = 30; // bottom margin (x-axis labels)
    const plotW = width - ml - mr;
    const plotH = height - mt - mb;

    // Clear
    ctx.fillStyle = '#1a1a2e';
    ctx.fillRect(0, 0, width, height);

    // Axes
    const fMin = 20;
    const fMax = 20000;
    const dbMin = -30;
    const dbMax = 15;

    const freqToX = (f: number) => {
      const logF = Math.log10(Math.max(f, fMin));
      const logMin = Math.log10(fMin);
      const logMax = Math.log10(fMax);
      return ml + ((logF - logMin) / (logMax - logMin)) * plotW;
    };

    const dbToY = (db: number) => {
      return mt + ((dbMax - db) / (dbMax - dbMin)) * plotH;
    };

    // Grid lines
    ctx.strokeStyle = 'rgba(255,255,255,0.08)';
    ctx.lineWidth = 1;
    ctx.font = '10px system-ui';
    ctx.fillStyle = 'rgba(255,255,255,0.5)';

    // Vertical grid (frequencies)
    const freqMarkers = [20, 50, 100, 200, 500, 1000, 2000, 5000, 10000, 20000];
    for (const f of freqMarkers) {
      const x = freqToX(f);
      ctx.beginPath();
      ctx.moveTo(x, mt);
      ctx.lineTo(x, mt + plotH);
      ctx.stroke();
      // Label
      const label = f >= 1000 ? `${f / 1000}k` : `${f}`;
      ctx.textAlign = 'center';
      ctx.fillText(label, x, mt + plotH + 15);
    }

    // Horizontal grid (dB)
    const dbMarkers = [-30, -20, -10, 0, 10];
    for (const db of dbMarkers) {
      const y = dbToY(db);
      ctx.beginPath();
      ctx.moveTo(ml, y);
      ctx.lineTo(ml + plotW, y);
      ctx.stroke();
      // Label
      ctx.textAlign = 'right';
      ctx.fillText(`${db} dB`, ml - 6, y + 3);
    }

    // Draw curve function
    const drawCurve = (data: CurveData, color: string, lineWidth = 1.5, dashed = false) => {
      if (!data.frequencies.length) return;
      ctx.strokeStyle = color;
      ctx.lineWidth = lineWidth;
      if (dashed) ctx.setLineDash([4, 4]);
      ctx.beginPath();
      let started = false;
      for (let i = 0; i < data.frequencies.length; i++) {
        const f = data.frequencies[i];
        const db = data.db[i];
        if (f < fMin || f > fMax || isNaN(db)) continue;
        const x = freqToX(f);
        const y = dbToY(Math.max(dbMin, Math.min(dbMax, db)));
        if (!started) {
          ctx.moveTo(x, y);
          started = true;
        } else {
          ctx.lineTo(x, y);
        }
      }
      ctx.stroke();
      ctx.setLineDash([]);
    };

    // Target curve (dashed gray)
    drawCurve(target, 'rgba(255,255,255,0.4)', 1, true);

    // Measured (red)
    if (measured) {
      drawCurve(measured, '#ff6b6b', 1.5);
    }

    // Fitted (teal)
    if (fitted) {
      drawCurve(fitted, '#4ecdc4', 1.5);
    }

    // Residual (yellow, thin line)
    if (residual) {
      drawCurve(residual, '#ffd93d', 1);
    }

    // Legend
    const legendY = mt + 10;
    let legendX = ml + 10;
    ctx.font = '11px system-ui';

    const legendItems: { color: string; label: string; dashed?: boolean }[] = [];
    if (measured) legendItems.push({ color: '#ff6b6b', label: 'Measured' });
    legendItems.push({ color: 'rgba(255,255,255,0.4)', label: 'Target', dashed: true });
    if (fitted) legendItems.push({ color: '#4ecdc4', label: 'Fitted' });
    if (residual) legendItems.push({ color: '#ffd93d', label: 'Residual' });

    for (const item of legendItems) {
      ctx.strokeStyle = item.color;
      ctx.lineWidth = 2;
      if (item.dashed) ctx.setLineDash([4, 4]);
      ctx.beginPath();
      ctx.moveTo(legendX, legendY);
      ctx.lineTo(legendX + 20, legendY);
      ctx.stroke();
      ctx.setLineDash([]);
      ctx.fillStyle = 'rgba(255,255,255,0.7)';
      ctx.textAlign = 'left';
      ctx.fillText(item.label, legendX + 26, legendY + 4);
      legendX += ctx.measureText(item.label).width + 50;
    }
  }, [measured, target, fitted, residual, width, height]);

  return (
    <canvas
      ref={canvasRef}
      className="curve-canvas"
      style={{ width, height, borderRadius: 8 }}
    />
  );
};

export default CurveCanvas;
