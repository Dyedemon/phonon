import { useState, useEffect, useRef, useCallback } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-dialog'
import { useI18n } from '../i18n'

interface LyricWord {
  time_ms: number
  text: string
}

interface LyricLine {
  time_ms: number
  text: string
  words: LyricWord[]
}

type LyricData =
  | { type: 'None' }
  | { type: 'Unsynced'; text: string }
  | { type: 'Synced'; lines: LyricLine[] }

interface SearchResultItem {
  id: number
  track_name: string
  artist_name: string
  has_synced: boolean
}

interface LyricSourceEntry {
  name: string
  file_name: string
}

interface LyricSourceScript {
  name: string
  search(keyword: string): Promise<SearchResultItem[]>
  fetchLyrics(id: string): Promise<string>
}

interface Props {
  trackPath: string | null
  positionSecs: number
}

const AUTO_SYNC_DELAY_MS = 3000

export default function LyricsPanel({ trackPath, positionSecs }: Props) {
  const { t } = useI18n()
  const [collapsed, setCollapsed] = useState(false)
  const [lyricData, setLyricData] = useState<LyricData>({ type: 'None' })
  const [activeIndex, setActiveIndex] = useState(-1)
  const [customLrcPath, setCustomLrcPath] = useState<string | null>(null)
  const [searching, setSearching] = useState(false)
  const [searchError, setSearchError] = useState('')
  const [showCustomSearch, setShowCustomSearch] = useState(false)
  const [customKeyword, setCustomKeyword] = useState('')
  const [searchResults, setSearchResults] = useState<SearchResultItem[]>([])
  const [loadingId, setLoadingId] = useState<number | null>(null)
  const [lyricSources, setLyricSources] = useState<LyricSourceEntry[]>([])
  const [activeSource, setActiveSource] = useState<LyricSourceScript | null>(null)
  const [activeSourceName, setActiveSourceName] = useState('')
  const [sourceLoading, setSourceLoading] = useState(false)
  const containerRef = useRef<HTMLDivElement>(null)
  const userScrollingRef = useRef(false)
  const scrollTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const lineRefs = useRef<Map<number, HTMLDivElement>>(new Map())

  // Load lyrics from auto-detection or custom path
  const loadLyrics = (path: string) => {
    invoke<LyricData>('get_lyrics', { path })
      .then((data) => setLyricData(data))
      .catch(() => setLyricData({ type: 'None' }))
  }

  // Auto-detect when track changes
  useEffect(() => {
    setLyricData({ type: 'None' })
    setActiveIndex(-1)
    setCustomLrcPath(null)
    lineRefs.current.clear()

    if (!trackPath) return
    loadLyrics(trackPath)
  }, [trackPath])

  // Reload from custom lrc path
  useEffect(() => {
    if (!customLrcPath) return
    invoke<LyricData>('load_lrc_file', { path: customLrcPath })
      .then((data) => setLyricData(data))
      .catch(() => setLyricData({ type: 'None' }))
  }, [customLrcPath])

  // Handle user scroll: pause auto-sync, resume after 3 seconds idle
  const handleUserScroll = useCallback(() => {
    userScrollingRef.current = true

    if (scrollTimerRef.current) {
      clearTimeout(scrollTimerRef.current)
    }
    scrollTimerRef.current = setTimeout(() => {
      userScrollingRef.current = false
    }, AUTO_SYNC_DELAY_MS)
  }, [])

  useEffect(() => {
    return () => {
      if (scrollTimerRef.current) clearTimeout(scrollTimerRef.current)
    }
  }, [])

  // Auto-scroll to active line for synced lyrics (only when user is not scrolling)
  useEffect(() => {
    if (lyricData.type !== 'Synced') return
    const lines = lyricData.lines
    if (lines.length === 0) return

    const posMs = positionSecs * 1000
    let idx = -1
    for (let i = 0; i < lines.length; i++) {
      if (lines[i].time_ms <= posMs) {
        idx = i
      } else {
        break
      }
    }
    setActiveIndex(idx)

    // Only auto-scroll when user is not manually scrolling
    if (idx >= 0 && !userScrollingRef.current) {
      const lineEl = lineRefs.current.get(idx)
      const container = containerRef.current
      if (lineEl && container) {
        const containerRect = container.getBoundingClientRect()
        const lineRect = lineEl.getBoundingClientRect()
        const relativeTop = lineRect.top - containerRect.top + container.scrollTop
        const targetScroll = relativeTop - container.clientHeight / 2 + lineEl.offsetHeight / 2
        container.scrollTo({ top: Math.max(0, targetScroll), behavior: 'smooth' })
      }
    }
  }, [positionSecs, lyricData])

  // Open file picker to manually select an LRC file
  const handleLoadLrc = async () => {
    try {
      const selected = await open({
        multiple: false,
        filters: [{ name: 'Lyrics', extensions: ['lrc', 'txt'] }],
      })
      if (selected && typeof selected === 'string') {
        setCustomLrcPath(selected)
      }
    } catch {
      // User cancelled
    }
  }

  // Search lyrics online via lrclib.net
  const handleSearchOnline = async () => {
    if (!trackPath) return
    setSearching(true)
    setSearchError('')
    try {
      const meta = await invoke<{ artist?: string; title?: string }>('get_track_metadata', { path: trackPath })
      console.log('search_lyrics: meta =', meta)
      if (!meta.artist || !meta.title) {
        setSearchError(t('lyrics.searchNoMeta'))
        return
      }
      console.log('search_lyrics: searching for', meta.artist, '-', meta.title)
      const data = await invoke<LyricData>('search_lyrics', { artist: meta.artist, title: meta.title })
      console.log('search_lyrics: result type =', data.type)
      setLyricData(data)
      if (data.type === 'None') {
        setSearchError(t('lyrics.searchNoResult'))
      }
    } catch (e) {
      console.error('search_lyrics failed:', e)
      setSearchError(t('lyrics.searchFailed') + ': ' + String(e))
    } finally {
      setSearching(false)
    }
  }

  // Custom keyword search — returns a list of results
  const handleCustomSearch = async () => {
    const keyword = customKeyword.trim()
    if (!keyword) return
    setSearching(true)
    setSearchError('')
    setSearchResults([])
    try {
      const results = await invoke<SearchResultItem[]>('search_lyrics_by_keyword', { keyword })
      console.log('custom_search: got', results.length, 'results')
      setSearchResults(results)
      if (results.length === 0) {
        setSearchError(t('lyrics.searchNoResult'))
      }
    } catch (e) {
      console.error('custom_search failed:', e)
      setSearchError(t('lyrics.searchFailed') + ': ' + String(e))
    } finally {
      setSearching(false)
    }
  }

  // Select a specific search result and load its lyrics
  const handleSelectSearchResult = async (item: SearchResultItem) => {
    setLoadingId(item.id)
    setSearchError('')
    try {
      const data = await invoke<LyricData>('fetch_lyrics_by_id', { trackId: item.id })
      console.log('fetch_lyrics_by_id: result type =', data.type)
      setLyricData(data)
      setSearchResults([])
      if (data.type === 'None') {
        setSearchError(t('lyrics.searchNoResult'))
      }
    } catch (e) {
      console.error('fetch_lyrics_by_id failed:', e)
      setSearchError(t('lyrics.searchFailed') + ': ' + String(e))
    } finally {
      setLoadingId(null)
    }
  }

  // ── Lyric Source Scripts ──────────────────────────────────

  // Scan for available lyric source scripts
  const loadLyricSources = async () => {
    try {
      const sources = await invoke<LyricSourceEntry[]>('scan_lyric_sources')
      setLyricSources(sources)
    } catch { /* ignore */ }
  }

  // Load and execute a specific lyric source script
  const loadScript = async (entry: LyricSourceEntry) => {
    setSourceLoading(true)
    setActiveSource(null)
    setActiveSourceName('')
    try {
      const content = await invoke<string>('read_lyric_script', { fileName: entry.file_name })
      // Create a fetch_url helper that calls the Rust backend
      const fetchUrlImpl = async (url: string, method?: string, headers?: string, body?: string) => {
        return invoke<string>('fetch_url', { url, method, headers, body })
      }
      // Execute the script in a sandboxed function
      const scriptFn = new Function('fetch_url', content)
      const source: LyricSourceScript = await scriptFn(fetchUrlImpl)
      if (!source || typeof source.search !== 'function' || typeof source.fetchLyrics !== 'function') {
        throw new Error('Script must export { name, search, fetchLyrics }')
      }
      setActiveSource(source)
      setActiveSourceName(source.name || entry.name)
      console.log('lyric_source: loaded', source.name)
    } catch (e) {
      console.error('lyric_source: failed to load script:', e)
      setSearchError(t('lyrics.scriptLoadFailed') + ': ' + String(e))
    } finally {
      setSourceLoading(false)
    }
  }

  // Search using the active script source
  const handleScriptSearch = async () => {
    if (!activeSource) return
    const keyword = customKeyword.trim()
    if (!keyword) return
    setSearching(true)
    setSearchError('')
    setSearchResults([])
    try {
      const results = await activeSource.search(keyword)
      console.log('script_search: got', results.length, 'results')
      setSearchResults(results)
      if (results.length === 0) {
        setSearchError(t('lyrics.searchNoResult'))
      }
    } catch (e) {
      console.error('script_search failed:', e)
      setSearchError(t('lyrics.searchFailed') + ': ' + String(e))
    } finally {
      setSearching(false)
    }
  }

  // Fetch lyrics using the active script source
  const handleScriptFetchLyrics = async (item: SearchResultItem) => {
    if (!activeSource) return
    setLoadingId(item.id)
    setSearchError('')
    try {
      const lrcText = await activeSource.fetchLyrics(String(item.id))
      if (!lrcText || lrcText.trim().length === 0) {
        setSearchError(t('lyrics.searchNoResult'))
        return
      }
      // Parse LRC text on the frontend side and convert to LyricData
      const data = await invoke<LyricData>('parse_lrc_text', { lrcText })
      console.log('script_fetch: result type =', data.type)
      setLyricData(data)
      setSearchResults([])
      if (data.type === 'None') {
        setSearchError(t('lyrics.searchNoResult'))
      }
    } catch (e) {
      console.error('script_fetch failed:', e)
      setSearchError(t('lyrics.searchFailed') + ': ' + String(e))
    } finally {
      setLoadingId(null)
    }
  }

  // Load sources on mount
  useEffect(() => {
    loadLyricSources()
  }, [])

  const setLineRef = (i: number) => (el: HTMLDivElement | null) => {
    if (el) {
      lineRefs.current.set(i, el)
    } else {
      lineRefs.current.delete(i)
    }
  }

  const hasLyrics = lyricData.type !== 'None'

  return (
    <div className="card" style={{ marginTop: 14 }}>
      <div
        className="collapsible-header"
        onClick={() => setCollapsed(!collapsed)}
        style={{ cursor: 'pointer', userSelect: 'none', display: 'flex', alignItems: 'center', gap: 8 }}
      >
        <span className={`collapsible-arrow ${collapsed ? '' : 'open'}`}>
          {collapsed ? '\u25B6' : '\u25BC'}
        </span>
        <h3 style={{ margin: 0, display: 'inline', flex: 1 }}>{t('lyrics.title')}</h3>
        {trackPath && hasLyrics && (
          <button
            className="btn btn-secondary"
            style={{ fontSize: 'var(--text-xs)', padding: '3px 10px' }}
            onClick={(e) => {
              e.stopPropagation()
              handleSearchOnline()
            }}
            disabled={searching}
          >
            {searching ? '...' : t('lyrics.searchOnline')}
          </button>
        )}
      </div>

      {!collapsed && (
        <div style={{ marginTop: 10 }}>
          {hasLyrics ? (
            <div
              ref={containerRef}
              className="lyrics-container"
              onWheel={handleUserScroll}
              onScroll={handleUserScroll}
              onTouchMove={handleUserScroll}
              style={{
                maxHeight: lyricData.type === 'Synced' ? 280 : 200,
                overflowY: 'auto',
                overflowX: 'hidden',
                padding: '8px 0',
                fontFamily: 'var(--lyrics)',
                fontSize: 'var(--text-base)',
                lineHeight: 2.0,
                color: 'var(--text-dim)',
                textAlign: 'center',
              }}
            >
              {lyricData.type === 'Unsynced' && (
                <div style={{ whiteSpace: 'pre-wrap', padding: '0 16px' }}>
                  {lyricData.text}
                </div>
              )}

              {lyricData.type === 'Synced' && lyricData.lines.map((line, i) => {
                const isActive = i === activeIndex
                const words = line.words
                const hasWords = words && words.length > 0
                const posMs = positionSecs * 1000

                // Calculate gradient sweep progress for the active line
                const lineGradientStyle: React.CSSProperties = {}
                if (isActive && hasWords && words.length > 1) {
                  const firstMs = words[0].time_ms
                  const lastMs = words[words.length - 1].time_ms
                  const lineDuration = lastMs - firstMs || 1
                  const lineProgress = Math.max(0, Math.min(1, (posMs - firstMs) / lineDuration))
                  const pct = lineProgress * 100
                  const softStart = Math.max(0, pct - 8)
                  const softEnd = Math.min(100, pct + 8)
                  lineGradientStyle.backgroundImage = `linear-gradient(to right, var(--accent) ${softStart}%, var(--accent) ${pct}%, var(--text-dim) ${softEnd}%, var(--text-dim) 100%)`
                  lineGradientStyle.backgroundClip = 'text'
                  lineGradientStyle.WebkitBackgroundClip = 'text'
                  lineGradientStyle.WebkitTextFillColor = 'transparent'
                }

                return (
                <div
                  key={i}
                  ref={setLineRef(i)}
                  className="lyric-line"
                  style={{
                    padding: '2px 16px',
                    fontWeight: isActive ? 600 : 400,
                    fontSize: isActive ? 14 : 13,
                    transition: 'font-size 0.3s',
                  }}
                >
                  {hasWords ? (
                    isActive && words.length > 1 ? (
                      <span style={lineGradientStyle}>
                        {words.map((w, j) => (
                          <span key={j}>{w.text}</span>
                        ))}
                      </span>
                    ) : (
                      words.map((w, j) => {
                        const highlighted = isActive && w.time_ms <= posMs
                        return (
                          <span
                            key={j}
                            style={{
                              color: highlighted ? 'var(--accent)' : 'var(--text-dim)',
                              fontWeight: highlighted ? 600 : 400,
                              transition: 'color 0.1s ease',
                            }}
                          >
                            {w.text}
                          </span>
                        )
                      })
                    )
                  ) : (
                    <span style={{
                      color: isActive ? 'var(--accent)' : 'var(--text-dim)',
                      transition: 'color 0.15s',
                    }}>
                      {line.text}
                    </span>
                  )}
                </div>
                )
              })}
            </div>
          ) : (
            <div
              style={{
                textAlign: 'center',
                padding: '32px 16px',
                color: 'var(--text-dim)',
              }}
            >
              <div style={{ fontSize: '38px', marginBottom: 8, opacity: 0.3 }}>
                {'\u266A'}
              </div>
              <p style={{ margin: '0 0 12px 0' }}>{t('lyrics.noLyrics')}</p>
              <div style={{ display: 'flex', gap: 8, justifyContent: 'center', flexWrap: 'wrap' }}>
                <button
                  className="btn btn-secondary"
                  onClick={(e) => {
                    e.stopPropagation()
                    handleLoadLrc()
                  }}
                >
                  {t('lyrics.loadLrc')}
                </button>
                {trackPath && (
                  <button
                    className="btn btn-secondary"
                    onClick={(e) => {
                      e.stopPropagation()
                      handleSearchOnline()
                    }}
                    disabled={searching}
                  >
                    {searching ? '...' : t('lyrics.searchOnline')}
                  </button>
                )}
              </div>
              <div style={{ marginTop: 10 }}>
                <button
                  className="btn btn-link"
                  onClick={(e) => {
                    e.stopPropagation()
                    setShowCustomSearch(!showCustomSearch)
                  }}
                  style={{ fontSize: 'var(--text-xs)', opacity: 0.6, cursor: 'pointer', background: 'none', border: 'none', color: 'var(--text-dim)' }}
                >
                  {showCustomSearch ? t('lyrics.hideCustom') : t('lyrics.customSearch')}
                </button>
                {showCustomSearch && (
                  <div style={{ marginTop: 8, display: 'flex', flexDirection: 'column', gap: 6, alignItems: 'center' }}>
                    {/* Source selector */}
                    {lyricSources.length > 0 && (
                      <div style={{ display: 'flex', gap: 4, alignItems: 'center', width: 220 }}>
                        <select
                          value={activeSource ? activeSourceName : ''}
                          onChange={(e) => {
                            const selected = lyricSources.find(s => s.name === e.target.value)
                            if (selected) {
                              loadScript(selected)
                            } else {
                              setActiveSource(null)
                              setActiveSourceName('')
                            }
                          }}
                          style={{
                            flex: 1, padding: '4px 6px', fontSize: 'var(--text-xs)',
                            borderRadius: 4, border: '1px solid var(--border)',
                            background: 'var(--input-bg)', color: 'var(--text)',
                            cursor: 'pointer',
                          }}
                          onClick={(e) => e.stopPropagation()}
                        >
                          <option value="">{t('lyrics.builtinSource')}</option>
                          {lyricSources.map(s => (
                            <option key={s.name} value={s.name}>{s.name}</option>
                          ))}
                        </select>
                        {sourceLoading && <span style={{ fontSize: 'var(--text-xs)', color: 'var(--text-muted)' }}>...</span>}
                      </div>
                    )}
                    <input
                      value={customKeyword}
                      onChange={(e) => setCustomKeyword(e.target.value)}
                      placeholder={t('lyrics.searchPlaceholder')}
                      style={{
                        width: 220, padding: '4px 8px', fontSize: 'var(--text-sm)',
                        borderRadius: 4, border: '1px solid var(--border)',
                        background: 'var(--input-bg)', color: 'var(--text)',
                      }}
                      onClick={(e) => e.stopPropagation()}
                      onKeyDown={(e) => {
                        if (e.key === 'Enter') {
                          activeSource ? handleScriptSearch() : handleCustomSearch()
                        }
                      }}
                    />
                    <button
                      className="btn btn-secondary"
                      onClick={(e) => {
                        e.stopPropagation()
                        activeSource ? handleScriptSearch() : handleCustomSearch()
                      }}
                      disabled={searching || !customKeyword.trim()}
                      style={{ fontSize: 'var(--text-xs)', padding: '4px 14px' }}
                    >
                      {searching ? '...' : t('lyrics.search')}
                    </button>
                  </div>
                )}
              </div>
              {searchResults.length > 0 && (
                <div style={{ marginTop: 10, textAlign: 'left' }}>
                  <div style={{
                    fontSize: 'var(--text-xs)', color: 'var(--text-muted)', marginBottom: 4,
                    padding: '0 4px',
                  }}>
                    {t('lyrics.searchResults')} ({searchResults.length})
                  </div>
                  <div style={{
                    maxHeight: 160, overflowY: 'auto',
                    border: '1px solid var(--border)',
                    borderRadius: 4,
                  }}>
                    {searchResults.map((item) => (
                      <div
                        key={item.id}
                        onClick={() => activeSource ? handleScriptFetchLyrics(item) : handleSelectSearchResult(item)}
                        style={{
                          padding: '6px 10px',
                          cursor: 'pointer',
                          borderBottom: '1px solid var(--border)',
                          fontSize: 'var(--text-sm)',
                          display: 'flex',
                          alignItems: 'center',
                          gap: 6,
                          background: loadingId === item.id ? 'var(--bg-hover)' : 'transparent',
                          transition: 'background 0.15s',
                        }}
                        onMouseEnter={(e) => {
                          if (loadingId !== item.id) (e.target as HTMLElement).style.background = 'var(--bg-hover)'
                        }}
                        onMouseLeave={(e) => {
                          if (loadingId !== item.id) (e.target as HTMLElement).style.background = 'transparent'
                        }}
                      >
                        <span style={{
                          color: item.has_synced ? 'var(--accent)' : 'var(--text-muted)',
                          fontSize: 'var(--text-xs)',
                          flexShrink: 0,
                        }}>
                          {item.has_synced ? '\u266B' : '\u266A'}
                        </span>
                        <span style={{ flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                          <span style={{ color: 'var(--text)' }}>{item.track_name}</span>
                          <span style={{ color: 'var(--text-muted)', marginLeft: 6 }}>{item.artist_name}</span>
                        </span>
                        {loadingId === item.id && (
                          <span style={{ fontSize: 'var(--text-xs)', color: 'var(--text-muted)', flexShrink: 0 }}>...</span>
                        )}
                      </div>
                    ))}
                  </div>
                </div>
              )}
              {searchError && (
                <p style={{
                  fontSize: 'var(--text-xs)', marginTop: 8, color: '#f44',
                  background: 'rgba(255,68,68,0.1)', padding: '4px 10px',
                  borderRadius: 4, wordBreak: 'break-all',
                }}>{searchError}</p>
              )}
              {customLrcPath && (
                <p style={{ fontSize: 'var(--text-xs)', marginTop: 8, opacity: 0.5, wordBreak: 'break-all' }}>
                  {customLrcPath}
                </p>
              )}
            </div>
          )}
        </div>
      )}
    </div>
  )
}