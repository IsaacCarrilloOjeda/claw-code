import { useState, useEffect, useRef, useMemo, useCallback } from 'react'
import ReactMarkdown from 'react-markdown'
import { apiFetch, uid } from '../lib/api.js'
import DiffReview from '../components/DiffReview.jsx'
import TokenMeter, { BudgetBar } from '../components/TokenMeter.jsx'

// CoderPanel — three chat kinds (brainstorm / coder / orchestrator) live in
// one sidebar with a file-index search. Send routing picks the endpoint based
// on the active chat's `agent_kind`. Polls /code/pending_diffs every 5s while
// mounted. Live token meter + budget bar added in D.4.

const CODER_CHATS_KEY = 'ghost-coder-chats'

const KIND_META = {
  brainstorm:   { label: 'Brainstorm',   color: '#a78bfa', icon: '🧠', endpoint: '/code/brainstorm' },
  coder:        { label: 'Coder',        color: '#4ade80', icon: '$',           endpoint: '/code/chat' },
  orchestrator: { label: 'Orchestrator', color: '#fb923c', icon: '$$',          endpoint: '/code/orchestrate' },
}

function loadCoderChats() {
  try {
    const raw = localStorage.getItem(CODER_CHATS_KEY)
    if (raw) return JSON.parse(raw)
  } catch { /* ignore */ }
  return []
}

function saveCoderChats(chats) {
  try { localStorage.setItem(CODER_CHATS_KEY, JSON.stringify(chats)) } catch { /* ignore */ }
}

// ---------- Sidebar pieces ----------

function KindGroup({ title, color, chats, activeChatId, onOpen, onDelete, open, onToggleOpen }) {
  return (
    <div style={{ marginBottom: 6 }}>
      <div
        onClick={onToggleOpen}
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 6,
          padding: '4px 12px',
          cursor: 'pointer',
          fontSize: 10,
          letterSpacing: '0.08em',
          textTransform: 'uppercase',
          color: 'var(--text-dim)',
        }}
      >
        <span style={{ fontSize: 8 }}>{open ? '▼' : '▶'}</span>
        <span style={{ color, fontWeight: 600 }}>{title}</span>
        <span style={{ marginLeft: 'auto', color: 'var(--text-dim)' }}>{chats.length}</span>
      </div>
      {open && chats.map(c => (
        <div
          key={c.id}
          onClick={() => onOpen(c.id)}
          style={{
            display: 'flex',
            alignItems: 'center',
            padding: '4px 12px 4px 24px',
            cursor: 'pointer',
            gap: 6,
            fontSize: 11,
            color: c.id === activeChatId ? 'var(--accent)' : 'var(--text-muted)',
            background: c.id === activeChatId ? 'var(--accent-dim)' : 'transparent',
            borderRadius: 'var(--radius-sm)',
            margin: '0 4px',
          }}
        >
          <span style={{
            flex: 1,
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap',
          }}>
            {c.name}
          </span>
          <span
            onClick={e => { e.stopPropagation(); onDelete(c.id) }}
            style={{ color: 'var(--text-dim)', fontSize: 11, cursor: 'pointer', padding: '0 2px' }}
            onMouseEnter={e => e.currentTarget.style.color = 'var(--red)'}
            onMouseLeave={e => e.currentTarget.style.color = 'var(--text-dim)'}
            title="delete"
          >
            x
          </span>
        </div>
      ))}
    </div>
  )
}

function NewThread({ onCreate }) {
  const [open, setOpen] = useState(false)
  return (
    <div style={{ padding: '8px 12px', borderBottom: '1px solid var(--border)' }}>
      <div
        onClick={() => setOpen(o => !o)}
        style={{
          fontSize: 11,
          color: 'var(--text-muted)',
          cursor: 'pointer',
          display: 'flex',
          alignItems: 'center',
          gap: 6,
          padding: '4px 0',
        }}
      >
        <span style={{ fontSize: 14, lineHeight: 1 }}>+</span>
        <span>new thread</span>
      </div>
      {open && (
        <div style={{ display: 'flex', gap: 4, marginTop: 6 }}>
          {Object.entries(KIND_META).map(([k, m]) => (
            <button
              key={k}
              onClick={() => { onCreate(k); setOpen(false) }}
              title={m.label}
              style={{
                flex: 1,
                padding: '6px 0',
                fontSize: 11,
                fontFamily: 'var(--mono)',
                background: 'transparent',
                color: m.color,
                border: `1px solid ${m.color}55`,
                borderRadius: 'var(--radius-sm)',
                cursor: 'pointer',
              }}
            >
              {m.icon}
            </button>
          ))}
        </div>
      )}
    </div>
  )
}

function FileIndexSearch({ daemonKey }) {
  const [q, setQ] = useState('')
  const [hits, setHits] = useState([])
  const [loading, setLoading] = useState(false)
  const [err, setErr] = useState(null)

  async function runSearch() {
    const query = q.trim()
    if (!query) return
    setLoading(true); setErr(null)
    try {
      const qs = new URLSearchParams({ q: query, k: '8' })
      const data = await apiFetch(`/code/files/search?${qs.toString()}`, {}, daemonKey)
      setHits(data.hits || [])
    } catch (e) {
      setErr(e.message)
      setHits([])
    } finally {
      setLoading(false)
    }
  }

  return (
    <div style={{ padding: '8px 12px', borderBottom: '1px solid var(--border)' }}>
      <div style={{ fontSize: 10, color: 'var(--text-dim)', letterSpacing: '0.08em', marginBottom: 4 }}>
        FILE INDEX
      </div>
      <div style={{ display: 'flex', gap: 4 }}>
        <input
          value={q}
          onChange={e => setQ(e.target.value)}
          onKeyDown={e => { if (e.key === 'Enter') runSearch() }}
          placeholder="search files..."
          style={{
            flex: 1,
            fontFamily: 'var(--mono)',
            fontSize: 11,
            background: 'var(--bg)',
            color: 'var(--text)',
            border: '1px solid var(--border)',
            borderRadius: 'var(--radius-sm)',
            padding: '4px 8px',
            outline: 'none',
          }}
        />
      </div>
      {loading && <div style={{ fontSize: 10, color: 'var(--text-dim)', marginTop: 4 }}>searching...</div>}
      {err && <div style={{ fontSize: 10, color: 'var(--red)', marginTop: 4 }}>{err}</div>}
      {hits.length > 0 && (
        <div style={{ marginTop: 6, display: 'flex', flexDirection: 'column', gap: 2 }}>
          {hits.map((h, i) => (
            <div
              key={i}
              title={h.summary}
              style={{
                fontSize: 10,
                fontFamily: 'var(--mono)',
                color: 'var(--text-muted)',
                padding: '2px 4px',
                overflow: 'hidden',
                textOverflow: 'ellipsis',
                whiteSpace: 'nowrap',
              }}
            >
              {h.path}
            </div>
          ))}
        </div>
      )}
    </div>
  )
}

// ---------- Main conversation pane ----------

function MessageList({ messages, running, onSendToCoder }) {
  const scrollRef = useRef(null)
  useEffect(() => {
    if (scrollRef.current) scrollRef.current.scrollTop = scrollRef.current.scrollHeight
  }, [messages, running])

  return (
    <div
      ref={scrollRef}
      style={{
        flex: 1,
        overflowY: 'auto',
        padding: '20px 0',
        display: 'flex',
        flexDirection: 'column',
        minHeight: 0,
      }}
    >
      {messages.length === 0 && !running && (
        <div style={{
          flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center',
          color: 'var(--text-dim)', fontSize: 12, fontFamily: 'var(--mono)',
        }}>
          start the conversation
        </div>
      )}
      {messages.map((m, i) => {
        const kindMeta = m.agent_kind ? KIND_META[m.agent_kind] : null
        const agentColor = kindMeta?.color ?? 'var(--accent)'
        return (
          <div
            key={i}
            style={{
              display: 'flex',
              justifyContent: m.role === 'user' ? 'flex-end' : 'flex-start',
              padding: '4px 20px',
            }}
          >
            {m.role === 'user' ? (
              <div style={{
                maxWidth: '70%',
                background: 'rgba(45,212,191,0.06)',
                border: '1px solid rgba(45,212,191,0.12)',
                borderRadius: '14px 14px 4px 14px',
                padding: '9px 14px',
              }}>
                <pre style={{
                  whiteSpace: 'pre-wrap', wordBreak: 'break-word',
                  color: 'var(--text)', lineHeight: 1.6, margin: 0,
                  fontFamily: 'var(--mono)', fontSize: 12,
                }}>{m.content}</pre>
              </div>
            ) : (
              <div style={{ maxWidth: '80%' }}>
                <div style={{
                  fontSize: 10, letterSpacing: '0.06em', marginBottom: 4,
                  color: m.role === 'error' ? 'var(--red)' : 'var(--text-muted)',
                  display: 'flex', gap: 6, alignItems: 'center',
                }}>
                  <span style={{
                    fontWeight: 600,
                    color: m.role === 'error' ? 'var(--red)' : agentColor,
                  }}>
                    {m.role === 'error' ? 'error' : (kindMeta?.label ?? 'Response')}
                  </span>
                  {m.job_id && (
                    <span style={{ color: 'var(--text-dim)', fontFamily: 'var(--mono)', fontSize: 9 }}>
                      {m.job_id.slice(0, 8)}
                    </span>
                  )}
                </div>
                <div className="ghost-md" style={{
                  color: m.role === 'error' ? 'var(--red)' : 'var(--text)',
                  lineHeight: 1.6,
                  fontFamily: 'var(--mono)', fontSize: 12,
                }}>
                  <ReactMarkdown>{m.content}</ReactMarkdown>
                </div>
                {m.agent_kind === 'brainstorm' && m.role !== 'error' && (
                  <div style={{ marginTop: 6 }}>
                    <button
                      onClick={() => onSendToCoder(m.content)}
                      style={{
                        padding: '4px 10px',
                        fontSize: 11,
                        fontFamily: 'var(--mono)',
                        background: 'transparent',
                        color: KIND_META.coder.color,
                        border: `1px solid ${KIND_META.coder.color}55`,
                        borderRadius: 'var(--radius-sm)',
                        cursor: 'pointer',
                      }}
                      title="Open a new coder thread pre-filled with this spec"
                    >
                      {'→'} Send to Coder
                    </button>
                  </div>
                )}
              </div>
            )}
          </div>
        )
      })}
      {running && (
        <div style={{ padding: '4px 20px', fontSize: 10, color: 'var(--text-dim)' }}>
          <span style={{ animation: 'blink 1s step-end infinite' }}>{'▋'}</span>
        </div>
      )}
    </div>
  )
}

function Composer({ disabled, running, onSend, tokenNode, budgetNode, overBudget }) {
  const [input, setInput] = useState('')
  const textareaRef = useRef(null)

  function handleSend() {
    if (!input.trim() || running) return
    onSend(input.trim())
    setInput('')
    if (textareaRef.current) textareaRef.current.style.height = 'auto'
  }

  function autoResize(e) {
    const el = e.target
    el.style.height = 'auto'
    el.style.height = Math.min(el.scrollHeight, 160) + 'px'
  }

  const hardBlock = disabled || running || overBudget
  return (
    <div style={{
      flexShrink: 0,
      background: 'var(--surface)',
      borderTop: '1px solid var(--border)',
    }}>
      {tokenNode && (
        <div style={{ padding: '4px 16px 0', display: 'flex', justifyContent: 'flex-end' }}>
          {tokenNode}
        </div>
      )}
      <div style={{
        padding: '10px 16px 14px',
        display: 'flex',
        gap: 8,
        alignItems: 'flex-end',
      }}>
        <textarea
          ref={textareaRef}
          rows={1}
          placeholder={
            overBudget ? 'daily cap reached — adjust in settings' :
            disabled ? 'select or create a thread' : 'message...'
          }
          value={input}
          onChange={e => { setInput(e.target.value); autoResize(e) }}
          onKeyDown={e => {
            if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); handleSend() }
          }}
          disabled={hardBlock}
          style={{
            flex: 1,
            fontFamily: 'var(--mono)',
            fontSize: 12,
            background: 'var(--bg)',
            color: 'var(--text)',
            border: '1px solid var(--border)',
            borderRadius: 'var(--radius)',
            padding: '10px 14px',
            outline: 'none',
            resize: 'none',
            minHeight: 42,
            maxHeight: 160,
            lineHeight: 1.6,
          }}
        />
        <button
          onClick={handleSend}
          disabled={hardBlock || !input.trim()}
          style={{
            flexShrink: 0,
            width: 40, height: 40,
            padding: 0,
            fontSize: 16,
            fontFamily: 'var(--mono)',
            background: (hardBlock || !input.trim()) ? 'var(--border)' : 'var(--accent)',
            color: (hardBlock || !input.trim()) ? 'var(--text-dim)' : 'var(--bg)',
            cursor: (hardBlock || !input.trim()) ? 'default' : 'pointer',
            borderRadius: 'var(--radius)',
            display: 'flex', alignItems: 'center', justifyContent: 'center',
            border: 'none',
          }}
        >
          {'↑'}
        </button>
      </div>
      <div style={{ padding: '0 16px 8px', fontSize: 10, color: 'var(--text-dim)', textAlign: 'center', fontFamily: 'var(--mono)' }}>
        enter to send / shift+enter for newline
      </div>
      {budgetNode}
    </div>
  )
}

// ---------- Root ----------

export default function CoderPanel({ daemonKey, alive, pinnedChats, setPinnedChats, setActivePinSlot, onFirstEntry }) {
  const [chats, setChats] = useState(loadCoderChats)
  const [activeChatId, setActiveChatId] = useState(null)
  const [running, setRunning] = useState(false)
  const [sectionOpen, setSectionOpen] = useState({ brainstorm: true, coder: true, orchestrator: true })
  const [pendingDiffs, setPendingDiffs] = useState([])
  const [autoApply, setAutoApply] = useState(false)
  const [diffBusy, setDiffBusy] = useState(false)
  const [diffCollapsed, setDiffCollapsed] = useState(false)
  const [activeJobId, setActiveJobId] = useState(null) // for live token SSE during a turn
  const [overBudget, setOverBudget] = useState(false)
  const firstEntryFired = useRef(false)

  useEffect(() => { saveCoderChats(chats) }, [chats])

  // Auto-collapse the main sidebar on first mount (per spec).
  useEffect(() => {
    if (!firstEntryFired.current) {
      firstEntryFired.current = true
      onFirstEntry?.()
    }
  }, [onFirstEntry])

  // Poll pending diffs while mounted. 5s cadence per spec.
  useEffect(() => {
    let cancelled = false
    async function pull() {
      try {
        const data = await apiFetch('/code/pending_diffs', {}, daemonKey)
        if (!cancelled) setPendingDiffs(Array.isArray(data) ? data : [])
      } catch { /* daemon offline etc. — leave last known */ }
    }
    pull()
    const id = setInterval(pull, 5_000)
    return () => { cancelled = true; clearInterval(id) }
  }, [daemonKey])

  // Read auto-apply setting once at mount + whenever key changes.
  useEffect(() => {
    let cancelled = false
    apiFetch('/settings', {}, daemonKey)
      .then(s => { if (!cancelled) setAutoApply(!!s?.['coder.auto_apply']) })
      .catch(() => { /* no daemon / not authed yet */ })
    return () => { cancelled = true }
  }, [daemonKey])

  async function handleDiffApply(id) {
    setDiffBusy(true)
    try {
      await apiFetch(`/code/diffs/${id}/apply`, { method: 'POST' }, daemonKey)
      setPendingDiffs(prev => prev.filter(d => d.id !== id))
    } catch (e) {
      // Surface the conflict on the card itself by marking it; simplest path
      // for v1 is an alert. Iterate later.
      alert(`apply failed: ${e.message}`)
    } finally {
      setDiffBusy(false)
    }
  }

  async function handleDiffReject(id) {
    setDiffBusy(true)
    try {
      await apiFetch(`/code/diffs/${id}/reject`, { method: 'POST' }, daemonKey)
      setPendingDiffs(prev => prev.filter(d => d.id !== id))
    } catch (e) {
      alert(`reject failed: ${e.message}`)
    } finally {
      setDiffBusy(false)
    }
  }

  const activeChat = useMemo(() => chats.find(c => c.id === activeChatId) || null, [chats, activeChatId])

  // Pin this coder chat whenever we switch into one.
  useEffect(() => {
    if (!activeChat) return
    setPinnedChats(prev => ({
      ...prev,
      code: { id: activeChat.id, kind: 'code', title: activeChat.name, agent_kind: activeChat.agent_kind },
    }))
    setActivePinSlot?.('code')
  }, [activeChat, setPinnedChats, setActivePinSlot])

  const grouped = useMemo(() => ({
    brainstorm: chats.filter(c => c.agent_kind === 'brainstorm'),
    coder: chats.filter(c => c.agent_kind === 'coder'),
    orchestrator: chats.filter(c => c.agent_kind === 'orchestrator'),
  }), [chats])

  async function dispatchSend({ chatSnapshot, text, prependUserMessage }) {
    const kind = chatSnapshot.agent_kind
    const chatId = chatSnapshot.id
    if (prependUserMessage) {
      setChats(prev => prev.map(c =>
        c.id === chatId ? { ...c, messages: [...c.messages, { role: 'user', content: text }] } : c
      ))
    }
    setRunning(true)
    try {
      const body = buildBody(kind, text, chatSnapshot, { skipLastUserInHistory: !prependUserMessage })
      const data = await apiFetch(KIND_META[kind].endpoint, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
      }, daemonKey)
      const assistantMsg = formatAssistant(kind, data)
      setChats(prev => prev.map(c => {
        if (c.id !== chatId) return c
        const next = { ...c, messages: [...c.messages, assistantMsg] }
        if (!c.server_chat_id) next.server_chat_id = data.chat_id ?? data.orchestration_id ?? null
        return next
      }))
    } catch (e) {
      setChats(prev => prev.map(c =>
        c.id === chatId
          ? { ...c, messages: [...c.messages, { role: 'error', content: e.message, agent_kind: kind }] }
          : c
      ))
    } finally {
      setRunning(false)
    }
  }

  const createChat = useCallback((agent_kind) => {
    const id = uid()
    const name = `${KIND_META[agent_kind].label} ${new Date().toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}`
    const chat = { id, name, agent_kind, messages: [], created_at: new Date().toISOString(), server_chat_id: null }
    setChats(prev => [chat, ...prev])
    setActiveChatId(id)
    return id
  }, [])

  function deleteChat(id) {
    setChats(prev => prev.filter(c => c.id !== id))
    if (activeChatId === id) setActiveChatId(null)
    setPinnedChats(prev => (prev?.code?.id === id ? { ...prev, code: null } : prev))
  }

  function handleSend(text) {
    if (!activeChat || running) return
    dispatchSend({ chatSnapshot: activeChat, text, prependUserMessage: true })
  }

  function handleSendToCoder(specText) {
    // Build a fresh chat with the spec seeded as the first user message, then
    // dispatch immediately. Snapshot the chat locally so dispatch doesn't race
    // against React state propagation.
    const id = uid()
    const name = `Coder ${new Date().toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}`
    const seedMsg = { role: 'user', content: specText }
    const newChat = {
      id, name, agent_kind: 'coder', messages: [seedMsg],
      created_at: new Date().toISOString(), server_chat_id: null,
    }
    setChats(prev => [newChat, ...prev])
    setActiveChatId(id)
    dispatchSend({ chatSnapshot: newChat, text: specText, prependUserMessage: false })
  }

  return (
    <div style={{ display: 'flex', flex: 1, minHeight: 0, overflow: 'hidden' }}>
      {/* Coder-specific sidebar */}
      <aside style={{
        width: 240,
        flexShrink: 0,
        background: 'var(--surface)',
        borderRight: '1px solid var(--border)',
        display: 'flex',
        flexDirection: 'column',
        overflow: 'hidden',
      }}>
        <NewThread onCreate={createChat} />
        <FileIndexSearch daemonKey={daemonKey} />
        <div style={{ flex: 1, overflowY: 'auto', padding: '8px 0' }}>
          <KindGroup
            title="Brainstorm" color={KIND_META.brainstorm.color}
            chats={grouped.brainstorm} activeChatId={activeChatId}
            onOpen={setActiveChatId} onDelete={deleteChat}
            open={sectionOpen.brainstorm} onToggleOpen={() => setSectionOpen(s => ({ ...s, brainstorm: !s.brainstorm }))}
          />
          <KindGroup
            title="Coder" color={KIND_META.coder.color}
            chats={grouped.coder} activeChatId={activeChatId}
            onOpen={setActiveChatId} onDelete={deleteChat}
            open={sectionOpen.coder} onToggleOpen={() => setSectionOpen(s => ({ ...s, coder: !s.coder }))}
          />
          <KindGroup
            title="Orchestrator" color={KIND_META.orchestrator.color}
            chats={grouped.orchestrator} activeChatId={activeChatId}
            onOpen={setActiveChatId} onDelete={deleteChat}
            open={sectionOpen.orchestrator} onToggleOpen={() => setSectionOpen(s => ({ ...s, orchestrator: !s.orchestrator }))}
          />
        </div>
      </aside>

      {/* Main conversation pane */}
      <main style={{ flex: 1, display: 'flex', flexDirection: 'column', minWidth: 0, overflow: 'hidden', minHeight: 0 }}>
        {activeChat ? (
          <>
            <div style={{
              padding: '6px 16px',
              borderBottom: '1px solid var(--border)',
              fontSize: 11,
              color: KIND_META[activeChat.agent_kind].color,
              letterSpacing: '0.04em',
              fontWeight: 600,
              background: 'var(--surface)',
              flexShrink: 0,
            }}>
              {KIND_META[activeChat.agent_kind].label.toUpperCase()} {'·'} {activeChat.name}
            </div>
            <MessageList messages={activeChat.messages} running={running} onSendToCoder={handleSendToCoder} />
            <Composer
              disabled={!alive}
              running={running}
              onSend={handleSend}
              overBudget={overBudget}
              tokenNode={
                running && activeJobId ? (
                  <TokenMeter mode="streaming" jobId={activeJobId} daemonKey={daemonKey} compact />
                ) : (
                  lastTokensFor(activeChat) && <TokenMeter mode="summary" tokens={lastTokensFor(activeChat)} compact />
                )
              }
              budgetNode={<BudgetBar daemonKey={daemonKey} onOverCap={setOverBudget} />}
            />
          </>
        ) : (
          <div style={{
            flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center',
            color: 'var(--text-dim)', fontSize: 12, fontFamily: 'var(--mono)',
          }}>
            pick a thread or hit "new thread"
          </div>
        )}
      </main>

      {/* Right-side diff review. Always shows a thin strip; expands to full
          panel when uncollapsed. Spec: auto-apply still renders cards as a log. */}
      <DiffReview
        diffs={pendingDiffs}
        autoApply={autoApply}
        onApply={handleDiffApply}
        onReject={handleDiffReject}
        collapsed={diffCollapsed}
        onToggleCollapsed={() => setDiffCollapsed(c => !c)}
      />
    </div>
  )
}

function lastTokensFor(chat) {
  if (!chat) return null
  for (let i = chat.messages.length - 1; i >= 0; i--) {
    const m = chat.messages[i]
    if (m.role === 'assistant' && m.tokens) return m.tokens
  }
  return null
}

function buildBody(kind, text, chat, options = {}) {
  if (kind === 'orchestrator') {
    return { spec: text, chat_id: chat.server_chat_id ?? undefined }
  }
  let msgs = chat.messages
  if (options.skipLastUserInHistory && msgs.length > 0 && msgs[msgs.length - 1].role === 'user') {
    msgs = msgs.slice(0, -1)
  }
  const history = msgs.slice(-10).map(m => ({
    role: m.role === 'error' ? 'assistant' : m.role,
    content: m.content,
  }))
  const base = { message: text, history }
  if (chat.server_chat_id) base.chat_id = chat.server_chat_id
  return base
}

function formatAssistant(kind, data) {
  if (kind === 'orchestrator') {
    const tasks = data.tasks || []
    const summary = tasks.length === 0
      ? '_no tasks emitted_'
      : tasks.map((t, i) => `**Task ${i + 1}**\n- prompt: ${t.task_prompt}\n- verify: \`${t.verify_command || '(none)'}\``).join('\n\n')
    return {
      role: 'assistant',
      agent_kind: kind,
      content: `Orchestration \`${(data.orchestration_id || '').slice(0, 8)}\` planned ${tasks.length} task(s).\n\n${summary}\n\n_Run them from the backend; live task polling lands in D.3._`,
      job_id: data.orchestration_id,
    }
  }
  return {
    role: 'assistant',
    agent_kind: kind,
    content: data.response ?? '(empty response)',
    job_id: data.job_id,
    tokens: data.tokens,
    pending_diff_ids: data.pending_diff_ids,
  }
}
