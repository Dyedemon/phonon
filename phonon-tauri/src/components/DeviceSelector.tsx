import { useState, useEffect, useCallback, useRef } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { setDevice } from '../api/devices'
import { useI18n } from '../i18n'

interface DeviceInfo {
  name: string
  id: string
  is_default: boolean
  sample_rates: number[]
  max_channels: number
  supports_exclusive: boolean
}

interface HotplugPayload {
  type: string
  event?: string
  device_name?: string
}

export default function DeviceSelector() {
  const { t } = useI18n()
  const [devices, setDevices] = useState<DeviceInfo[]>([])
  const devicesRef = useRef<DeviceInfo[]>([])
  const [current, setCurrent] = useState<string | null>(null)
  const [currentRate, setCurrentRate] = useState<number | null>(null)
  const [loading, setLoading] = useState(false)
  const mountedRef = useRef(true)

  // Cleanup on unmount
  useEffect(() => {
    mountedRef.current = true
    return () => { mountedRef.current = false }
  }, [])

  const refresh = useCallback(async () => {
    // Only show loading spinner on first load (no cached devices)
    if (devicesRef.current.length === 0) {
      setLoading(true)
    }
    try {
      const list = await invoke<DeviceInfo[]>('list_devices')
      if (!mountedRef.current) return
      setDevices(list)
      devicesRef.current = list
      const cur = await invoke<DeviceInfo | null>('get_current_device')
      if (!mountedRef.current) return
      setCurrent(cur?.name ?? null)
      if (cur && cur.sample_rates.length > 0) {
        setCurrentRate(cur.sample_rates[0])
      }
    } catch (_) { /* ignore */ } finally {
      if (mountedRef.current) setLoading(false)
    }
  }, [])

  useEffect(() => {
    refresh()
  }, [refresh])

  // Listen for hotplug events to auto-refresh the device list
  useEffect(() => {
    let unlistenFn: (() => void) | null = null
    const setup = async () => {
      try {
        unlistenFn = await listen<HotplugPayload>('app-event', (event) => {
          if (event.payload.type === 'DeviceHotplug') {
            refresh()
          }
        })
      } catch (_) { /* ignore */ }
    }
    setup()
    return () => {
      if (unlistenFn) unlistenFn()
    }
  }, [refresh])

  const selectDevice = async (name: string) => {
    try {
      await setDevice(name)
      // Refresh from backend to get accurate active device state
      await refresh()
    } catch (e) {
      console.error('Failed to set device:', e)
    }
  }

  return (
    <div className="card" style={{ cursor: loading ? 'wait' : 'default' }}>
      <div className="toolbar">
        <h3 style={{ flex: 1, margin: 0 }}>{t('devices.title', 'Audio Devices')}</h3>
        {currentRate && (
          <span style={{ fontSize: 'var(--text-sm)', color: 'var(--text-muted)', marginRight: 12 }}>
            {t('devices.sampleRate', 'Output')}: {(currentRate / 1000).toFixed(1)}kHz
          </span>
        )}
        <button
          className="btn btn-outline btn-sm"
          onClick={refresh}
          disabled={loading}
          style={{ cursor: loading ? 'wait' : 'pointer' }}
        >
          {loading ? t('devices.refreshing') : `\u21BB ${t('devices.refresh')}`}
        </button>
      </div>
      {loading && devices.length === 0 ? (
        <div className="empty-state">
          <div className="icon">{'\uD83D\uDD0A'}</div>
          <p>{t('devices.detecting')}</p>
        </div>
      ) : devices.length === 0 ? (
        <div className="empty-state">
          <div className="icon">{'\uD83D\uDD0A'}</div>
          <p>{t('devices.noDevice', 'No audio devices found')}</p>
        </div>
      ) : (
        <div>
          {devices.map((dev) => (
            <div
              key={dev.id}
              className="list-item"
              style={{
                cursor: 'pointer',
                border: current === dev.name ? '1px solid var(--accent)' : '1px solid transparent',
                borderRadius: 6,
                marginBottom: 4,
              }}
              onClick={() => selectDevice(dev.name)}
            >
              <div className="info">
                <div className="title">
                  {dev.is_default && <span style={{ color: 'var(--accent)', marginRight: 6 }}>*</span>}
                  {dev.name}
                  {dev.supports_exclusive && (
                    <span style={{ fontSize: 'var(--text-xs)', color: 'var(--success)', marginLeft: 8 }}>{t('devices.exclusive')}</span>
                  )}
                </div>
                <div className="subtitle">
                  {dev.max_channels}ch | {dev.sample_rates.length > 0 ? dev.sample_rates.map(r => `${(r/1000).toFixed(1)}kHz`).join(', ') : t('devices.defaultRates')}
                </div>
              </div>
              {current === dev.name && (
                <span style={{ color: 'var(--accent)', fontSize: 'var(--text-sm)', fontWeight: 600 }}>{t('devices.active')}</span>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  )
}