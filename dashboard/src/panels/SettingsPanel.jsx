import { useState, useEffect, useCallback, useMemo } from 'react'
import { API, STORAGE_KEY, apiFetch } from '../lib/api.js'

// Full Settings page. Reads / writes via:
//   GET  /settings                         -> { key: value, ... }
//   PUT  /settings/<key>   body { value }  -> one key at a time
//   GET  /code/health                      -> { kill_switch, budget_remaining_cents, daemon_alive }
//   GET  /code/spend                       -> { today_cents, cap_cents, ... }
//   GET  /code/templates                   -> [Template]
//   POST /code/templates/stamp             -> StampedOutput
//   GET  /code/index/stats                 -> IndexStoredStats
//   POST /code/index/rebuild               -> IndexStats

const SECTIONS = [
  { id: 'providers', label: 'Providers' },
  { id: 'coder',     label: 'Coder' },
  { id: 'daemon',    label: 'Daemon' },
  { id: 'templates', label: 'Templates' },
  { id: 'index',     label: 'Index' },
]

const PROVIDER_OPTIONS = ['default', 'anthropic', 'openrouter']
const AGENTS_FOR_OVERRIDE = ['coder', 'brainstorm', 'orchestrator', 'chat']
const DAEMON_URL_KEY = 'ghost-daemon-url'

// ---------- Small atoms ----------

function Toggle({ on, onChange, disabled }) {
  return (
    <div
      onClick={() => !disabled && onChange(!on)}
      style={{
        width: 36, height: 20,
        background: on ? 'var(--accent)' : 'var(--border)',
        borderRadius: 10,
        cursor: disabled ? 'default' : 'pointer',
        position: 'relative',
        transition: 'background var(--transition)',
        flexShrink: 0,
        opacity: disabled ? 0.5 : 1,
      }}
    >
      <div style={{
        width: 16, height: 16,
        borderRadius: '50%',
        background: '#fff',
        position: 'absolute',
        top: 2,
        left: on ? 18 : 2,
        transition: 'left var(--transition)',
      }} />
    </div>
  )
}

function Field({ label, description, children }) {
  return (
    <div style={{
      display: 'flex',
      justifyContent: 'space-between',
      alignItems: 'flex-start',
      gap: 16,
      padding: '12px 16px',
      background: 'var(--surface-2)',
      border: '1px solid var(--border)',
      borderRadius: 'var(--radius)',
    }}>
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ fontSize: 12, fontWeight: 500, color: 'var(--text)' }}>{label}</div>
        {description && (
          <div style={{ fontSize: 11, color: 'var(--text-dim)', marginTop: 2, lineHeight: 1.4 }}>
            {description}
          </div>
        )}
      </div>
      <div style={{ flexShrink: 0 }}>
        {children}
      </div>
    </div>
  )
}

function SectionHeader({ children }) {
  return (
    <div style={{
      fontSize: 10,
      fontWeight: 700,
      letterSpacing: '0.12em',
      color: 'var(--text-muted)',
      marginBottom: 10,
      marginTop: 4,
    }}>
      {children}
    </div>
  )
}

// ---------- Data hook ----------

function useSettings(daemonKey) {
  const [settings, setSettings] = useState({})
  const [loading, setLoading] = useState(true)
  const [err, setErr] = useState(null)

  const reload = useCallback(async () => {
    setLoading(true); setErr(null)
    try {
      const s = await apiFetch('/settings', {}, daemonKey)
      setSettings(s || {})
    } catch (e) {
      setErr(e.message)
    } finally {
      setLoading(false)
    }
  }, [daemonKey])

  useEffect(() => { reload() }, [reload])

  const put = useCallback(async (key, value) => {
    try {
      await apiFetch(`/settings/${encodeURIComponent(key)}`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ value }),
      }, daemonKey)
      setSettings(prev => ({ ...prev, [key]: value }))
    } catch (e) {
      alert(`save failed for ${key}: ${e.message}`)
    }
  }, [daemonKey])

  return { settings, loading, err, put, reload }
}

// ---------- Sections ----------

function ProvidersSection({ settings, put }) {
  const globalDefault = settings['provider.default'] || 'anthropic'
  const perAgent = settings['provider.per_agent'] || {}

  function setAgentOverride(agent, value) {
    const next = { ...perAgent }
    if (value === 'default') delete next[agent]
    else next[agent] = value
    put('provider.per_agent', next)
  }

  return (
    <>
      <SectionHeader>PROVIDERS</SectionHeader>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
        <Field
          label="Global default"
          description="Provider used when an agent has no override."
        >
          <div style={{ display: 'flex', gap: 6 }}>
            {['anthropic', 'openrouter'].map(opt => (
              <button
                key={opt}
                onClick={() => put('provider.default', opt)}
                style={{
                  padding: '5px 12px',
                  fontSize: 11,
                  fontFamily: 'var(--mono)',
                  background: globalDefault === opt ? 'var(--accent)' : 'transparent',
                  color: globalDefault === opt ? 'var(--bg)' : 'var(--text-muted)',
                  border: `1px solid ${globalDefault === opt ? 'var(--accent)' : 'var(--border)'}`,
                  borderRadius: 'var(--radius-sm)',
                  cursor: 'pointer',
                }}
              >
                {opt}
              </button>
            ))}
          </div>
        </Field>

        <div style={{
          background: 'var(--surface-2)',
          border: '1px solid var(--border)',
          borderRadius: 'var(--radius)',
          overflow: 'hidden',
        }}>
          <div style={{
            padding: '10px 16px',
            borderBottom: '1px solid var(--border)',
            fontSize: 12,
            fontWeight: 500,
            color: 'var(--text)',
          }}>
            Per-agent overrides
          </div>
          {AGENTS_FOR_OVERRIDE.map((agent, i) => {
            const current = perAgent[agent] ?? 'default'
            return (
              <div
                key={agent}
                style={{
                  display: 'flex',
                  justifyContent: 'space-between',
                  alignItems: 'center',
                  padding: '8px 16px',
                  borderTop: i > 0 ? '1px solid var(--border)' : 'none',
                  fontSize: 11,
                  fontFamily: 'var(--mono)',
                }}
              >
                <span style={{ color: 'var(--text-muted)' }}>{agent}</span>
                <select
                  value={current}
                  onChange={e => setAgentOverride(agent, e.target.value)}
                  style={{
                    fontFamily: 'var(--mono)',
                    fontSize: 11,
                    background: 'var(--bg)',
                    color: 'var(--text)',
                    border: '1px solid var(--border)',
                    borderRadius: 'var(--radius-sm)',
                    padding: '3px 6px',
                    outline: 'none',
                  }}
                >
                  {PROVIDER_OPTIONS.map(opt => (
                    <option key={opt} value={opt}>{opt}</option>
                  ))}
                </select>
              </div>
            )
          })}
        </div>
      </div>
    </>
  )
}

function CoderSection({ settings, put, daemonKey }) {
  const budget = settings['coder.budget_cents_per_day'] ?? 200
  const autoApply = !!settings['coder.auto_apply']
  const summarize = !!settings['coder.summarize_as_you_go']

  const [budgetInput, setBudgetInput] = useState(String(budget))
  useEffect(() => { setBudgetInput(String(budget)) }, [budget])

  const [health, setHealth] = useState(null)
  useEffect(() => {
    let cancel = false
    async function pull() {
      try {
        const h = await apiFetch('/code/health', {}, daemonKey)
        if (!cancel) setHealth(h)
      } catch { /* leave last known */ }
    }
    pull()
    const id = setInterval(pull, 15_000)
    return () => { cancel = true; clearInterval(id) }
  }, [daemonKey])

  function commitBudget() {
    const n = Number.parseInt(budgetInput, 10)
    if (!Number.isFinite(n) || n < 0) {
      setBudgetInput(String(budget))
      return
    }
    if (n !== budget) put('coder.budget_cents_per_day', n)
  }

  return (
    <>
      <SectionHeader>CODER</SectionHeader>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
        <Field
          label="Daily budget (cents)"
          description={`Current: $${(budget / 100).toFixed(2)}/day. Change applies immediately.`}
        >
          <input
            type="number"
            min={0}
            value={budgetInput}
            onChange={e => setBudgetInput(e.target.value)}
            onBlur={commitBudget}
            onKeyDown={e => { if (e.key === 'Enter') e.currentTarget.blur() }}
            style={{
              width: 90,
              fontFamily: 'var(--mono)',
              fontSize: 12,
              background: 'var(--bg)',
              color: 'var(--text)',
              border: '1px solid var(--border)',
              borderRadius: 'var(--radius-sm)',
              padding: '4px 8px',
              outline: 'none',
              textAlign: 'right',
            }}
          />
        </Field>

        <div>
          <Field
            label="Auto-apply diffs"
            description="When on, coder writes to disk immediately without approval."
          >
            <Toggle on={autoApply} onChange={v => put('coder.auto_apply', v)} />
          </Field>
          {autoApply && (
            <div style={{
              marginTop: 6,
              padding: '8px 12px',
              background: 'rgba(244,63,94,0.08)',
              border: '1px solid rgba(244,63,94,0.3)',
              borderRadius: 'var(--radius-sm)',
              fontSize: 11,
              color: '#f43f5e',
              fontFamily: 'var(--mono)',
            }}>
              ⚠ Auto-apply is on. Coder writes to disk immediately without approval.
              Make sure you're in a branch.
            </div>
          )}
        </div>

        <Field
          label="Summarize as you go"
          description="Condenses substantive exchanges, drops IT-help chatter."
        >
          <Toggle on={summarize} onChange={v => put('coder.summarize_as_you_go', v)} />
        </Field>

        <Field
          label="Kill switch"
          description={
            health?.kill_switch
              ? 'Coder is DISABLED. Set GHOST_CODING_AGENT=off (env) or coder.kill_switch=false (settings) then restart.'
              : 'Coder is enabled. Set GHOST_CODING_AGENT=off and restart the daemon to disable all coder agents.'
          }
        >
          <span style={{
            fontSize: 11,
            fontFamily: 'var(--mono)',
            fontWeight: 600,
            padding: '4px 10px',
            borderRadius: 12,
            background: health?.kill_switch ? 'rgba(244,63,94,0.12)' : 'rgba(74,222,128,0.12)',
            border: `1px solid ${health?.kill_switch ? 'rgba(244,63,94,0.4)' : 'rgba(74,222,128,0.4)'}`,
            color: health?.kill_switch ? '#f43f5e' : '#4ade80',
          }}>
            {health?.kill_switch ? 'OFF' : 'ON'}
          </span>
        </Field>
      </div>
    </>
  )
}

function DaemonSection({ daemonKey }) {
  const [daemonUrl, setDaemonUrl] = useState(() => {
    try { return localStorage.getItem(DAEMON_URL_KEY) || API } catch { return API }
  })
  const [keyInput, setKeyInput] = useState(daemonKey || '')
  const [alive, setAlive] = useState(null)
  const [coderAlive, setCoderAlive] = useState(null)

  useEffect(() => {
    let cancel = false
    async function pull() {
      try {
        const r = await fetch(`${API}/health`, { signal: AbortSignal.timeout(4_000) })
        if (!cancel) setAlive(r.ok)
      } catch { if (!cancel) setAlive(false) }
      try {
        const r = await fetch(`${API}/code/health`, { signal: AbortSignal.timeout(4_000) })
        if (!cancel) setCoderAlive(r.ok)
      } catch { if (!cancel) setCoderAlive(false) }
    }
    pull()
    const id = setInterval(pull, 10_000)
    return () => { cancel = true; clearInterval(id) }
  }, [])

  function saveKey() {
    try { localStorage.setItem(STORAGE_KEY, keyInput.trim()) } catch { /* ignore */ }
    alert('Key saved. Reload to take effect across all panels.')
  }

  function saveUrl() {
    try { localStorage.setItem(DAEMON_URL_KEY, daemonUrl.trim()) } catch { /* ignore */ }
    alert('Daemon URL saved. Reload to take effect.\n\nNote: build-time VITE_DAEMON_URL still wins on production builds.')
  }

  return (
    <>
      <SectionHeader>DAEMON</SectionHeader>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
        <Field label="Health" description={`Currently calling ${API}`}>
          <div style={{ display: 'flex', gap: 12, alignItems: 'center' }}>
            <span style={{ display: 'flex', alignItems: 'center', gap: 6, fontSize: 11, fontFamily: 'var(--mono)' }}>
              <span style={{
                width: 8, height: 8, borderRadius: '50%',
                background: alive === null ? 'var(--text-dim)' : alive ? '#4ade80' : '#f43f5e',
              }} />
              daemon
            </span>
            <span style={{ display: 'flex', alignItems: 'center', gap: 6, fontSize: 11, fontFamily: 'var(--mono)' }}>
              <span style={{
                width: 8, height: 8, borderRadius: '50%',
                background: coderAlive === null ? 'var(--text-dim)' : coderAlive ? '#4ade80' : '#f43f5e',
              }} />
              code
            </span>
          </div>
        </Field>

        <Field label="Bearer key" description="Sent on every request. Stored in browser localStorage.">
          <div style={{ display: 'flex', gap: 6 }}>
            <input
              type="password"
              value={keyInput}
              onChange={e => setKeyInput(e.target.value)}
              placeholder="GHOST_DAEMON_KEY"
              style={{
                width: 200,
                fontFamily: 'var(--mono)',
                fontSize: 12,
                background: 'var(--bg)',
                color: 'var(--text)',
                border: '1px solid var(--border)',
                borderRadius: 'var(--radius-sm)',
                padding: '4px 8px',
                outline: 'none',
              }}
            />
            <button
              onClick={saveKey}
              style={{
                padding: '4px 10px', fontSize: 11, fontFamily: 'var(--mono)',
                background: 'var(--accent)', color: 'var(--bg)',
                border: 'none', borderRadius: 'var(--radius-sm)', cursor: 'pointer',
              }}
            >SAVE</button>
          </div>
        </Field>

        <Field label="Daemon URL" description="Default http://127.0.0.1:7878. Reload required after saving.">
          <div style={{ display: 'flex', gap: 6 }}>
            <input
              type="text"
              value={daemonUrl}
              onChange={e => setDaemonUrl(e.target.value)}
              placeholder="http://127.0.0.1:7878"
              style={{
                width: 260,
                fontFamily: 'var(--mono)',
                fontSize: 12,
                background: 'var(--bg)',
                color: 'var(--text)',
                border: '1px solid var(--border)',
                borderRadius: 'var(--radius-sm)',
                padding: '4px 8px',
                outline: 'none',
              }}
            />
            <button
              onClick={saveUrl}
              style={{
                padding: '4px 10px', fontSize: 11, fontFamily: 'var(--mono)',
                background: 'var(--accent)', color: 'var(--bg)',
                border: 'none', borderRadius: 'var(--radius-sm)', cursor: 'pointer',
              }}
            >SAVE</button>
          </div>
        </Field>
      </div>
    </>
  )
}

function TemplatesSection({ daemonKey }) {
  const [list, setList] = useState([])
  const [err, setErr] = useState(null)
  const [modalFor, setModalFor] = useState(null) // template object
  const [placeholders, setPlaceholders] = useState({})
  const [stamped, setStamped] = useState(null)
  const [stamping, setStamping] = useState(false)

  useEffect(() => {
    apiFetch('/code/templates', {}, daemonKey)
      .then(rows => setList(Array.isArray(rows) ? rows : []))
      .catch(e => setErr(e.message))
  }, [daemonKey])

  function openTemplate(t) {
    setModalFor(t)
    const empty = {}
    for (const p of t.placeholders || []) empty[p.name] = ''
    setPlaceholders(empty)
    setStamped(null)
  }

  async function stamp() {
    if (!modalFor) return
    setStamping(true)
    try {
      const data = await apiFetch('/code/templates/stamp', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          template_name: modalFor.name,
          placeholders,
        }),
      }, daemonKey)
      setStamped(data)
    } catch (e) {
      alert(`stamp failed: ${e.message}`)
    } finally {
      setStamping(false)
    }
  }

  return (
    <>
      <SectionHeader>TEMPLATES</SectionHeader>
      {err && <div style={{ color: 'var(--red)', fontSize: 11, marginBottom: 8 }}>{err}</div>}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
        {list.length === 0 && !err && (
          <div style={{ color: 'var(--text-dim)', fontSize: 11, fontFamily: 'var(--mono)' }}>
            no templates registered
          </div>
        )}
        {list.map(t => (
          <div
            key={t.name}
            onClick={() => openTemplate(t)}
            style={{
              padding: '10px 14px',
              background: 'var(--surface-2)',
              border: '1px solid var(--border)',
              borderRadius: 'var(--radius)',
              cursor: 'pointer',
            }}
          >
            <div style={{ fontSize: 12, fontWeight: 500, color: 'var(--text)', fontFamily: 'var(--mono)' }}>
              {t.name}
            </div>
            {t.description && (
              <div style={{ fontSize: 11, color: 'var(--text-dim)', marginTop: 2 }}>{t.description}</div>
            )}
          </div>
        ))}
      </div>

      {modalFor && (
        <div
          onClick={() => setModalFor(null)}
          style={{
            position: 'fixed', inset: 0,
            background: 'rgba(0,0,0,0.5)',
            display: 'flex', alignItems: 'center', justifyContent: 'center',
            zIndex: 100,
          }}
        >
          <div
            onClick={e => e.stopPropagation()}
            style={{
              width: 'min(720px, 90vw)',
              maxHeight: '85vh',
              background: 'var(--surface)',
              border: '1px solid var(--border)',
              borderRadius: 'var(--radius)',
              display: 'flex', flexDirection: 'column',
              overflow: 'hidden',
            }}
          >
            <header style={{
              padding: '10px 16px',
              borderBottom: '1px solid var(--border)',
              display: 'flex',
              alignItems: 'center',
            }}>
              <span style={{ fontSize: 12, fontWeight: 600, fontFamily: 'var(--mono)', color: 'var(--text)' }}>
                {modalFor.name}
              </span>
              <span
                onClick={() => setModalFor(null)}
                style={{ marginLeft: 'auto', fontSize: 14, color: 'var(--text-dim)', cursor: 'pointer', padding: '0 4px' }}
              >
                x
              </span>
            </header>
            <div style={{ padding: 16, overflowY: 'auto', display: 'flex', flexDirection: 'column', gap: 10 }}>
              {(modalFor.placeholders || []).map(p => (
                <label key={p.name} style={{ display: 'flex', flexDirection: 'column', gap: 3 }}>
                  <span style={{ fontSize: 11, color: 'var(--text-muted)', fontFamily: 'var(--mono)' }}>
                    {p.name}
                    {p.description && <span style={{ color: 'var(--text-dim)', marginLeft: 8 }}>— {p.description}</span>}
                  </span>
                  <input
                    value={placeholders[p.name] ?? ''}
                    onChange={e => setPlaceholders(prev => ({ ...prev, [p.name]: e.target.value }))}
                    placeholder={p.example || ''}
                    style={{
                      fontFamily: 'var(--mono)',
                      fontSize: 12,
                      background: 'var(--bg)',
                      color: 'var(--text)',
                      border: '1px solid var(--border)',
                      borderRadius: 'var(--radius-sm)',
                      padding: '6px 10px',
                      outline: 'none',
                    }}
                  />
                </label>
              ))}

              <button
                onClick={stamp}
                disabled={stamping}
                style={{
                  alignSelf: 'flex-start',
                  padding: '6px 14px',
                  fontSize: 11,
                  fontFamily: 'var(--mono)',
                  fontWeight: 600,
                  background: stamping ? 'var(--border)' : 'var(--accent)',
                  color: stamping ? 'var(--text-dim)' : 'var(--bg)',
                  border: 'none',
                  borderRadius: 'var(--radius-sm)',
                  cursor: stamping ? 'default' : 'pointer',
                  marginTop: 4,
                }}
              >
                {stamping ? 'STAMPING...' : 'STAMP'}
              </button>

              {stamped && (
                <div style={{
                  marginTop: 8,
                  border: '1px solid var(--border)',
                  borderRadius: 'var(--radius-sm)',
                  overflow: 'hidden',
                }}>
                  <div style={{
                    padding: '6px 10px',
                    borderBottom: '1px solid var(--border)',
                    fontSize: 11,
                    fontFamily: 'var(--mono)',
                    color: 'var(--text-muted)',
                    display: 'flex',
                    alignItems: 'center',
                  }}>
                    <span>{stamped.path}</span>
                    <button
                      onClick={() => navigator.clipboard?.writeText(stamped.content ?? '')}
                      style={{
                        marginLeft: 'auto',
                        padding: '2px 8px',
                        fontSize: 10,
                        fontFamily: 'var(--mono)',
                        background: 'transparent',
                        color: 'var(--text-muted)',
                        border: '1px solid var(--border)',
                        borderRadius: 'var(--radius-sm)',
                        cursor: 'pointer',
                      }}
                    >copy</button>
                  </div>
                  <pre style={{
                    margin: 0,
                    padding: 10,
                    maxHeight: 300,
                    overflow: 'auto',
                    background: 'var(--bg)',
                    color: 'var(--text)',
                    fontSize: 11,
                    fontFamily: 'var(--mono)',
                    lineHeight: 1.5,
                    whiteSpace: 'pre',
                  }}>{stamped.content}</pre>
                </div>
              )}
              <div style={{ fontSize: 10, color: 'var(--text-dim)', fontFamily: 'var(--mono)' }}>
                Manual "Queue as diff" flow lands in a follow-up — for now, use the coder agent to apply stamped output.
              </div>
            </div>
          </div>
        </div>
      )}
    </>
  )
}

function IndexSection({ settings, put, daemonKey }) {
  const [stats, setStats] = useState(null)
  const [rebuilding, setRebuilding] = useState(false)
  const watcher = !!settings['coder.index_watcher_enabled']

  const pullStats = useCallback(async () => {
    try {
      const s = await apiFetch('/code/index/stats', {}, daemonKey)
      setStats(s)
    } catch { /* silent */ }
  }, [daemonKey])

  useEffect(() => { pullStats() }, [pullStats])

  async function rebuild() {
    setRebuilding(true)
    try {
      await apiFetch('/code/index/rebuild', { method: 'POST' }, daemonKey)
      await pullStats()
    } catch (e) {
      alert(`rebuild failed: ${e.message}`)
    } finally {
      setRebuilding(false)
    }
  }

  const totalFiles = stats?.total_files ?? stats?.files ?? '—'
  const totalBytes = stats?.total_bytes ?? stats?.bytes
  const lastIndexedRaw = stats?.last_indexed_at ?? stats?.most_recent_indexed_at
  const lastIndexed = lastIndexedRaw ? new Date(lastIndexedRaw).toLocaleString() : '—'

  return (
    <>
      <SectionHeader>INDEX</SectionHeader>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
        <Field
          label="File count"
          description={totalBytes ? `${(totalBytes / 1024).toFixed(0)} KB indexed` : undefined}
        >
          <span style={{ fontFamily: 'var(--mono)', fontSize: 12, color: 'var(--text)' }}>{totalFiles}</span>
        </Field>

        <Field label="Last indexed" description="Wall-clock time the index was most recently touched.">
          <span style={{ fontFamily: 'var(--mono)', fontSize: 11, color: 'var(--text-muted)' }}>{lastIndexed}</span>
        </Field>

        <Field label="Rebuild now" description="Walks the repo and re-embeds every indexable file. Blocks until done.">
          <button
            onClick={rebuild}
            disabled={rebuilding}
            style={{
              padding: '5px 14px',
              fontSize: 11,
              fontFamily: 'var(--mono)',
              fontWeight: 600,
              background: rebuilding ? 'var(--border)' : 'var(--accent)',
              color: rebuilding ? 'var(--text-dim)' : 'var(--bg)',
              border: 'none',
              borderRadius: 'var(--radius-sm)',
              cursor: rebuilding ? 'default' : 'pointer',
            }}
          >
            {rebuilding ? 'REBUILDING...' : 'REBUILD'}
          </button>
        </Field>

        <Field
          label="File watcher"
          description="Auto-re-index files as they change. Requires daemon restart to take effect."
        >
          <Toggle on={watcher} onChange={v => put('coder.index_watcher_enabled', v)} />
        </Field>
      </div>
    </>
  )
}

// ---------- Root ----------

export default function SettingsPanel({ daemonKey }) {
  const [active, setActive] = useState('providers')
  const { settings, loading, put } = useSettings(daemonKey)

  const body = useMemo(() => {
    switch (active) {
      case 'providers': return <ProvidersSection settings={settings} put={put} />
      case 'coder':     return <CoderSection settings={settings} put={put} daemonKey={daemonKey} />
      case 'daemon':    return <DaemonSection daemonKey={daemonKey} />
      case 'templates': return <TemplatesSection daemonKey={daemonKey} />
      case 'index':     return <IndexSection settings={settings} put={put} daemonKey={daemonKey} />
      default:          return null
    }
  }, [active, settings, put, daemonKey])

  return (
    <div style={{ flex: 1, display: 'flex', minHeight: 0, overflow: 'hidden' }}>
      {/* Section nav */}
      <aside style={{
        width: 160,
        flexShrink: 0,
        padding: '16px 0',
        borderRight: '1px solid var(--border)',
        background: 'var(--surface)',
      }}>
        {SECTIONS.map(s => {
          const isActive = s.id === active
          return (
            <div
              key={s.id}
              onClick={() => setActive(s.id)}
              style={{
                padding: '8px 16px',
                fontSize: 12,
                fontFamily: 'var(--mono)',
                color: isActive ? 'var(--accent)' : 'var(--text-muted)',
                background: isActive ? 'var(--accent-dim)' : 'transparent',
                cursor: 'pointer',
                borderLeft: `2px solid ${isActive ? 'var(--accent)' : 'transparent'}`,
              }}
            >
              {s.label}
            </div>
          )
        })}
      </aside>

      {/* Content */}
      <main style={{ flex: 1, overflowY: 'auto', padding: 24, minWidth: 0 }}>
        {loading && !Object.keys(settings).length ? (
          <div style={{ color: 'var(--text-dim)', fontSize: 12, fontFamily: 'var(--mono)' }}>loading...</div>
        ) : body}
      </main>
    </div>
  )
}
