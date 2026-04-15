import { useState, useEffect, useRef, useCallback } from 'react'

// VITE_DAEMON_URL lets the dashboard point at a Railway-deployed daemon.
// Falls back to localhost for local dev (npm run dev).
const API = import.meta.env.VITE_DAEMON_URL || 'http://127.0.0.1:7878'

// ─── Helpers ──────────────────────────────────────────────────────────────────

function fmtUptime(secs) {
  if (secs < 60) return `${secs}s`
  if (secs < 3600) return `${Math.floor(secs / 60)}m ${secs % 60}s`
  const h = Math.floor(secs / 3600)
  const m = Math.floor((secs % 3600) / 60)
  return `${h}h ${m}m`
}

function fmtBytes(b) {
  if (b < 1024) return `${b} B`
  if (b < 1024 ** 2) return `${(b / 1024).toFixed(1)} KB`
  return `${(b / 1024 ** 2).toFixed(1)} MB`
}

function timeAgo(unix) {
  if (!unix) return '—'
  const diff = Math.floor(Date.now() / 1000) - unix
  if (diff < 60) return `${diff}s ago`
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`
  return `${Math.floor(diff / 86400)}d ago`
}

const DAEMON_KEY_STORAGE = 'ghost-daemon-key'

const CATEGORY_COLOR = {
  personal:  '#00e5ff',
  projects:  '#00ff88',
  code:      '#ffb020',
  style:     '#c678dd',
  social:    '#56b6c2',
  calendar:  '#ff3d6b',
}

async function apiFetch(path, opts = {}, token = null) {
  // Default to a 10s timeout so a hung daemon can't lock the UI forever.
  // Callers can pass their own signal via opts.signal to override.
  const signal = opts.signal ?? AbortSignal.timeout(10_000)
  const headers = { ...(opts.headers || {}) }
  if (token) headers['Authorization'] = `Bearer ${token}`
  const r = await fetch(`${API}${path}`, { ...opts, headers, signal })
  if (!r.ok) throw new Error(`${r.status} ${r.statusText}`)
  return r.json()
}

// ─── Styles ───────────────────────────────────────────────────────────────────

const CSS = `
  @import url('https://fonts.googleapis.com/css2?family=IBM+Plex+Mono:wght@400;500;600&family=Syne:wght@700;800&display=swap');

  *, *::before, *::after { box-sizing: border-box; margin: 0; padding: 0; }

  :root {
    --bg:       #0d0d0f;
    --surface:  #111318;
    --border:   #1e2330;
    --cyan:     #00e5ff;
    --green:    #00ff88;
    --red:      #ff3d6b;
    --amber:    #ffb020;
    --muted:    #4a5568;
    --text:     #c8d0e0;
    --mono:     'IBM Plex Mono', monospace;
    --display:  'Syne', sans-serif;
  }

  html, body, #root {
    height: 100%;
    background: var(--bg);
    color: var(--text);
    font-family: var(--mono);
    font-size: 13px;
    line-height: 1.5;
    overflow: hidden;
  }

  /* Scan line overlay */
  body::before {
    content: '';
    position: fixed;
    inset: 0;
    background: repeating-linear-gradient(
      to bottom,
      transparent,
      transparent 2px,
      rgba(0,0,0,0.04) 2px,
      rgba(0,0,0,0.04) 4px
    );
    pointer-events: none;
    z-index: 9999;
  }

  ::-webkit-scrollbar { width: 4px; }
  ::-webkit-scrollbar-track { background: transparent; }
  ::-webkit-scrollbar-thumb { background: var(--border); border-radius: 2px; }

  .session-row { transition: background 0.1s; cursor: default; }
  .session-row:hover { background: #161a24; }

  .pulse {
    animation: pulse 2s ease-in-out infinite;
  }
  @keyframes pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.3; }
  }

  .blink {
    animation: blink 1s step-end infinite;
  }
  @keyframes blink {
    0%, 100% { opacity: 1; }
    50% { opacity: 0; }
  }

  textarea {
    resize: none;
    font-family: var(--mono);
    font-size: 13px;
    background: #080a0d;
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 10px 12px;
    width: 100%;
    outline: none;
    transition: border-color 0.2s;
    line-height: 1.6;
  }
  textarea:focus { border-color: var(--cyan); }

  button {
    font-family: var(--mono);
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
    border: none;
    border-radius: 3px;
    padding: 8px 18px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    transition: all 0.15s;
  }

  input[type=text] {
    font-family: var(--mono);
    font-size: 13px;
    background: #080a0d;
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 8px 12px;
    outline: none;
    width: 100%;
    transition: border-color 0.2s;
  }
  input[type=text]:focus { border-color: var(--cyan); }
`

// ─── Sub-components ───────────────────────────────────────────────────────────

function Dot({ alive }) {
  return (
    <span style={{
      display: 'inline-block',
      width: 8, height: 8,
      borderRadius: '50%',
      background: alive ? 'var(--green)' : 'var(--red)',
      boxShadow: alive ? '0 0 8px var(--green)' : '0 0 8px var(--red)',
      flexShrink: 0,
    }} className={alive ? 'pulse' : ''} />
  )
}

function StatChip({ label, value, accent }) {
  return (
    <div style={{
      display: 'flex', flexDirection: 'column', gap: 2,
      padding: '6px 14px',
      borderLeft: '1px solid var(--border)',
    }}>
      <span style={{ color: 'var(--muted)', fontSize: 10, textTransform: 'uppercase', letterSpacing: '0.1em' }}>
        {label}
      </span>
      <span style={{ color: accent || 'var(--text)', fontWeight: 600, fontSize: 13 }}>
        {value ?? '—'}
      </span>
    </div>
  )
}

// ─── Main App ─────────────────────────────────────────────────────────────────

export default function App() {
  const [status, setStatus] = useState(null)
  const [alive, setAlive] = useState(false)
  const [sessions, setSessions] = useState([])
  const [prompt, setPrompt] = useState('')
  const [running, setRunning] = useState(false)
  const [chatMessages, setChatMessages] = useState([]) // [{role, content, job_id?}]
  const [memSearch, setMemSearch] = useState('')
  const [notes, setNotes] = useState(null)
  const [notesLoading, setNotesLoading] = useState(false)
  const [activeTab, setActiveTab] = useState('chat')
  const [lastPoll, setLastPoll] = useState(null)
  const [daemonKey, setDaemonKey] = useState(
    () => (typeof localStorage !== 'undefined' && localStorage.getItem(DAEMON_KEY_STORAGE)) || ''
  )

  useEffect(() => {
    if (typeof localStorage === 'undefined') return
    if (daemonKey) localStorage.setItem(DAEMON_KEY_STORAGE, daemonKey)
    else localStorage.removeItem(DAEMON_KEY_STORAGE)
  }, [daemonKey])
  const outputRef = useRef(null)
  const promptAbortRef = useRef(null)
  const mountedRef = useRef(true)

  useEffect(() => {
    mountedRef.current = true
    return () => {
      mountedRef.current = false
      if (promptAbortRef.current) promptAbortRef.current.abort()
    }
  }, [])

  const poll = useCallback(async () => {
    try {
      const [s, sess] = await Promise.all([
        apiFetch('/status', { signal: AbortSignal.timeout(5_000) }, daemonKey),
        apiFetch('/sessions', { signal: AbortSignal.timeout(5_000) }, daemonKey),
      ])
      if (!mountedRef.current) return
      setStatus(s)
      setSessions(sess.sessions || [])
      setAlive(true)
    } catch {
      if (!mountedRef.current) return
      setAlive(false)
      setStatus(null)
    }
    if (mountedRef.current) setLastPoll(new Date())
  }, [daemonKey])

  useEffect(() => {
    poll()
    const id = setInterval(poll, 10_000)
    return () => clearInterval(id)
  }, [poll])

  useEffect(() => {
    if (outputRef.current) {
      outputRef.current.scrollTop = outputRef.current.scrollHeight
    }
  }, [chatMessages])

  async function sendPrompt() {
    if (!prompt.trim() || running) return
    if (promptAbortRef.current) promptAbortRef.current.abort()
    const controller = new AbortController()
    promptAbortRef.current = controller

    const userMsg = prompt.trim()
    setPrompt('')
    setChatMessages(prev => [...prev, { role: 'user', content: userMsg }])
    setRunning(true)
    try {
      // Send last 6 messages (3 exchanges) as context, stripping job_id.
      const history = chatMessages.slice(-6).map(m => ({ role: m.role, content: m.content }))
      const data = await apiFetch('/chat', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ message: userMsg, history }),
        signal: controller.signal,
      }, daemonKey)
      if (!mountedRef.current) return
      setChatMessages(prev => [...prev, { role: 'assistant', content: data.response, job_id: data.job_id }])
    } catch (e) {
      if (e.name === 'AbortError' || !mountedRef.current) return
      setChatMessages(prev => [...prev, { role: 'error', content: e.message }])
    } finally {
      if (mountedRef.current) setRunning(false)
      if (promptAbortRef.current === controller) promptAbortRef.current = null
    }
  }

  function handleKeyDown(e) {
    if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) sendPrompt()
  }

  async function loadNotes() {
    setNotesLoading(true)
    try {
      const data = await apiFetch('/memories', {}, daemonKey)
      if (mountedRef.current) setNotes(data.notes || [])
    } catch (e) {
      if (mountedRef.current) setNotes([])
    } finally {
      if (mountedRef.current) setNotesLoading(false)
    }
  }

  async function deleteNote(id) {
    try {
      await apiFetch(`/memories/${id}`, { method: 'DELETE' }, daemonKey)
      setNotes(prev => prev ? prev.filter(n => n.id !== id) : prev)
    } catch {
      // ignore — row may already be gone
    }
  }

  useEffect(() => {
    if (activeTab === 'memory' && notes === null) loadNotes()
  }, [activeTab])

  // ── Layout ────────────────────────────────────────────────────────────────

  return (
    <>
      <style>{CSS}</style>

      <div style={{
        display: 'grid',
        gridTemplateRows: 'auto 1fr',
        height: '100vh',
        overflow: 'hidden',
      }}>

        {/* ── Top Bar ── */}
        <header style={{
          display: 'flex',
          alignItems: 'center',
          gap: 0,
          padding: '0 24px',
          height: 52,
          borderBottom: '1px solid var(--border)',
          background: 'var(--surface)',
          flexShrink: 0,
        }}>
          {/* Wordmark */}
          <div style={{
            fontFamily: 'var(--display)',
            fontSize: 15,
            fontWeight: 800,
            letterSpacing: '-0.02em',
            color: 'var(--cyan)',
            marginRight: 24,
            whiteSpace: 'nowrap',
          }}>
            GHOST
          </div>

          {/* Status */}
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginRight: 24 }}>
            <Dot alive={alive} />
            <span style={{ color: alive ? 'var(--green)' : 'var(--red)', fontWeight: 600, fontSize: 11, textTransform: 'uppercase', letterSpacing: '0.1em' }}>
              {alive ? 'online' : 'offline'}
            </span>
          </div>

          {/* Stats */}
          <div style={{ display: 'flex', flex: 1, overflow: 'hidden' }}>
            {status && <>
              <StatChip label="uptime" value={fmtUptime(status.uptime_secs)} accent="var(--cyan)" />
              <StatChip label="sessions" value={status.session_count} />
              <StatChip label="version" value={`v${status.version}`} />
              <StatChip label="pid" value={status.pid} />
            </>}
            {!status && !alive && (
              <div style={{ padding: '6px 14px', color: 'var(--muted)', borderLeft: '1px solid var(--border)' }}>
                daemon unreachable
              </div>
            )}
          </div>

          {/* Daemon key */}
          <div style={{ display: 'flex', alignItems: 'center', gap: 6, marginRight: 16 }}>
            <span style={{ color: 'var(--muted)', fontSize: 10, letterSpacing: '0.1em', textTransform: 'uppercase' }}>
              key
            </span>
            <input
              type="password"
              value={daemonKey}
              onChange={e => setDaemonKey(e.target.value)}
              placeholder={daemonKey ? '' : 'CLAW_DAEMON_KEY'}
              spellCheck={false}
              autoComplete="off"
              style={{
                fontFamily: 'var(--mono)',
                fontSize: 11,
                background: '#080a0d',
                color: 'var(--text)',
                border: '1px solid var(--border)',
                borderRadius: 3,
                padding: '4px 8px',
                width: 140,
                outline: 'none',
              }}
              title="Stored in localStorage. Sent as Authorization: Bearer <key> to the daemon."
            />
          </div>

          {/* Poll indicator */}
          <div style={{ color: 'var(--muted)', fontSize: 10, whiteSpace: 'nowrap' }}>
            {lastPoll && `polled ${timeAgo(Math.floor(lastPoll / 1000))}`}
          </div>
        </header>

        {/* ── Body ── */}
        <div style={{
          display: 'grid',
          gridTemplateColumns: '1fr 280px',
          overflow: 'hidden',
        }}>

          {/* ── Left: Main panel ── */}
          <div style={{
            display: 'flex',
            flexDirection: 'column',
            overflow: 'hidden',
            borderRight: '1px solid var(--border)',
          }}>
            {/* Tabs */}
            <div style={{
              display: 'flex',
              borderBottom: '1px solid var(--border)',
              background: 'var(--surface)',
              flexShrink: 0,
            }}>
              {['chat', 'memory'].map(tab => (
                <button
                  key={tab}
                  onClick={() => setActiveTab(tab)}
                  style={{
                    background: 'none',
                    color: activeTab === tab ? 'var(--cyan)' : 'var(--muted)',
                    borderBottom: activeTab === tab ? '2px solid var(--cyan)' : '2px solid transparent',
                    borderRadius: 0,
                    padding: '12px 20px',
                    fontSize: 11,
                    letterSpacing: '0.1em',
                  }}
                >
                  {tab.toUpperCase()}
                </button>
              ))}
            </div>

            {activeTab === 'chat' && (
              <div style={{ display: 'flex', flexDirection: 'column', flex: 1, overflow: 'hidden', padding: 20, gap: 16 }}>
                {/* Input area */}
                <div style={{ flexShrink: 0 }}>
                  <textarea
                    rows={4}
                    placeholder="Ask anything… (Ctrl+Enter to send)"
                    value={prompt}
                    onChange={e => setPrompt(e.target.value)}
                    onKeyDown={handleKeyDown}
                    disabled={running || !alive}
                  />
                  <div style={{ display: 'flex', justifyContent: 'flex-end', marginTop: 8, gap: 10, alignItems: 'center' }}>
                    {running && (
                      <span style={{ color: 'var(--muted)', fontSize: 11 }}>
                        running<span className="blink">_</span>
                      </span>
                    )}
                    <button
                      onClick={sendPrompt}
                      disabled={running || !alive || !prompt.trim()}
                      style={{
                        background: running || !alive ? 'var(--border)' : 'var(--cyan)',
                        color: running || !alive ? 'var(--muted)' : '#000',
                        cursor: running || !alive ? 'not-allowed' : 'pointer',
                      }}
                    >
                      {running ? 'running' : 'send'}
                    </button>
                  </div>
                </div>

                {/* Conversation thread */}
                <div
                  ref={outputRef}
                  style={{
                    flex: 1,
                    overflowY: 'auto',
                    background: '#080a0d',
                    border: '1px solid var(--border)',
                    borderRadius: 4,
                    padding: chatMessages.length ? '14px 16px' : 0,
                    position: 'relative',
                    display: 'flex',
                    flexDirection: 'column',
                    gap: 16,
                  }}
                >
                  {chatMessages.length === 0 && !running && (
                    <div style={{
                      position: 'absolute', inset: 0,
                      display: 'flex', alignItems: 'center', justifyContent: 'center',
                      color: 'var(--muted)', fontSize: 12,
                    }}>
                      output will appear here
                    </div>
                  )}
                  {chatMessages.map((msg, i) => (
                    <div key={i}>
                      <div style={{
                        fontSize: 10, letterSpacing: '0.1em', marginBottom: 4,
                        color: msg.role === 'user' ? 'var(--cyan)' : msg.role === 'error' ? 'var(--red)' : 'var(--green)',
                        display: 'flex', gap: 10, alignItems: 'center',
                      }}>
                        <span>{msg.role === 'user' ? 'you' : msg.role === 'error' ? 'error' : 'ghost'}</span>
                        {msg.job_id && (
                          <span style={{ color: 'var(--muted)', fontWeight: 400 }}>
                            · {msg.job_id.slice(0, 8)}
                          </span>
                        )}
                      </div>
                      <pre style={{
                        whiteSpace: 'pre-wrap', wordBreak: 'break-word',
                        color: msg.role === 'error' ? 'var(--red)' : 'var(--text)',
                        lineHeight: 1.7, margin: 0,
                      }}>
                        {msg.content}
                      </pre>
                    </div>
                  ))}
                  {running && (
                    <div style={{ color: 'var(--muted)', fontSize: 12 }}>
                      ghost<span className="blink">_</span>
                    </div>
                  )}
                </div>
                {/* Clear conversation */}
                {chatMessages.length > 0 && (
                  <div style={{ display: 'flex', justifyContent: 'flex-end' }}>
                    <button
                      onClick={() => setChatMessages([])}
                      style={{ background: 'none', color: 'var(--muted)', fontSize: 10, padding: '2px 6px' }}
                    >
                      clear
                    </button>
                  </div>
                )}
              </div>
            )}

            {activeTab === 'memory' && (
              <div style={{ display: 'flex', flexDirection: 'column', flex: 1, overflow: 'hidden', padding: 20, gap: 14 }}>
                {/* Toolbar */}
                <div style={{ display: 'flex', gap: 10, flexShrink: 0, alignItems: 'center' }}>
                  <input
                    type="text"
                    placeholder="Filter notes..."
                    value={memSearch}
                    onChange={e => setMemSearch(e.target.value)}
                    style={{ flex: 1 }}
                  />
                  <button
                    onClick={loadNotes}
                    disabled={notesLoading}
                    style={{
                      background: 'var(--border)',
                      color: 'var(--text)',
                      flexShrink: 0,
                      padding: '8px 14px',
                    }}
                  >
                    {notesLoading ? '...' : 'refresh'}
                  </button>
                </div>

                {/* Notes list */}
                <div style={{ flex: 1, overflowY: 'auto', display: 'flex', flexDirection: 'column', gap: 6 }}>
                  {notesLoading && notes === null && (
                    <div style={{ color: 'var(--muted)', fontSize: 12, padding: 8 }}>loading<span className="blink">_</span></div>
                  )}
                  {!notesLoading && notes !== null && notes.length === 0 && (
                    <div style={{ color: 'var(--muted)', fontSize: 12, padding: 8 }}>
                      no memories yet — GHOST learns as you chat
                    </div>
                  )}
                  {notes && notes
                    .filter(n => !memSearch || n.content.toLowerCase().includes(memSearch.toLowerCase()) || n.category.toLowerCase().includes(memSearch.toLowerCase()))
                    .map(note => (
                      <div key={note.id} style={{
                        display: 'flex', alignItems: 'flex-start', gap: 10,
                        background: '#080a0d',
                        border: '1px solid var(--border)',
                        borderRadius: 4,
                        padding: '10px 12px',
                      }}>
                        <span style={{
                          flexShrink: 0,
                          fontSize: 9,
                          fontWeight: 700,
                          letterSpacing: '0.08em',
                          textTransform: 'uppercase',
                          color: '#000',
                          background: CATEGORY_COLOR[note.category] || 'var(--muted)',
                          borderRadius: 3,
                          padding: '2px 6px',
                          marginTop: 1,
                        }}>
                          {note.category}
                        </span>
                        <span style={{ flex: 1, color: 'var(--text)', fontSize: 12, lineHeight: 1.6 }}>
                          {note.content}
                        </span>
                        <span style={{ flexShrink: 0, color: 'var(--muted)', fontSize: 10, marginTop: 2 }}>
                          {note.created_at?.slice(0, 10)}
                        </span>
                        <button
                          onClick={() => deleteNote(note.id)}
                          style={{
                            flexShrink: 0,
                            background: 'none',
                            color: 'var(--muted)',
                            padding: '0 4px',
                            fontSize: 14,
                            lineHeight: 1,
                          }}
                          title="Delete note"
                        >
                          ×
                        </button>
                      </div>
                    ))
                  }
                </div>
              </div>
            )}
          </div>

          {/* ── Right: Sessions ── */}
          <div style={{
            display: 'flex',
            flexDirection: 'column',
            overflow: 'hidden',
          }}>
            <div style={{
              padding: '12px 16px',
              borderBottom: '1px solid var(--border)',
              background: 'var(--surface)',
              fontSize: 11,
              color: 'var(--muted)',
              letterSpacing: '0.1em',
              textTransform: 'uppercase',
              flexShrink: 0,
            }}>
              Sessions ({sessions.length})
            </div>

            <div style={{ overflowY: 'auto', flex: 1 }}>
              {sessions.length === 0 && (
                <div style={{ padding: 20, color: 'var(--muted)', fontSize: 12 }}>
                  {alive ? 'no sessions found' : 'daemon offline'}
                </div>
              )}
              {sessions.map((s, i) => {
                const ws = s.workspace?.slice(0, 8) ?? '????????'
                const key = `${s.workspace ?? 'ws'}-${s.file ?? s.modified_unix ?? i}`
                return (
                  <div
                    key={key}
                    className="session-row"
                    style={{
                      padding: '10px 16px',
                      borderBottom: '1px solid var(--border)',
                    }}
                  >
                    <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                      <code style={{ color: 'var(--cyan)', fontSize: 12 }}>
                        {ws}
                      </code>
                      <span style={{ color: 'var(--muted)', fontSize: 10 }}>
                        {fmtBytes(s.bytes)}
                      </span>
                    </div>
                    <div style={{ color: 'var(--muted)', fontSize: 10, marginTop: 2 }}>
                      {timeAgo(s.modified_unix)}
                    </div>
                  </div>
                )
              })}
            </div>
          </div>

        </div>
      </div>
    </>
  )
}
