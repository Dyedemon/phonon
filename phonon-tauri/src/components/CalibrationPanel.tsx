import React, { useState, useCallback, useEffect, useRef } from 'react';
import { listen } from '@tauri-apps/api/event';
import { useI18n } from '../i18n';
import {
  runCalibrationFit,
  applyCalibrationResult,
  bypassCalibration,
  getCalibrationState,
  getTargetCurve,
  calibrationSampleRate,
  parseMeasurement,
  toMeasurementPairs,
  TARGET_CURVES,
  type FitRequest,
  type FitResult,
  type FitCurves,
  type MeasurementCurve,
} from '../api/calibration';
import { listPlugins } from '../api/plugins';
import CurveCanvas from './calibration/CurveCanvas';

/** True when a plugin manifest belongs to the external calibration plugin. */
function isCalibrationName(name: string): boolean {
  const n = name.toLowerCase();
  return n === 'calibration' || n === 'acoustic calibration';
}

const CalibrationPanel: React.FC = () => {
  const { t } = useI18n();
  const [pluginFound, setPluginFound] = useState(false);
  const [pluginLoaded, setPluginLoaded] = useState(false);
  const [checking, setChecking] = useState(true);
  const [fitting, setFitting] = useState(false);
  const [applying, setApplying] = useState(false);
  const [fitResultL, setFitResultL] = useState<FitResult | null>(null);
  const [fitResultR, setFitResultR] = useState<FitResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [calibrationEnabled, setCalibrationEnabled] = useState(false);

  // Fit parameters (the engine ABI takes max_bands / fir_taps / sample_rate;
  // smoothing and boost/cut limits are engine-internal).
  const [targetCurveId, setTargetCurveId] = useState(TARGET_CURVES[0].id);
  const [maxBands, setMaxBands] = useState(16);
  const [firTaps, setFirTaps] = useState(2048);
  const [sampleRate, setSampleRate] = useState(48000);

  // Measurement data
  const [measuredL, setMeasuredL] = useState<MeasurementCurve | null>(null);
  const [measuredR, setMeasuredR] = useState<MeasurementCurve | null>(null);
  const [linkLr, setLinkLr] = useState(true);

  // Target curve for display (fetched from the engine — same data it fits against)
  const [targetCurve, setTargetCurve] = useState<FitCurves | null>(null);

  const fileInputLRef = useRef<HTMLInputElement>(null);
  const fileInputRRef = useRef<HTMLInputElement>(null);
  const wasLoadedRef = useRef(false);

  // Check if the calibration plugin is installed / loaded
  const checkPluginLoaded = useCallback(async () => {
    try {
      const plugins = await listPlugins();
      const calib = plugins.find(p => isCalibrationName(p.manifest.name));
      setPluginFound(!!calib);
      setPluginLoaded(
        calib ? calib.state === 'Loaded' || calib.state === 'Running' : false,
      );
    } catch {
      setPluginFound(false);
      setPluginLoaded(false);
    } finally {
      setChecking(false);
    }
  }, []);

  useEffect(() => {
    checkPluginLoaded();
    const unlisten = listen('plugins-changed', () => {
      checkPluginLoaded();
    });
    // Pure-vite (no Tauri backend) mode: listen rejects — degrade to no-op
    // so the panel still mounts for E2E rendering assertions.
    unlisten.catch(() => {});
    return () => {
      unlisten.then(fn => fn()).catch(() => {});
    };
  }, [checkPluginLoaded]);

  // Leaving the loaded state (plugin disabled or removed) must not leave
  // hidden DSP effects behind: bypass the native PEQ and the FIR plugin.
  useEffect(() => {
    if (pluginLoaded) {
      wasLoadedRef.current = true;
    } else if (wasLoadedRef.current && !checking) {
      wasLoadedRef.current = false;
      setFitResultL(null);
      setFitResultR(null);
      setCalibrationEnabled(false);
      bypassCalibration().catch(() => {});
    }
  }, [pluginLoaded, checking]);

  // Load current calibration state when the plugin appears
  useEffect(() => {
    if (!pluginLoaded) return;
    getCalibrationState()
      .then(state => {
        setCalibrationEnabled(state.peq.enabled && state.firConfig?.bypass !== true);
      })
      .catch(() => {});
  }, [pluginLoaded]);

  // Read the live output sample rate once (filters are designed for it)
  useEffect(() => {
    calibrationSampleRate().then(sr => setSampleRate(sr));
  }, []);

  // Fetch the display target curve whenever the preset changes
  useEffect(() => {
    if (!pluginLoaded) return;
    let cancelled = false;
    getTargetCurve(targetCurveId)
      .then(curve => {
        if (!cancelled) setTargetCurve(curve);
      })
      .catch(() => {
        if (!cancelled) setTargetCurve(null);
      });
    return () => {
      cancelled = true;
    };
  }, [targetCurveId, pluginLoaded]);

  const handleFileUpload = useCallback(
    (channel: 'L' | 'R') => (e: React.ChangeEvent<HTMLInputElement>) => {
      const file = e.target.files?.[0];
      if (!file) return;
      const reader = new FileReader();
      reader.onload = ev => {
        try {
          const curve = parseMeasurement(ev.target?.result as string);
          if (curve.frequencies.length > 0) {
            if (channel === 'L') setMeasuredL(curve);
            else setMeasuredR(curve);
            setError(null);
          } else {
            setError(t('calibration.invalidMeasurement'));
          }
        } catch {
          setError(t('calibration.fileParseError'));
        }
      };
      reader.readAsText(file);
      e.target.value = '';
    },
    [t],
  );

  const generateDemoMeasurement = useCallback(() => {
    // Synthetic measurement for demo purposes: bass roll-off, mid dip,
    // room-mode bump and treble roll-off.
    const n = 256;
    const freqs: number[] = [];
    for (let i = 0; i < n; i++) {
      freqs.push(20 * Math.pow(1000, i / (n - 1)));
    }
    const db = freqs.map(f => {
      const logF = Math.log10(f);
      let bass = 0;
      if (f < 80) bass = -10 * (1 - (logF - Math.log10(20)) / (Math.log10(80) - Math.log10(20)));
      let mid = 0;
      if (f >= 200 && f <= 1000) {
        const tt = (logF - Math.log10(200)) / (Math.log10(1000) - Math.log10(200));
        mid = -3 * Math.sin(tt * Math.PI);
      }
      let treble = 0;
      if (f > 5000) {
        treble = -4 * ((logF - Math.log10(5000)) / (Math.log10(20000) - Math.log10(5000)));
      }
      let room = 0;
      if (f >= 60 && f <= 120) {
        const tt = (logF - Math.log10(60)) / (Math.log10(120) - Math.log10(60));
        room = 4 * Math.sin(tt * Math.PI);
      }
      return bass + mid + treble + room + (Math.random() - 0.5) * 0.5;
    });
    setMeasuredL({ frequencies: freqs, db });
    if (linkLr) setMeasuredR(null);
    setError(null);
  }, [linkLr]);

  const handleRunFit = useCallback(async () => {
    if (!measuredL) {
      setError(t('calibration.noMeasurement'));
      return;
    }
    setFitting(true);
    setError(null);
    try {
      const buildRequest = (curve: MeasurementCurve): FitRequest => ({
        measurement: toMeasurementPairs(curve),
        target_curve_id: targetCurveId,
        custom_target: null,
        max_bands: Math.min(20, Math.max(12, maxBands)),
        fir_taps: Math.min(8192, Math.max(64, firTaps)),
        sample_rate: sampleRate,
      });
      const fitL = await runCalibrationFit(buildRequest(measuredL));
      // Separate R measurement: a second mono fit, its L-side output used
      // as the right-channel correction (the engine fits one channel at a time).
      const fitR = !linkLr && measuredR ? await runCalibrationFit(buildRequest(measuredR)) : null;
      setFitResultL(fitL);
      setFitResultR(fitR);
    } catch (e) {
      setError(String(e));
    } finally {
      setFitting(false);
    }
  }, [measuredL, measuredR, linkLr, targetCurveId, maxBands, firTaps, sampleRate, t]);

  const handleApply = useCallback(async () => {
    if (!fitResultL) return;
    setApplying(true);
    setError(null);
    try {
      // Separate-R mode: the right channel comes from the second fit's L side.
      const bandsR = fitResultR ? fitResultR.bands_L : fitResultL.bands_R;
      const firR = fitResultR ? fitResultR.fir_L : fitResultL.fir_R;
      await applyCalibrationResult(
        fitResultL.bands_L,
        bandsR,
        fitResultL.fir_L,
        firR,
        fitResultL.fit_id,
        sampleRate,
      );
      setCalibrationEnabled(true);
    } catch (e) {
      setError(String(e));
    } finally {
      setApplying(false);
    }
  }, [fitResultL, fitResultR, sampleRate]);

  const handleBypass = useCallback(async () => {
    setApplying(true);
    try {
      await bypassCalibration();
      setCalibrationEnabled(false);
    } catch (e) {
      setError(String(e));
    } finally {
      setApplying(false);
    }
  }, []);

  // Residual = fitted − target, both on the standard 256-pt grid
  const residual = fitResultL && targetCurve
    ? {
        frequencies: fitResultL.curves_L.freq,
        db: fitResultL.curves_L.mag_db.map((v, i) => v - (targetCurve.mag_db[i] ?? 0)),
      }
    : null;

  const fittedL = fitResultL
    ? { frequencies: fitResultL.curves_L.freq, db: fitResultL.curves_L.mag_db }
    : null;
  const targetData = targetCurve
    ? { frequencies: targetCurve.freq, db: targetCurve.mag_db }
    : { frequencies: [], db: [] };

  return (
    <div className="calibration-panel">
      <div className="panel-header">
        <h2>{t('calibration.title')}</h2>
        {pluginLoaded && (
          <div className="calibration-status">
            <span className={`status-dot ${calibrationEnabled ? 'active' : 'bypassed'}`} />
            <span>{calibrationEnabled ? t('calibration.active') : t('calibration.bypassed')}</span>
          </div>
        )}
      </div>

      {!pluginLoaded && !checking && (
        <div className="calibration-not-installed">
          <h3>
            {t(pluginFound ? 'calibration.notEnabled.title' : 'calibration.notInstalled.title')}
          </h3>
          <p className="hint">
            {t(
              pluginFound
                ? 'calibration.notEnabled.hint'
                : 'calibration.notInstalled.desc',
            )}
          </p>
        </div>
      )}

      {pluginLoaded && (
        <>
          {error && <div className="error-banner">{error}</div>}
          <div className="calibration-sections">
            {/* ① Measurement import */}
            <section className="calibration-section">
              <h3>{t('calibration.measurement')}</h3>
              <div className="section-row">
                <button onClick={() => fileInputLRef.current?.click()} className="btn">
                  {t('calibration.importMeasurement')}
                </button>
                <input
                  ref={fileInputLRef}
                  type="file"
                  accept=".txt,.csv,.frd,.json"
                  onChange={handleFileUpload('L')}
                  style={{ display: 'none' }}
                />
                <button
                  onClick={() => fileInputRRef.current?.click()}
                  className="btn"
                  disabled={linkLr}
                  style={linkLr ? { opacity: 0.5 } : undefined}
                >
                  {t('calibration.importMeasurementR')}
                </button>
                <input
                  ref={fileInputRRef}
                  type="file"
                  accept=".txt,.csv,.frd,.json"
                  onChange={handleFileUpload('R')}
                  style={{ display: 'none' }}
                />
                <button onClick={generateDemoMeasurement} className="btn btn-secondary">
                  {t('calibration.demoData')}
                </button>
                <span className="hint-text">
                  {measuredL
                    ? `${measuredL.frequencies.length} ${t('calibration.points')}`
                    : t('calibration.noMeasurementLoaded')}
                </span>
              </div>
              <div className="section-row">
                <label className="checkbox">
                  <input
                    type="checkbox"
                    checked={linkLr}
                    onChange={e => setLinkLr(e.target.checked)}
                  />
                  {t('calibration.linkLr')}
                </label>
              </div>
            </section>

            {/* ② Target curve */}
            <section className="calibration-section">
              <h3>{t('calibration.targetCurve')}</h3>
              <div className="section-row">
                <select
                  value={targetCurveId}
                  onChange={e => setTargetCurveId(e.target.value)}
                >
                  {TARGET_CURVES.map(preset => (
                    <option key={preset.id} value={preset.id}>
                      {t(`calibration.target.${preset.labelKey}`)}
                    </option>
                  ))}
                </select>
              </div>
            </section>

            {/* ③ Fit parameters */}
            <section className="calibration-section">
              <h3>{t('calibration.fitParams')}</h3>
              <div className="param-grid">
                <div className="param-item">
                  <label>{t('calibration.maxBands')}</label>
                  <input
                    type="number"
                    min={12}
                    max={20}
                    value={maxBands}
                    onChange={e => setMaxBands(parseInt(e.target.value) || 16)}
                  />
                </div>
                <div className="param-item">
                  <label>{t('calibration.firTaps')}</label>
                  <input
                    type="number"
                    min={64}
                    max={8192}
                    step={64}
                    value={firTaps}
                    onChange={e => setFirTaps(parseInt(e.target.value) || 2048)}
                  />
                </div>
                <div className="param-item">
                  <label>{t('calibration.sampleRate')}</label>
                  <span className="hint-text">{sampleRate} Hz</span>
                </div>
              </div>
            </section>

            {/* ④ Curve visualization */}
            <section className="calibration-section">
              <h3>{t('calibration.curves')}</h3>
              <CurveCanvas
                measured={measuredL}
                target={targetData}
                fitted={fittedL}
                residual={residual}
                width={720}
                height={320}
              />
            </section>

            {/* ⑤ Actions + diagnostics */}
            <section className="calibration-section">
              <div className="action-row">
                <button
                  onClick={handleRunFit}
                  disabled={fitting || !measuredL}
                  className="btn btn-primary"
                >
                  {fitting ? t('calibration.fitting') : t('calibration.runFit')}
                </button>
                {fitResultL && (
                  <>
                    <button
                      onClick={handleApply}
                      disabled={applying}
                      className="btn btn-success"
                    >
                      {applying ? t('calibration.applying') : t('calibration.apply')}
                    </button>
                    <button
                      onClick={handleBypass}
                      disabled={applying}
                      className="btn btn-secondary"
                    >
                      {t('calibration.bypass')}
                    </button>
                  </>
                )}
              </div>
              {fitResultL && (
                <div className="fit-metrics">
                  <div className="metric">
                    <span className="metric-label">{t('calibration.rmsError')}</span>
                    <span className="metric-value">{fitResultL.rms_error_db.toFixed(2)} dB</span>
                  </div>
                  <div className="metric">
                    <span className="metric-label">{t('calibration.peakError')}</span>
                    <span className="metric-value">{fitResultL.peak_error_db.toFixed(2)} dB</span>
                  </div>
                  <div className="metric">
                    <span className="metric-label">{t('calibration.bandsUsed')}</span>
                    <span className="metric-value">{fitResultL.bands_L.length}</span>
                  </div>
                  <div className="metric">
                    <span className="metric-label">{t('calibration.firTapsUsed')}</span>
                    <span className="metric-value">{fitResultL.fir_L.length}</span>
                  </div>
                </div>
              )}
            </section>
          </div>
        </>
      )}
    </div>
  );
};

export default CalibrationPanel;
