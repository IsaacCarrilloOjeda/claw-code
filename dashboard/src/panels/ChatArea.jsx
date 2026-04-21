import { useState, useEffect, useRef, useMemo } from 'react'
import ReactMarkdown from 'react-markdown'
import { AGENTS } from '../lib/api.js'

function ChatThread({ messages, running, alive, selectedAgents, onSend }) {
  const [input, setInput] = useState('')
  const scrollRef = useRef(null)
  const textareaRef = useRef(null)

  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight
    }
  }, [messages, running])

  function handleSend() {
    if (!input.trim() || running) return
    onSend(input.trim())
    setInput('')
    if (textareaRef.current) textareaRef.current.style.height = 'auto'
  }

  function handleKeyDown(e) {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault()
      handleSend()
    }
  }

  function autoResize(e) {
    const el = e.target
    el.style.height = 'auto'
    el.style.height = Math.min(el.scrollHeight, 160) + 'px'
  }

  const activeAgentLabel = useMemo(() => {
    if (selectedAgents.length === 0) return 'Chat'
    return selectedAgents.map(id => AGENTS.find(a => a.id === id)?.label ?? id).join(' + ')
  }, [selectedAgents])

  return (
    <div style={{ display: 'flex', flexDirection: 'column', flex: 1, overflow: 'hidden' }}>
      {/* Message area */}
      <div
        ref={scrollRef}
        style={{
          flex: 1,
          overflowY: 'auto',
          padding: '20px 0',
          display: 'flex',
          flexDirection: 'column',
          gap: 0,
          minHeight: 0,
        }}
      >
        {messages.length === 0 && !running && (
          <div style={{
            flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center',
            color: 'var(--text-dim)', fontSize: 12, fontFamily: 'var(--mono)',
          }}>
            start a conversation
          </div>
        )}

        {messages.map((msg, i) => (
          <div
            key={i}
            style={{
              display: 'flex',
              justifyContent: msg.role === 'user' ? 'flex-end' : 'flex-start',
              padding: '4px 20px',
            }}
          >
            {msg.role === 'user' ? (
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
                }}>
                  {msg.content}
                </pre>
              </div>
            ) : (
              <div style={{ maxWidth: '80%' }}>
                <div style={{
                  fontSize: 10, letterSpacing: '0.06em', marginBottom: 4,
                  color: msg.role === 'error' ? 'var(--red)' : 'var(--text-muted)',
                  display: 'flex', gap: 6, alignItems: 'center',
                }}>
                  <span style={{
                    fontWeight: 600,
                    color: msg.role === 'error' ? 'var(--red)' : (AGENTS.find(a => a.id === msg.agent)?.color ?? 'var(--accent)'),
                  }}>
                    {msg.role === 'error'
                      ? 'error'
                      : (!msg.agent || msg.agent === 'chat'
                          ? 'Chat'
                          : (AGENTS.find(a => a.id === msg.agent)?.label ?? msg.agent))}
                  </span>
                  {msg.job_id && (
                    <span style={{ color: 'var(--text-dim)', fontFamily: 'var(--mono)', fontSize: 9 }}>
                      {msg.job_id.slice(0, 8)}
                    </span>
                  )}
                </div>
                <div className="ghost-md" style={{
                  color: msg.role === 'error' ? 'var(--red)' : 'var(--text)',
                  lineHeight: 1.6,
                  fontFamily: 'var(--mono)', fontSize: 12,
                }}>
                  <ReactMarkdown>{msg.content}</ReactMarkdown>
                </div>
              </div>
            )}
          </div>
        ))}

        {running && (
          <div style={{ padding: '4px 20px', display: 'flex', justifyContent: 'flex-start' }}>
            <div style={{ fontSize: 10, color: 'var(--text-dim)' }}>
              <span style={{ color: 'var(--accent)', marginRight: 8, fontWeight: 600 }}>{activeAgentLabel}</span>
              <span style={{ animation: 'blink 1s step-end infinite' }}>{'\u258B'}</span>
            </div>
          </div>
        )}
      </div>

      {/* Input bar */}
      <div style={{
        flexShrink: 0,
        padding: '10px 16px 14px',
        background: 'var(--surface)',
      }}>
        {messages.length > 0 && (
          <div style={{ display: 'flex', justifyContent: 'flex-end', marginBottom: 4 }}>
            <span
              onClick={() => onSend(null)}
              style={{ color: 'var(--text-dim)', fontSize: 10, cursor: 'pointer', fontFamily: 'var(--mono)' }}
              onMouseEnter={e => e.target.style.color = 'var(--text-muted)'}
              onMouseLeave={e => e.target.style.color = 'var(--text-dim)'}
            >
              clear
            </span>
          </div>
        )}
        <div style={{ display: 'flex', gap: 8, alignItems: 'flex-end' }}>
          <textarea
            ref={textareaRef}
            rows={1}
            placeholder={alive ? `Message ${activeAgentLabel}...` : 'daemon offline'}
            value={input}
            onChange={e => { setInput(e.target.value); autoResize(e) }}
            onKeyDown={handleKeyDown}
            disabled={running || !alive}
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
              transition: 'border-color var(--transition)',
            }}
            onFocus={e => e.target.style.borderColor = 'var(--accent)'}
            onBlur={e => e.target.style.borderColor = 'var(--border)'}
          />
          <button
            onClick={handleSend}
            disabled={running || !alive || !input.trim()}
            style={{
              flexShrink: 0,
              width: 40, height: 40,
              padding: 0,
              fontSize: 16,
              fontFamily: 'var(--mono)',
              background: running || !alive || !input.trim() ? 'var(--border)' : 'var(--accent)',
              color: running || !alive || !input.trim() ? 'var(--text-dim)' : 'var(--bg)',
              cursor: running || !alive || !input.trim() ? 'default' : 'pointer',
              borderRadius: 'var(--radius)',
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'center',
              border: 'none',
              transition: 'all var(--transition)',
            }}
          >
            {'\u2191'}
          </button>
        </div>
        <div style={{ marginTop: 4, fontSize: 10, color: 'var(--text-dim)', textAlign: 'center', fontFamily: 'var(--mono)' }}>
          enter to send / shift+enter for newline
        </div>
      </div>
    </div>
  )
}

function AgentToggles({ selected, onToggle, collapsed, onCollapseToggle }) {
  return (
    <div style={{
      borderTop: '1px solid var(--border)',
      background: 'var(--surface)',
      flexShrink: 0,
    }}>
      {/* Collapse header */}
      <div
        onClick={onCollapseToggle}
        style={{
          display: 'flex',
          alignItems: 'center',
          padding: '4px 16px',
          cursor: 'pointer',
          gap: 6,
          fontSize: 10,
          color: 'var(--text-dim)',
          letterSpacing: '0.06em',
          textTransform: 'uppercase',
        }}
      >
        <span style={{ fontSize: 8 }}>{collapsed ? '\u25B6' : '\u25BC'}</span>
        <span>agents</span>
      </div>

      {!collapsed && (
        <div style={{
          display: 'flex',
          flexWrap: 'wrap',
          gap: 4,
          padding: '0 16px 8px',
        }}>
          {AGENTS.map(agent => {
            const isOn = selected.includes(agent.id)
            return (
              <button
                key={agent.id}
                onClick={() => onToggle(agent.id)}
                style={{
                  background: isOn ? agent.color + '1a' : 'transparent',
                  border: `1px solid ${isOn ? agent.color + '55' : 'var(--border)'}`,
                  color: isOn ? agent.color : 'var(--text-muted)',
                  borderRadius: 'var(--radius-sm)',
                  padding: '3px 10px',
                  fontSize: 11,
                  fontWeight: isOn ? 600 : 400,
                  cursor: 'pointer',
                  transition: 'all var(--transition)',
                  lineHeight: '18px',
                }}
              >
                {agent.label}
              </button>
            )
          })}
        </div>
      )}
    </div>
  )
}

const PREVIEW_HEADINGS = {
  chat:           'CHAT',
  director:       'DIRECTOR',
  research:       'RESEARCH RESULTS',
  calendar:       'CALENDAR',
  chief_of_staff: 'PLAN',
  docs:           'DOCS',
  dreamer:        'REFLECTIONS',
  scheduled:      'SCHEDULED TASKS',
}

const PREVIEW_EMPTY = {
  chat:           'no preview',
  director:       'no routing decision yet',
  research:       'no results yet',
  calendar:       'no events loaded',
  chief_of_staff: 'no plan yet',
  docs:           'no documents loaded',
  dreamer:        'no reflections yet',
  scheduled:      'no scheduled tasks',
}

function PreviewTab({ selectedAgents }) {
  const primary = selectedAgents[0] || 'chat'
  const agent = AGENTS.find(a => a.id === primary)
  const color = agent?.color ?? 'var(--accent)'
  const heading = PREVIEW_HEADINGS[primary] ?? (agent?.label.toUpperCase() ?? 'PREVIEW')
  const empty = PREVIEW_EMPTY[primary] ?? 'no preview'

  return (
    <div style={{ flex: 1, padding: 20 }}>
      <div style={{ color, fontSize: 10, fontWeight: 600, letterSpacing: '0.06em', marginBottom: 16 }}>
        {heading}
      </div>
      <div style={{ color: 'var(--text-dim)', fontSize: 12 }}>
        {empty}
      </div>
    </div>
  )
}

function ContextTab({ selectedAgents }) {
  const [coreExpanded, setCoreExpanded] = useState(false)
  const [memExpanded, setMemExpanded] = useState(false)

  // Planned tools per agent. `chat` lines up with `chat_dispatcher` in the
  // Rust backend; the rest correspond to `Intent::*` variants in `agents/intent.rs`.
  const toolsByAgent = {
    chat:           ['chat_dispatcher', 'memory_search'],
    director:       ['sonnet_reply', 'route_correct'],
    research:       ['brave_search', 'page_reader', 'summarize'],
    calendar:       ['gcal_read', 'gcal_create', 'gcal_edit', 'gcal_delete'],
    chief_of_staff: ['plan_build', 'plan_execute', 'memory_search'],
    docs:           ['drive_read', 'docs_edit', 'docs_create'],
    dreamer:        ['memory_consolidate', 'ghost_events_read'],
    scheduled:      ['cron_trigger', 'job_dispatch'],
  }

  const primary = selectedAgents[0] || 'chat'
  const tools = toolsByAgent[primary] || toolsByAgent.chat

  return (
    <div style={{ flex: 1, padding: 20, overflowY: 'auto' }}>
      {/* Core context card */}
      <div style={{
        background: 'var(--surface-2)',
        border: '1px solid var(--border)',
        borderRadius: 'var(--radius)',
        marginBottom: 12,
        overflow: 'hidden',
      }}>
        <div
          onClick={() => setCoreExpanded(!coreExpanded)}
          style={{
            padding: '10px 14px',
            cursor: 'pointer',
            display: 'flex',
            alignItems: 'center',
            gap: 8,
            fontSize: 11,
            color: 'var(--text-muted)',
          }}
        >
          <span style={{ fontSize: 8 }}>{coreExpanded ? '\u25BC' : '\u25B6'}</span>
          <span style={{ fontWeight: 600 }}>Core Context File</span>
        </div>
        {coreExpanded && (
          <div style={{
            padding: '0 14px 12px',
            fontSize: 11,
            color: 'var(--text-dim)',
            fontFamily: 'var(--mono)',
          }}>
            Loaded from GHOST_CORE_CONTEXT_PATH at dispatch time.
          </div>
        )}
      </div>

      {/* Injected memories card */}
      <div style={{
        background: 'var(--surface-2)',
        border: '1px solid var(--border)',
        borderRadius: 'var(--radius)',
        marginBottom: 16,
        overflow: 'hidden',
      }}>
        <div
          onClick={() => setMemExpanded(!memExpanded)}
          style={{
            padding: '10px 14px',
            cursor: 'pointer',
            display: 'flex',
            alignItems: 'center',
            gap: 8,
            fontSize: 11,
            color: 'var(--text-muted)',
          }}
        >
          <span style={{ fontSize: 8 }}>{memExpanded ? '\u25BC' : '\u25B6'}</span>
          <span style={{ fontWeight: 600 }}>Injected Memories</span>
        </div>
        {memExpanded && (
          <div style={{
            padding: '0 14px 12px',
            fontSize: 11,
            color: 'var(--text-dim)',
            fontFamily: 'var(--mono)',
          }}>
            Semantic search results injected at runtime (Phase 2).
          </div>
        )}
      </div>

      {/* Tools */}
      <div style={{
        fontSize: 10,
        fontWeight: 600,
        color: 'var(--text-dim)',
        letterSpacing: '0.06em',
        textTransform: 'uppercase',
        marginBottom: 8,
      }}>
        Tools
      </div>
      <div style={{ display: 'flex', flexWrap: 'wrap', gap: 6 }}>
        {tools.map(tool => (
          <span key={tool} style={{
            padding: '4px 10px',
            background: 'var(--surface-2)',
            border: '1px solid var(--border)',
            borderRadius: 12,
            fontSize: 11,
            fontFamily: 'var(--mono)',
            color: 'var(--text-muted)',
          }}>
            {tool}
          </span>
        ))}
      </div>
    </div>
  )
}

function ThinkingTab() {
  return (
    <div style={{
      flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center',
      color: 'var(--text-dim)', fontSize: 12, fontFamily: 'var(--mono)',
    }}>
      Thinking will stream here when enabled.
    </div>
  )
}

export default function ChatArea({
  chat, alive, selectedAgents, setSelectedAgents, running,
  onSendMessage, onClearMessages, agentsCollapsed, onAgentsCollapseToggle,
}) {
  const [innerTab, setInnerTab] = useState('chat')
  const tabs = ['Chat', 'Preview', 'Context', 'Thinking']

  function handleAgentToggle(agentId) {
    setSelectedAgents(prev => {
      if (agentId === 'chat') {
        if (prev.includes('chat')) {
          const without = prev.filter(id => id !== 'chat')
          return without.length === 0 ? ['chat'] : without
        }
        return [...prev, 'chat']
      }
      // Specialist toggle
      if (prev.includes(agentId)) {
        const without = prev.filter(id => id !== agentId)
        return without.length === 0 ? ['chat'] : without
      }
      // Max: chat + one specialist
      const specialists = prev.filter(id => id !== 'chat')
      if (specialists.length >= 1) {
        const hasChat = prev.includes('chat')
        return hasChat ? ['chat', agentId] : [agentId]
      }
      return [...prev, agentId]
    })
  }

  function handleSend(text) {
    if (text === null) {
      onClearMessages()
      return
    }
    onSendMessage(text)
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', flex: 1, overflow: 'hidden' }}>
      {/* Inner tabs */}
      <div style={{
        display: 'flex',
        borderBottom: '1px solid var(--border)',
        background: 'var(--surface)',
        flexShrink: 0,
      }}>
        {tabs.map(tab => {
          const key = tab.toLowerCase()
          const active = innerTab === key
          return (
            <button
              key={key}
              onClick={() => setInnerTab(key)}
              style={{
                background: 'none',
                border: 'none',
                borderBottom: active ? '2px solid var(--accent)' : '2px solid transparent',
                color: active ? 'var(--accent)' : 'var(--text-dim)',
                padding: '9px 16px',
                fontSize: 11,
                fontWeight: active ? 600 : 400,
                cursor: 'pointer',
                letterSpacing: '0.04em',
                transition: 'all var(--transition)',
              }}
            >
              {tab}
            </button>
          )
        })}
      </div>

      {/* Tab content */}
      {innerTab === 'chat' && (
        <>
          <ChatThread
            messages={chat.messages}
            running={running}
            alive={alive}
            selectedAgents={selectedAgents}
            onSend={handleSend}
          />
          <AgentToggles
            selected={selectedAgents}
            onToggle={handleAgentToggle}
            collapsed={agentsCollapsed}
            onCollapseToggle={onAgentsCollapseToggle}
          />
        </>
      )}
      {innerTab === 'preview' && <PreviewTab selectedAgents={selectedAgents} />}
      {innerTab === 'context' && <ContextTab selectedAgents={selectedAgents} />}
      {innerTab === 'thinking' && <ThinkingTab />}
    </div>
  )
}
