import { useState, useEffect, useRef, useCallback, useMemo } from 'react'
import ReactMarkdown from 'react-markdown'

// ─── Config ──────────────────────────────────────────────────────────────────

const API = import.meta.env.VITE_DAEMON_URL || 'http://127.0.0.1:7878'
const STORAGE_KEY = 'ghost-daemon-key'
const PROJECTS_KEY = 'ghost-projects'

const AGENTS = [
  { id: 'echo',     label: 'Echo',     color: '#2dd4bf' },
  { id: 'research', label: 'Research', color: '#3b82f6' },
  { id: 'email',    label: 'Email',    color: '#a78bfa' },
  { id: 'calendar', label: 'Calendar', color: '#f59e0b' },
  { id: 'code',     label: 'Code',     color: '#34d399' },
  { id: 'itguide',  label: 'IT Guide', color: '#22d3ee' },
  { id: 'law',      label: 'Law',      color: '#f43f5e' },
]

// ─── Helpers ─────────────────────────────────────────────────────────────────

function uid() {
  return crypto.randomUUID?.() ?? Math.random().toString(36).slice(2, 10)
}

function fmtUptime(secs) {
  if (secs == null) return '--'
  const d = Math.floor(secs / 86400)
  const h = Math.floor((secs % 86400) / 3600)
  const m = Math.floor((secs % 3600) / 60)
  if (d > 0) return `${d}d ${h}h`
  if (h > 0) return `${h}h ${m}m`
  if (m > 0) return `${m}m`
  return `${secs}s`
}

async function apiFetch(path, opts = {}, token = null) {
  const headers = { ...(opts.headers || {}) }
  if (token) headers['Authorization'] = `Bearer ${token}`
  const signal = opts.signal ?? AbortSignal.timeout(10_000)
  const r = await fetch(`${API}${path}`, { ...opts, headers, signal })
  if (!r.ok) throw new Error(`${r.status}`)
  return r.json()
}

function timeAgo(isoString) {
  const secs = Math.floor((Date.now() - new Date(isoString).getTime()) / 1000)
  if (secs < 60) return 'now'
  if (secs < 3600) return `${Math.floor(secs / 60)}m`
  if (secs < 86400) return `${Math.floor(secs / 3600)}h`
  if (secs < 604800) return `${Math.floor(secs / 86400)}d`
  return new Date(isoString).toLocaleDateString()
}

function loadProjects() {
  try {
    const raw = localStorage.getItem(PROJECTS_KEY)
    if (raw) return JSON.parse(raw)
  } catch { /* ignore */ }
  return [{ id: uid(), name: 'Default', expanded: true, chats: [{ id: uid(), name: 'General', messages: [] }] }]
}

function saveProjects(projects) {
  localStorage.setItem(PROJECTS_KEY, JSON.stringify(projects))
}

// ─── Auth Screen ─────────────────────────────────────────────────────────────

function AuthScreen({ onAuth }) {
  const [key, setKey] = useState('')
  const [checking, setChecking] = useState(false)
  const [error, setError] = useState(null)
  const inputRef = useRef(null)

  useEffect(() => { inputRef.current?.focus() }, [])

  async function submit(e) {
    e.preventDefault()
    if (!key.trim()) return
    setChecking(true)
    setError(null)
    try {
      // First check if daemon is reachable at all (health is open)
      await apiFetch('/health', { signal: AbortSignal.timeout(6_000) })
    } catch {
      setChecking(false)
      setError('daemon unreachable')
      return
    }
    try {
      // Validate key against a protected endpoint
      await fetch(`${API}/director/config`, {
        method: 'POST',
        headers: {
          'Authorization': `Bearer ${key.trim()}`,
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({}),
        signal: AbortSignal.timeout(6_000),
      }).then(r => {
        // 401 = wrong key. Anything else (200, 400, 500) means the key was accepted.
        if (r.status === 401) throw new Error('401')
      })
      localStorage.setItem(STORAGE_KEY, key.trim())
      onAuth(key.trim())
    } catch (err) {
      if (err.message.includes('401')) {
        setError('wrong key')
      } else {
        // Key might be valid but endpoint errored for another reason — let them in
        localStorage.setItem(STORAGE_KEY, key.trim())
        onAuth(key.trim())
      }
    } finally {
      setChecking(false)
    }
  }

  return (
    <div style={{
      height: '100vh',
      display: 'flex',
      flexDirection: 'column',
      alignItems: 'center',
      justifyContent: 'center',
      background: 'var(--bg)',
      gap: 0,
    }}>
      {error && (
        <div style={{
          position: 'fixed', top: 0, left: 0, right: 0,
          background: 'rgba(244,63,94,0.12)',
          borderBottom: '1px solid rgba(244,63,94,0.3)',
          color: '#f43f5e',
          fontSize: 12,
          fontWeight: 600,
          padding: '8px 16px',
          textAlign: 'center',
          letterSpacing: '0.04em',
        }}>
          {error}
        </div>
      )}

      <div style={{
        fontFamily: 'var(--sans)',
        fontSize: 28,
        fontWeight: 700,
        color: 'var(--accent)',
        letterSpacing: '-0.03em',
        marginBottom: 8,
      }}>
        GHOST
      </div>
      <div style={{ color: 'var(--text-muted)', fontSize: 12, marginBottom: 32 }}>
        enter daemon key
      </div>

      <form onSubmit={submit} style={{ display: 'flex', gap: 8, width: 340 }}>
        <input
          ref={inputRef}
          type="password"
          value={key}
          onChange={e => { setKey(e.target.value); setError(null) }}
          placeholder="GHOST_DAEMON_KEY"
          autoComplete="off"
          spellCheck={false}
          style={{
            flex: 1,
            fontFamily: 'var(--mono)',
            fontSize: 13,
            background: 'var(--surface)',
            color: 'var(--text)',
            border: `1px solid ${error ? 'var(--red)' : 'var(--border)'}`,
            borderRadius: 'var(--radius)',
            padding: '10px 14px',
            outline: 'none',
            transition: 'border-color var(--transition)',
          }}
          onFocus={e => { if (!error) e.target.style.borderColor = 'var(--accent)' }}
          onBlur={e => { if (!error) e.target.style.borderColor = 'var(--border)' }}
        />
        <button
          type="submit"
          disabled={checking || !key.trim()}
          style={{
            fontFamily: 'var(--mono)',
            fontSize: 12,
            fontWeight: 600,
            padding: '10px 20px',
            background: checking || !key.trim() ? 'var(--border)' : 'var(--accent)',
            color: checking || !key.trim() ? 'var(--text-muted)' : 'var(--bg)',
            border: 'none',
            borderRadius: 'var(--radius)',
            cursor: checking || !key.trim() ? 'default' : 'pointer',
            letterSpacing: '0.04em',
            textTransform: 'uppercase',
            transition: 'all var(--transition)',
          }}
        >
          {checking ? '...' : 'enter'}
        </button>
      </form>

      {error === 'wrong key' && (
        <div style={{ color: 'var(--red)', fontSize: 12, marginTop: 12 }}>
          wrong key
        </div>
      )}
    </div>
  )
}

// ─── Top Bar ─────────────────────────────────────────────────────────────────

function TopBar({ alive, status, openTabs, activeTabId, onSelectTab, onCloseTab }) {
  return (
    <header style={{
      display: 'flex',
      alignItems: 'center',
      height: 'var(--topbar-h)',
      background: 'var(--surface)',
      borderBottom: '1px solid var(--border)',
      padding: '0 16px',
      flexShrink: 0,
      gap: 0,
      overflow: 'hidden',
    }}>
      {/* Health dot + GHOST */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexShrink: 0 }}>
        <span style={{
          width: 7, height: 7,
          borderRadius: '50%',
          background: alive ? 'var(--green)' : 'var(--red)',
          boxShadow: alive ? '0 0 6px var(--green)' : '0 0 6px var(--red)',
          display: 'inline-block',
        }} />
        <span style={{
          fontWeight: 700,
          fontSize: 13,
          color: 'var(--accent)',
          letterSpacing: '-0.02em',
        }}>GHOST</span>
      </div>

      {/* Uptime */}
      <span style={{
        color: 'var(--text-muted)',
        fontSize: 11,
        fontFamily: 'var(--mono)',
        marginLeft: 10,
        flexShrink: 0,
      }}>
        {status ? fmtUptime(status.uptime_secs) : '--'}
      </span>

      {/* Divider */}
      <div style={{
        width: 1, height: 20,
        background: 'var(--border)',
        margin: '0 12px',
        flexShrink: 0,
      }} />

      {/* Chat tabs */}
      <div style={{
        display: 'flex',
        gap: 2,
        overflow: 'hidden',
        flex: 1,
        minWidth: 0,
      }}>
        {openTabs.map(tab => {
          const isActive = tab.id === activeTabId
          const tabColor = isActive ? 'var(--blue)' : 'var(--green)'
          return (
            <div
              key={tab.id}
              onClick={() => onSelectTab(tab.id)}
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 6,
                padding: '4px 10px',
                background: isActive ? 'var(--surface-2)' : 'transparent',
                borderRadius: 'var(--radius-sm)',
                cursor: 'pointer',
                flexShrink: 0,
                maxWidth: 160,
                transition: 'background var(--transition)',
              }}
              onMouseEnter={e => { if (!isActive) e.currentTarget.style.background = 'var(--bg-raised)' }}
              onMouseLeave={e => { if (!isActive) e.currentTarget.style.background = 'transparent' }}
            >
              <span style={{
                width: 5, height: 5,
                borderRadius: '50%',
                background: tabColor,
                flexShrink: 0,
              }} />
              <span style={{
                fontSize: 11,
                color: isActive ? 'var(--text-bright)' : 'var(--text-muted)',
                overflow: 'hidden',
                textOverflow: 'ellipsis',
                whiteSpace: 'nowrap',
                fontWeight: isActive ? 600 : 400,
              }}>
                {tab.name}
              </span>
              <span
                onClick={e => { e.stopPropagation(); onCloseTab(tab.id) }}
                style={{
                  fontSize: 13,
                  color: 'var(--text-dim)',
                  cursor: 'pointer',
                  lineHeight: 1,
                  padding: '0 2px',
                  flexShrink: 0,
                }}
                onMouseEnter={e => e.target.style.color = 'var(--red)'}
                onMouseLeave={e => e.target.style.color = 'var(--text-dim)'}
              >
                x
              </span>
            </div>
          )
        })}
      </div>
    </header>
  )
}

// ─── Job Banner ──────────────────────────────────────────────────────────────

function JobBanner({ job, onDismiss }) {
  if (!job) return null
  return (
    <div style={{
      height: 'var(--banner-h)',
      background: 'var(--accent-dim)',
      borderBottom: '1px solid var(--border)',
      display: 'flex',
      alignItems: 'center',
      padding: '0 16px',
      fontSize: 11,
      fontFamily: 'var(--mono)',
      gap: 12,
      flexShrink: 0,
    }}>
      <span style={{
        width: 6, height: 6,
        borderRadius: '50%',
        background: 'var(--accent)',
        animation: 'pulse 1.5s ease-in-out infinite',
      }} />
      <span style={{ color: 'var(--accent)', fontWeight: 600 }}>{job.agent}</span>
      <span style={{ color: 'var(--text-muted)' }}>{job.status}</span>
      <span style={{ color: 'var(--text-muted)' }}>{job.elapsed != null ? `${job.elapsed}s` : ''}</span>
      <span style={{ flex: 1 }} />
      <span
        onClick={onDismiss}
        style={{ color: 'var(--text-dim)', cursor: 'pointer', fontSize: 14, lineHeight: 1 }}
        onMouseEnter={e => e.target.style.color = 'var(--text)'}
        onMouseLeave={e => e.target.style.color = 'var(--text-dim)'}
      >
        x
      </span>
    </div>
  )
}

// ─── Sidebar: Project Tree ───────────────────────────────────────────────────

function ProjectTree({ projects, setProjects, onOpenChat, activeChatId }) {
  function addProject() {
    setProjects(prev => [...prev, { id: uid(), name: 'New Project', expanded: false, chats: [] }])
  }

  function toggleProject(pid) {
    setProjects(prev => prev.map(p => p.id === pid ? { ...p, expanded: !p.expanded } : p))
  }

  function renameProject(pid, name) {
    setProjects(prev => prev.map(p => p.id === pid ? { ...p, name } : p))
  }

  function deleteProject(pid) {
    setProjects(prev => prev.filter(p => p.id !== pid))
  }

  function addChat(pid) {
    setProjects(prev => prev.map(p => {
      if (p.id !== pid) return p
      return { ...p, expanded: true, chats: [...p.chats, { id: uid(), name: 'New Chat', messages: [] }] }
    }))
  }

  function deleteChat(pid, cid) {
    setProjects(prev => prev.map(p => {
      if (p.id !== pid) return p
      return { ...p, chats: p.chats.filter(c => c.id !== cid) }
    }))
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 2, padding: '8px 0' }}>
      {/* Add project button */}
      <div
        onClick={addProject}
        style={{
          padding: '6px 12px',
          fontSize: 11,
          color: 'var(--text-muted)',
          cursor: 'pointer',
          display: 'flex',
          alignItems: 'center',
          gap: 6,
          transition: 'color var(--transition)',
        }}
        onMouseEnter={e => e.currentTarget.style.color = 'var(--accent)'}
        onMouseLeave={e => e.currentTarget.style.color = 'var(--text-muted)'}
      >
        <span style={{ fontSize: 14, lineHeight: 1 }}>+</span>
        <span>new project</span>
      </div>

      {projects.map(project => (
        <ProjectItem
          key={project.id}
          project={project}
          onToggle={() => toggleProject(project.id)}
          onRename={name => renameProject(project.id, name)}
          onDelete={() => deleteProject(project.id)}
          onAddChat={() => addChat(project.id)}
          onDeleteChat={cid => deleteChat(project.id, cid)}
          onOpenChat={(cid) => onOpenChat(project.id, cid)}
          activeChatId={activeChatId}
        />
      ))}
    </div>
  )
}

function ProjectItem({ project, onToggle, onRename, onDelete, onAddChat, onDeleteChat, onOpenChat, activeChatId }) {
  const [editing, setEditing] = useState(false)
  const [editName, setEditName] = useState(project.name)
  const [confirmDelete, setConfirmDelete] = useState(false)
  const inputRef = useRef(null)

  useEffect(() => {
    if (editing) {
      inputRef.current?.focus()
      inputRef.current?.select()
    }
  }, [editing])

  function commitRename() {
    const trimmed = editName.trim()
    if (trimmed && trimmed !== project.name) onRename(trimmed)
    else setEditName(project.name)
    setEditing(false)
  }

  function handleDelete() {
    if (!confirmDelete) {
      setConfirmDelete(true)
      setTimeout(() => setConfirmDelete(false), 3000)
      return
    }
    onDelete()
  }

  return (
    <div>
      {/* Project row */}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          padding: '5px 12px',
          cursor: 'pointer',
          gap: 6,
          transition: 'background var(--transition)',
        }}
        onMouseEnter={e => e.currentTarget.style.background = 'var(--bg-raised)'}
        onMouseLeave={e => e.currentTarget.style.background = 'transparent'}
      >
        <span
          onClick={onToggle}
          style={{ fontSize: 9, color: 'var(--text-dim)', width: 12, textAlign: 'center', flexShrink: 0 }}
        >
          {project.expanded ? '\u25BC' : '\u25B6'}
        </span>

        {editing ? (
          <input
            ref={inputRef}
            value={editName}
            onChange={e => setEditName(e.target.value)}
            onBlur={commitRename}
            onKeyDown={e => { if (e.key === 'Enter') commitRename(); if (e.key === 'Escape') { setEditName(project.name); setEditing(false) } }}
            style={{
              flex: 1,
              background: 'var(--surface-2)',
              border: '1px solid var(--accent)',
              borderRadius: 'var(--radius-sm)',
              color: 'var(--text)',
              fontSize: 12,
              padding: '2px 6px',
              outline: 'none',
              fontFamily: 'var(--sans)',
            }}
          />
        ) : (
          <span
            onClick={onToggle}
            onDoubleClick={() => { setEditName(project.name); setEditing(true) }}
            style={{
              flex: 1,
              fontSize: 12,
              fontWeight: 500,
              color: 'var(--text)',
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              whiteSpace: 'nowrap',
            }}
          >
            {project.name}
          </span>
        )}

        {/* Add chat */}
        <span
          onClick={e => { e.stopPropagation(); onAddChat() }}
          style={{
            fontSize: 14, lineHeight: 1, color: 'var(--text-dim)', cursor: 'pointer',
            padding: '0 2px', flexShrink: 0,
          }}
          onMouseEnter={e => e.target.style.color = 'var(--accent)'}
          onMouseLeave={e => e.target.style.color = 'var(--text-dim)'}
        >
          +
        </span>

        {/* Delete project */}
        <span
          onClick={e => { e.stopPropagation(); handleDelete() }}
          style={{
            fontSize: 11, lineHeight: 1,
            color: confirmDelete ? 'var(--red)' : 'var(--text-dim)',
            cursor: 'pointer', padding: '0 2px', flexShrink: 0,
            fontFamily: 'var(--mono)',
            fontWeight: confirmDelete ? 600 : 400,
          }}
          onMouseEnter={e => { if (!confirmDelete) e.target.style.color = 'var(--red)' }}
          onMouseLeave={e => { if (!confirmDelete) e.target.style.color = 'var(--text-dim)' }}
        >
          {confirmDelete ? 'confirm?' : 'x'}
        </span>
      </div>

      {/* Chats under project */}
      {project.expanded && (
        <div style={{ paddingLeft: 20 }}>
          {project.chats.map(chat => (
            <ChatItem
              key={chat.id}
              chat={chat}
              isActive={chat.id === activeChatId}
              onOpen={() => onOpenChat(chat.id)}
              onDelete={() => onDeleteChat(chat.id)}
            />
          ))}
        </div>
      )}
    </div>
  )
}

function ChatItem({ chat, isActive, onOpen, onDelete }) {
  const [confirmDelete, setConfirmDelete] = useState(false)

  function handleDelete(e) {
    e.stopPropagation()
    if (!confirmDelete) {
      setConfirmDelete(true)
      setTimeout(() => setConfirmDelete(false), 3000)
      return
    }
    onDelete()
  }

  return (
    <div
      onClick={onOpen}
      style={{
        display: 'flex',
        alignItems: 'center',
        padding: '4px 8px',
        cursor: 'pointer',
        gap: 6,
        borderRadius: 'var(--radius-sm)',
        background: isActive ? 'var(--accent-dim)' : 'transparent',
        transition: 'background var(--transition)',
      }}
      onMouseEnter={e => { if (!isActive) e.currentTarget.style.background = 'var(--bg-raised)' }}
      onMouseLeave={e => { if (!isActive) e.currentTarget.style.background = isActive ? 'var(--accent-dim)' : 'transparent' }}
    >
      <span style={{
        fontSize: 12,
        color: isActive ? 'var(--accent)' : 'var(--text-muted)',
        flex: 1,
        overflow: 'hidden',
        textOverflow: 'ellipsis',
        whiteSpace: 'nowrap',
      }}>
        {chat.name}
      </span>
      <span
        onClick={handleDelete}
        style={{
          fontSize: 11, lineHeight: 1,
          color: confirmDelete ? 'var(--red)' : 'var(--text-dim)',
          cursor: 'pointer', padding: '0 2px', flexShrink: 0,
          fontFamily: 'var(--mono)',
          fontWeight: confirmDelete ? 600 : 400,
        }}
        onMouseEnter={e => { if (!confirmDelete) e.target.style.color = 'var(--red)' }}
        onMouseLeave={e => { if (!confirmDelete) e.target.style.color = 'var(--text-dim)' }}
      >
        {confirmDelete ? 'confirm?' : 'x'}
      </span>
    </div>
  )
}

// ─── Sidebar Bottom Nav ──────────────────────────────────────────────────────

function SidebarNav({ activeNav, onNav }) {
  const items = ['SMS', 'Settings', 'Statistics', 'About']
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 1, padding: '8px 0' }}>
      {items.map(item => {
        const active = activeNav === item.toLowerCase()
        return (
          <div
            key={item}
            onClick={() => onNav(item.toLowerCase())}
            style={{
              padding: '6px 14px',
              fontSize: 12,
              color: active ? 'var(--accent)' : 'var(--text-muted)',
              cursor: 'pointer',
              background: active ? 'var(--accent-dim)' : 'transparent',
              borderRadius: 'var(--radius-sm)',
              margin: '0 6px',
              transition: 'all var(--transition)',
            }}
            onMouseEnter={e => { if (!active) e.currentTarget.style.background = 'var(--bg-raised)' }}
            onMouseLeave={e => { if (!active) e.currentTarget.style.background = active ? 'var(--accent-dim)' : 'transparent' }}
          >
            {item}
          </div>
        )
      })}
    </div>
  )
}

// ─── Left Sidebar ────────────────────────────────────────────────────────────

function Sidebar({ collapsed, onToggle, projects, setProjects, onOpenChat, activeChatId, activeNav, onNav, bottomRatio, onBottomResize }) {
  const dragRef = useRef(null)
  const sidebarRef = useRef(null)

  function startDrag(e) {
    e.preventDefault()
    const startY = e.clientY
    const startRatio = bottomRatio

    function onMove(ev) {
      if (!sidebarRef.current) return
      const rect = sidebarRef.current.getBoundingClientRect()
      const totalH = rect.height
      const delta = startY - ev.clientY
      const newRatio = Math.min(0.5, Math.max(0.1, startRatio + delta / totalH))
      onBottomResize(newRatio)
    }

    function onUp() {
      document.removeEventListener('mousemove', onMove)
      document.removeEventListener('mouseup', onUp)
    }

    document.addEventListener('mousemove', onMove)
    document.addEventListener('mouseup', onUp)
  }

  if (collapsed) {
    return (
      <div style={{
        width: 36,
        background: 'var(--surface)',
        borderRight: '1px solid var(--border)',
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        flexShrink: 0,
      }}>
        <div
          onClick={onToggle}
          style={{
            padding: '12px 0',
            cursor: 'pointer',
            color: 'var(--text-dim)',
            fontSize: 13,
            transition: 'color var(--transition)',
          }}
          onMouseEnter={e => e.currentTarget.style.color = 'var(--accent)'}
          onMouseLeave={e => e.currentTarget.style.color = 'var(--text-dim)'}
          title="Expand sidebar"
        >
          {'\u25B6'}
        </div>
      </div>
    )
  }

  return (
    <div
      ref={sidebarRef}
      style={{
        width: 'var(--sidebar-w)',
        background: 'var(--surface)',
        borderRight: '1px solid var(--border)',
        display: 'flex',
        flexDirection: 'column',
        flexShrink: 0,
        overflow: 'hidden',
        position: 'relative',
      }}
    >
      {/* Collapse button on right edge */}
      <div
        onClick={onToggle}
        style={{
          position: 'absolute',
          top: 10,
          right: 0,
          width: 18,
          height: 22,
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          cursor: 'pointer',
          color: 'var(--text-dim)',
          fontSize: 9,
          zIndex: 2,
          borderRadius: '3px 0 0 3px',
          transition: 'all var(--transition)',
        }}
        onMouseEnter={e => { e.currentTarget.style.color = 'var(--accent)'; e.currentTarget.style.background = 'var(--surface-2)' }}
        onMouseLeave={e => { e.currentTarget.style.color = 'var(--text-dim)'; e.currentTarget.style.background = 'transparent' }}
        title="Collapse sidebar"
      >
        {'\u25C0'}
      </div>

      {/* Top: scrollable project tree */}
      <div style={{
        flex: `1 1 ${(1 - bottomRatio) * 100}%`,
        overflowY: 'auto',
        overflowX: 'hidden',
        minHeight: 0,
      }}>
        <ProjectTree
          projects={projects}
          setProjects={setProjects}
          onOpenChat={onOpenChat}
          activeChatId={activeChatId}
        />
      </div>

      {/* Draggable divider */}
      <div
        ref={dragRef}
        onMouseDown={startDrag}
        style={{
          height: 3,
          background: 'var(--border)',
          cursor: 'ns-resize',
          flexShrink: 0,
          transition: 'background var(--transition)',
        }}
        onMouseEnter={e => e.currentTarget.style.background = 'var(--accent)'}
        onMouseLeave={e => e.currentTarget.style.background = 'var(--border)'}
      />

      {/* Bottom: fixed nav */}
      <div style={{
        flex: `0 0 ${bottomRatio * 100}%`,
        overflowY: 'auto',
        overflowX: 'hidden',
        minHeight: 0,
      }}>
        <SidebarNav activeNav={activeNav} onNav={onNav} />
      </div>
    </div>
  )
}

// ─── Agent Toggles ───────────────────────────────────────────────────────────

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

// ─── Chat Tab (messages + input) ─────────────────────────────────────────────

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
    if (selectedAgents.length === 0) return 'Echo'
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
                    {msg.role === 'error' ? 'error' : (msg.agent ? AGENTS.find(a => a.id === msg.agent)?.label ?? msg.agent : 'Echo')}
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

// ─── Preview Tab ─────────────────────────────────────────────────────────────

function PreviewTab({ selectedAgents }) {
  const primary = selectedAgents[0] || 'echo'
  const agent = AGENTS.find(a => a.id === primary)
  const label = agent?.label ?? 'Echo'
  const color = agent?.color ?? 'var(--accent)'

  if (primary === 'code') {
    return (
      <div style={{
        flex: 1, display: 'flex', flexDirection: 'column',
        background: '#0a0a0a',
        fontFamily: 'var(--mono)',
        fontSize: 12,
        color: 'var(--text-dim)',
        padding: 20,
      }}>
        <div style={{ color, fontSize: 10, fontWeight: 600, letterSpacing: '0.06em', marginBottom: 12 }}>
          CODE TERMINAL
        </div>
        <div style={{ flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
          no output yet
        </div>
      </div>
    )
  }

  if (primary === 'email') {
    return (
      <div style={{ flex: 1, padding: 20 }}>
        <div style={{ color, fontSize: 10, fontWeight: 600, letterSpacing: '0.06em', marginBottom: 16 }}>
          EMAIL DRAFTS
        </div>
        <div style={{
          background: 'var(--surface-2)',
          border: '1px solid var(--border)',
          borderRadius: 'var(--radius)',
          padding: 16,
          color: 'var(--text-dim)',
          fontSize: 12,
        }}>
          no drafts yet
        </div>
      </div>
    )
  }

  if (primary === 'research') {
    return (
      <div style={{ flex: 1, padding: 20 }}>
        <div style={{ color, fontSize: 10, fontWeight: 600, letterSpacing: '0.06em', marginBottom: 16 }}>
          RESEARCH RESULTS
        </div>
        <div style={{ color: 'var(--text-dim)', fontSize: 12 }}>
          no results yet
        </div>
      </div>
    )
  }

  if (primary === 'calendar') {
    return (
      <div style={{ flex: 1, padding: 20 }}>
        <div style={{ color, fontSize: 10, fontWeight: 600, letterSpacing: '0.06em', marginBottom: 16 }}>
          CALENDAR
        </div>
        <div style={{ color: 'var(--text-dim)', fontSize: 12 }}>
          no events loaded
        </div>
      </div>
    )
  }

  if (primary === 'itguide') {
    return (
      <div style={{ flex: 1, padding: 20 }}>
        <div style={{ color, fontSize: 10, fontWeight: 600, letterSpacing: '0.06em', marginBottom: 16 }}>
          STEP MAP
        </div>
        <div style={{ color: 'var(--text-dim)', fontSize: 12 }}>
          no steps yet
        </div>
      </div>
    )
  }

  if (primary === 'law') {
    return (
      <div style={{ flex: 1, padding: 20 }}>
        <div style={{ color, fontSize: 10, fontWeight: 600, letterSpacing: '0.06em', marginBottom: 16 }}>
          LEGAL CITATIONS
        </div>
        <div style={{ color: 'var(--text-dim)', fontSize: 12 }}>
          no citations yet
        </div>
      </div>
    )
  }

  // echo / default
  return (
    <div style={{ flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center', color: 'var(--text-dim)', fontSize: 12 }}>
      {label} preview
    </div>
  )
}

// ─── Context Tab ─────────────────────────────────────────────────────────────

function ContextTab({ selectedAgents }) {
  const [coreExpanded, setCoreExpanded] = useState(false)
  const [memExpanded, setMemExpanded] = useState(false)

  const toolsByAgent = {
    echo: ['chat_dispatcher', 'memory_search'],
    research: ['brave_search', 'page_reader', 'summarize'],
    email: ['gmail_read', 'gmail_draft', 'gmail_send'],
    calendar: ['gcal_read', 'gcal_create', 'gcal_edit', 'gcal_delete'],
    code: ['e2b_execute', 'file_write', 'github_push'],
    itguide: ['brave_search', 'screenshot_read', 'step_map'],
    law: ['cornell_search', 'case_lookup', 'citation_format'],
  }

  const primary = selectedAgents[0] || 'echo'
  const tools = toolsByAgent[primary] || toolsByAgent.echo

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

// ─── Thinking Tab ────────────────────────────────────────────────────────────

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

// ─── Settings Panel ──────────────────────────────────────────────────────────

function SettingsPanel() {
  return (
    <div style={{ flex: 1, padding: 24, overflowY: 'auto' }}>
      <div style={{ fontSize: 14, fontWeight: 600, color: 'var(--text-bright)', marginBottom: 20 }}>
        Settings
      </div>

      <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
        <SettingRow label="Job status banner" description="Show in-flight job status below the top bar" defaultOn={true} />
        <SettingRow label="Auto-switch tabs" description="Switch to agent preview tab on response" defaultOn={false} />
        <SettingRow label="Agent thinking (Echo)" description="Stream narrated reasoning for Echo agent" defaultOn={false} />
        <SettingRow label="Agent thinking (Research)" description="Stream narrated reasoning for Research agent" defaultOn={true} />
        <SettingRow label="Agent thinking (Code)" description="Stream narrated reasoning for Code agent" defaultOn={true} />
      </div>
    </div>
  )
}

function SettingRow({ label, description, defaultOn }) {
  const [on, setOn] = useState(defaultOn)
  return (
    <div style={{
      display: 'flex',
      alignItems: 'center',
      justifyContent: 'space-between',
      padding: '12px 16px',
      background: 'var(--surface-2)',
      border: '1px solid var(--border)',
      borderRadius: 'var(--radius)',
    }}>
      <div>
        <div style={{ fontSize: 12, fontWeight: 500, color: 'var(--text)' }}>{label}</div>
        <div style={{ fontSize: 11, color: 'var(--text-dim)', marginTop: 2 }}>{description}</div>
      </div>
      <div
        onClick={() => setOn(!on)}
        style={{
          width: 36, height: 20,
          background: on ? 'var(--accent)' : 'var(--border)',
          borderRadius: 10,
          cursor: 'pointer',
          position: 'relative',
          transition: 'background var(--transition)',
          flexShrink: 0,
          marginLeft: 16,
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
    </div>
  )
}

// ─── Statistics Panel ────────────────────────────────────────────────────────

function StatisticsPanel() {
  return (
    <div style={{ flex: 1, padding: 24, overflowY: 'auto' }}>
      <div style={{ fontSize: 14, fontWeight: 600, color: 'var(--text-bright)', marginBottom: 20 }}>
        Statistics
      </div>
      <div style={{ color: 'var(--text-dim)', fontSize: 12, fontFamily: 'var(--mono)' }}>
        Usage statistics will appear here once jobs are tracked.
      </div>
    </div>
  )
}

// ─── About Panel ─────────────────────────────────────────────────────────────

function AboutPanel() {
  return (
    <div style={{ flex: 1, padding: 24, overflowY: 'auto' }}>
      <div style={{ fontSize: 14, fontWeight: 600, color: 'var(--text-bright)', marginBottom: 20 }}>
        About GHOST
      </div>

      <div style={{ display: 'flex', flexDirection: 'column', gap: 12, fontSize: 12, color: 'var(--text)' }}>
        <p style={{ lineHeight: 1.6 }}>
          GHOST is a personal AI operating system. Routes requests through a Director AI to specialist agents for code, email, calendar, research, and more.
        </p>

        <div style={{
          background: 'var(--surface-2)',
          border: '1px solid var(--border)',
          borderRadius: 'var(--radius)',
          padding: 14,
        }}>
          <div style={{ fontSize: 10, fontWeight: 600, color: 'var(--text-dim)', letterSpacing: '0.06em', textTransform: 'uppercase', marginBottom: 8 }}>
            Agents
          </div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 4, fontFamily: 'var(--mono)', fontSize: 11 }}>
            {AGENTS.map(a => (
              <div key={a.id} style={{ display: 'flex', gap: 8 }}>
                <span style={{ color: a.color, width: 70 }}>{a.label}</span>
                <span style={{ color: 'var(--text-muted)' }}>
                  {a.id === 'echo' && 'General chat, default agent'}
                  {a.id === 'research' && 'Brave Search, web synthesis'}
                  {a.id === 'email' && 'Gmail read, draft, send'}
                  {a.id === 'calendar' && 'Google Calendar CRUD'}
                  {a.id === 'code' && 'DeepSeek + E2B sandbox'}
                  {a.id === 'itguide' && 'Step-by-step navigation'}
                  {a.id === 'law' && 'US legal research + citations'}
                </span>
              </div>
            ))}
          </div>
        </div>

        <div style={{ fontSize: 10, color: 'var(--text-dim)', lineHeight: 1.6, marginTop: 8 }}>
          AI disclaimer: GHOST provides AI-generated responses. Verify critical information independently.
          This system is built and operated by Isaac Carrillo / KYNE Systems.
        </div>
      </div>
    </div>
  )
}

// ─── SMS Panel ──────────────────────────────────────────────────────────────

function SmsPanel({ daemonKey }) {
  const [contacts, setContacts] = useState([])
  const [loading, setLoading] = useState(true)
  const [selectedPhone, setSelectedPhone] = useState(null)
  const [search, setSearch] = useState('')
  const [showSchedule, setShowSchedule] = useState(false)
  const [showAddForm, setShowAddForm] = useState(false)
  const [convos, setConvos] = useState({}) // { [phone]: { messages: [], hasMore: bool, loading: bool } }
  const [scheduleEntries, setScheduleEntries] = useState([])

  useEffect(() => { loadContacts() }, [])

  async function loadContacts() {
    setLoading(true)
    try {
      const data = await apiFetch('/sms/contacts', {}, daemonKey)
      setContacts(Array.isArray(data) ? data : data.contacts || [])
    } catch { /* ignore */ }
    setLoading(false)
  }

  async function loadConversation(phone, before = null) {
    const key = phone
    if (!before) {
      setConvos(prev => ({ ...prev, [key]: { messages: [], hasMore: true, loading: true } }))
    } else {
      setConvos(prev => ({ ...prev, [key]: { ...prev[key], loading: true } }))
    }
    try {
      const qs = before ? `?limit=30&before=${before}` : '?limit=30'
      const data = await apiFetch(`/sms/history/${encodeURIComponent(phone)}${qs}`, {}, daemonKey)
      const msgs = data.messages || data
      const hasMore = data.has_more ?? (msgs.length === 30)
      setConvos(prev => {
        const existing = prev[key]?.messages || []
        const merged = before ? [...msgs, ...existing] : msgs
        return { ...prev, [key]: { messages: merged, hasMore, loading: false } }
      })
    } catch {
      setConvos(prev => ({ ...prev, [key]: { ...(prev[key] || { messages: [], hasMore: false }), loading: false } }))
    }
  }

  async function sendMessage(phone, text) {
    // Optimistically append
    const tempMsg = { id: 'temp-' + uid(), role: 'assistant', content: text, created_at: new Date().toISOString(), manual: true }
    setConvos(prev => {
      const existing = prev[phone] || { messages: [], hasMore: false, loading: false }
      return { ...prev, [phone]: { ...existing, messages: [...existing.messages, tempMsg] } }
    })
    try {
      const data = await apiFetch('/sms/send', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ to: phone, body: text }),
      }, daemonKey)
      // Update temp message with real id
      setConvos(prev => {
        const convo = prev[phone]
        if (!convo) return prev
        return { ...prev, [phone]: { ...convo, messages: convo.messages.map(m => m.id === tempMsg.id ? { ...m, id: data.message_id, sent: true } : m) } }
      })
    } catch {
      // Mark as failed
      setConvos(prev => {
        const convo = prev[phone]
        if (!convo) return prev
        return { ...prev, [phone]: { ...convo, messages: convo.messages.map(m => m.id === tempMsg.id ? { ...m, failed: true } : m) } }
      })
    }
  }

  async function toggleAutoReply(phone, enabled) {
    // Optimistic update
    setContacts(prev => prev.map(c => c.phone === phone ? { ...c, auto_reply: enabled } : c))
    try {
      await apiFetch(`/sms/contacts/${encodeURIComponent(phone)}/auto-reply`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ enabled }),
      }, daemonKey)
    } catch {
      // Revert
      setContacts(prev => prev.map(c => c.phone === phone ? { ...c, auto_reply: !enabled } : c))
    }
  }

  async function renameContact(phone, name) {
    try {
      await apiFetch(`/sms/contacts/${encodeURIComponent(phone)}/name`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ name }),
      }, daemonKey)
      setContacts(prev => prev.map(c => c.phone === phone ? { ...c, display_name: name } : c))
    } catch { /* ignore */ }
  }

  async function addContact(phone, name) {
    if (name) {
      try {
        await apiFetch(`/sms/contacts/${encodeURIComponent(phone)}/name`, {
          method: 'PUT',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ name }),
        }, daemonKey)
      } catch { /* ignore */ }
    }
    await loadContacts()
    setShowAddForm(false)
    setSelectedPhone(phone)
    loadConversation(phone)
  }

  async function loadSchedule() {
    try {
      const data = await apiFetch('/schedule', {}, daemonKey)
      setScheduleEntries(data)
    } catch { /* ignore */ }
  }

  function handleSelectContact(phone) {
    setSelectedPhone(phone)
    if (!convos[phone]) loadConversation(phone)
  }

  const filtered = contacts.filter(c => {
    if (!search) return true
    const q = search.toLowerCase()
    return (c.display_name || '').toLowerCase().includes(q) || (c.phone || '').includes(q)
  })

  const selectedContact = contacts.find(c => c.phone === selectedPhone)
  const convo = selectedPhone ? convos[selectedPhone] : null

  return (
    <div style={{ flex: 1, display: 'flex', overflow: 'hidden', position: 'relative' }}>
      {/* Left column: contact list */}
      <div style={{
        width: 260, flexShrink: 0,
        background: 'var(--surface)',
        borderRight: '1px solid var(--border)',
        display: 'flex', flexDirection: 'column',
        overflow: 'hidden',
      }}>
        {/* Header: search + add + schedule */}
        <div style={{ padding: '10px 10px 6px', display: 'flex', flexDirection: 'column', gap: 6, flexShrink: 0 }}>
          <div style={{ display: 'flex', gap: 6, alignItems: 'center' }}>
            <input
              placeholder="Search contacts..."
              value={search}
              onChange={e => setSearch(e.target.value)}
              style={{
                flex: 1, fontFamily: 'var(--mono)', fontSize: 11,
                background: 'var(--surface)', color: 'var(--text)',
                border: '1px solid var(--border)', borderRadius: 'var(--radius-sm)',
                padding: '6px 8px', outline: 'none',
              }}
              onFocus={e => e.target.style.borderColor = 'var(--accent)'}
              onBlur={e => e.target.style.borderColor = 'var(--border)'}
            />
            <span
              onClick={loadContacts}
              style={{ color: 'var(--text-dim)', cursor: 'pointer', fontSize: 12, padding: '4px', lineHeight: 1 }}
              onMouseEnter={e => e.target.style.color = 'var(--accent)'}
              onMouseLeave={e => e.target.style.color = 'var(--text-dim)'}
              title="Refresh"
            >{'\u21BB'}</span>
            <span
              onClick={() => setShowAddForm(!showAddForm)}
              style={{ color: 'var(--text-dim)', cursor: 'pointer', fontSize: 16, fontWeight: 600, padding: '2px 4px', lineHeight: 1 }}
              onMouseEnter={e => e.target.style.color = 'var(--accent)'}
              onMouseLeave={e => e.target.style.color = 'var(--text-dim)'}
              title="Add contact"
            >+</span>
            <span
              onClick={() => { setShowSchedule(!showSchedule); if (!showSchedule) loadSchedule() }}
              style={{
                fontSize: 10, fontWeight: 600, padding: '4px 8px',
                color: showSchedule ? 'var(--accent)' : 'var(--text-dim)',
                cursor: 'pointer', letterSpacing: '0.04em',
                background: showSchedule ? 'var(--accent-dim)' : 'transparent',
                borderRadius: 'var(--radius-sm)',
                transition: 'all var(--transition)',
              }}
              onMouseEnter={e => { if (!showSchedule) e.target.style.color = 'var(--accent)' }}
              onMouseLeave={e => { if (!showSchedule) e.target.style.color = showSchedule ? 'var(--accent)' : 'var(--text-dim)' }}
            >SCHED</span>
          </div>
          {showAddForm && <SmsAddForm onAdd={addContact} onCancel={() => setShowAddForm(false)} />}
        </div>

        {/* Contact list */}
        <div style={{ flex: 1, overflowY: 'auto', overflowX: 'hidden' }}>
          {loading && contacts.length === 0 && (
            <div style={{ padding: 20, textAlign: 'center', color: 'var(--text-dim)', fontSize: 11 }}>loading...</div>
          )}
          {!loading && contacts.length === 0 && (
            <div style={{ padding: 20, textAlign: 'center', color: 'var(--text-dim)', fontSize: 11, lineHeight: 1.6 }}>
              No SMS conversations yet. Messages will appear here when GHOST receives texts.
            </div>
          )}
          {filtered.map(c => (
            <SmsContactRow
              key={c.phone}
              contact={c}
              active={c.phone === selectedPhone}
              onSelect={() => handleSelectContact(c.phone)}
              onToggleAutoReply={enabled => toggleAutoReply(c.phone, enabled)}
              onRename={name => renameContact(c.phone, name)}
            />
          ))}
        </div>
      </div>

      {/* Right column: conversation */}
      <div style={{ flex: 1, display: 'flex', flexDirection: 'column', overflow: 'hidden', minWidth: 0 }}>
        {selectedPhone && selectedContact ? (
          <SmsConversation
            contact={selectedContact}
            convo={convo}
            onSend={text => sendMessage(selectedPhone, text)}
            onLoadMore={() => {
              const msgs = convo?.messages || []
              if (msgs.length > 0 && convo?.hasMore) loadConversation(selectedPhone, msgs[0].id)
            }}
          />
        ) : (
          <div style={{
            flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center',
            flexDirection: 'column', gap: 8,
          }}>
            <div style={{ fontSize: 14, fontWeight: 600, color: 'var(--text-dim)' }}>SMS</div>
            <div style={{ color: 'var(--text-dim)', fontSize: 12 }}>select a contact to view messages</div>
          </div>
        )}
      </div>

      {/* Schedule overlay */}
      {showSchedule && (
        <SmsSchedulePanel
          daemonKey={daemonKey}
          entries={scheduleEntries}
          setEntries={setScheduleEntries}
          onClose={() => setShowSchedule(false)}
        />
      )}
    </div>
  )
}

function SmsAddForm({ onAdd, onCancel }) {
  const [phone, setPhone] = useState('')
  const [name, setName] = useState('')
  return (
    <div style={{
      display: 'flex', flexDirection: 'column', gap: 4,
      padding: '6px 0',
    }}>
      <input
        placeholder="+1..."
        value={phone}
        onChange={e => setPhone(e.target.value)}
        style={{
          fontFamily: 'var(--mono)', fontSize: 11,
          background: 'var(--bg)', color: 'var(--text)',
          border: '1px solid var(--border)', borderRadius: 'var(--radius-sm)',
          padding: '5px 8px', outline: 'none',
        }}
      />
      <input
        placeholder="Name (optional)"
        value={name}
        onChange={e => setName(e.target.value)}
        style={{
          fontFamily: 'var(--sans)', fontSize: 11,
          background: 'var(--bg)', color: 'var(--text)',
          border: '1px solid var(--border)', borderRadius: 'var(--radius-sm)',
          padding: '5px 8px', outline: 'none',
        }}
      />
      <div style={{ display: 'flex', gap: 4 }}>
        <button
          onClick={() => { if (phone.trim()) onAdd(phone.trim(), name.trim() || null) }}
          disabled={!phone.trim()}
          style={{
            flex: 1, padding: '4px 0', fontSize: 10, fontWeight: 600,
            background: phone.trim() ? 'var(--accent)' : 'var(--border)',
            color: phone.trim() ? 'var(--bg)' : 'var(--text-dim)',
            border: 'none', borderRadius: 'var(--radius-sm)',
            cursor: phone.trim() ? 'pointer' : 'default',
          }}
        >ADD</button>
        <button
          onClick={onCancel}
          style={{
            padding: '4px 10px', fontSize: 10,
            background: 'transparent', color: 'var(--text-dim)',
            border: '1px solid var(--border)', borderRadius: 'var(--radius-sm)',
            cursor: 'pointer',
          }}
        >cancel</button>
      </div>
    </div>
  )
}

function SmsContactRow({ contact, active, onSelect, onToggleAutoReply, onRename }) {
  const [editing, setEditing] = useState(false)
  const [editName, setEditName] = useState(contact.display_name || '')
  const inputRef = useRef(null)

  useEffect(() => { if (editing) { inputRef.current?.focus(); inputRef.current?.select() } }, [editing])

  function commitRename() {
    const trimmed = editName.trim()
    if (trimmed && trimmed !== (contact.display_name || '')) onRename(trimmed)
    else setEditName(contact.display_name || '')
    setEditing(false)
  }

  const displayName = contact.display_name || contact.phone
  const lastMsg = contact.last_message || ''
  const preview = lastMsg.length > 40 ? lastMsg.slice(0, 40) + '...' : lastMsg

  return (
    <div
      onClick={onSelect}
      style={{
        display: 'flex', alignItems: 'center', gap: 8,
        padding: '8px 10px', cursor: 'pointer',
        background: active ? 'var(--accent-dim)' : 'transparent',
        transition: 'background var(--transition)',
        borderBottom: '1px solid rgba(255,255,255,0.03)',
      }}
      onMouseEnter={e => { if (!active) e.currentTarget.style.background = 'var(--bg-raised)' }}
      onMouseLeave={e => { e.currentTarget.style.background = active ? 'var(--accent-dim)' : 'transparent' }}
    >
      {/* Auto-reply toggle */}
      <div
        onClick={e => { e.stopPropagation(); onToggleAutoReply(!contact.auto_reply) }}
        style={{
          width: 28, height: 16, flexShrink: 0,
          background: contact.auto_reply ? 'var(--accent)' : 'var(--border)',
          borderRadius: 8, cursor: 'pointer',
          position: 'relative', transition: 'background var(--transition)',
        }}
        title={contact.auto_reply ? 'Auto-reply ON' : 'Auto-reply OFF'}
      >
        <div style={{
          width: 12, height: 12, borderRadius: '50%', background: '#fff',
          position: 'absolute', top: 2,
          left: contact.auto_reply ? 14 : 2,
          transition: 'left var(--transition)',
        }} />
      </div>

      {/* Name + preview */}
      <div style={{ flex: 1, minWidth: 0 }}>
        {editing ? (
          <input
            ref={inputRef}
            value={editName}
            onChange={e => setEditName(e.target.value)}
            onBlur={commitRename}
            onKeyDown={e => { if (e.key === 'Enter') commitRename(); if (e.key === 'Escape') { setEditName(contact.display_name || ''); setEditing(false) } }}
            onClick={e => e.stopPropagation()}
            style={{
              width: '100%', background: 'var(--surface-2)',
              border: '1px solid var(--accent)', borderRadius: 'var(--radius-sm)',
              color: 'var(--text)', fontSize: 12, padding: '1px 4px', outline: 'none',
              fontFamily: 'var(--sans)',
            }}
          />
        ) : (
          <div
            onDoubleClick={e => { e.stopPropagation(); setEditName(contact.display_name || contact.phone); setEditing(true) }}
            style={{
              fontSize: 12, fontWeight: 500,
              color: active ? 'var(--text-bright)' : 'var(--text)',
              overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
            }}
          >{displayName}</div>
        )}
        {preview && (
          <div style={{
            fontSize: 10, color: 'var(--text-dim)', marginTop: 2,
            overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
          }}>{preview}</div>
        )}
      </div>

      {/* Time ago */}
      {contact.last_message_at && (
        <span style={{ fontSize: 9, color: 'var(--text-dim)', flexShrink: 0, fontFamily: 'var(--mono)' }}>
          {timeAgo(contact.last_message_at)}
        </span>
      )}
    </div>
  )
}

function SmsConversation({ contact, convo, onSend, onLoadMore }) {
  const [input, setInput] = useState('')
  const scrollRef = useRef(null)
  const textareaRef = useRef(null)
  const prevHeightRef = useRef(0)
  const isInitialLoad = useRef(true)

  // Auto-scroll to bottom on initial load or new message
  useEffect(() => {
    if (!scrollRef.current || !convo?.messages?.length) return
    if (isInitialLoad.current || !convo.loading) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight
      isInitialLoad.current = false
    }
  }, [convo?.messages?.length])

  // Reset initial load flag when contact changes
  useEffect(() => { isInitialLoad.current = true }, [contact.phone])

  // Preserve scroll position when prepending older messages
  useEffect(() => {
    if (!scrollRef.current) return
    const el = scrollRef.current
    if (prevHeightRef.current > 0 && el.scrollHeight > prevHeightRef.current) {
      el.scrollTop = el.scrollHeight - prevHeightRef.current
    }
    prevHeightRef.current = 0
  }, [convo?.messages])

  function handleScroll() {
    const el = scrollRef.current
    if (!el || !convo?.hasMore || convo?.loading) return
    if (el.scrollTop < 60) {
      prevHeightRef.current = el.scrollHeight
      onLoadMore()
    }
  }

  function handleSend() {
    if (!input.trim()) return
    onSend(input.trim())
    setInput('')
    if (textareaRef.current) textareaRef.current.style.height = 'auto'
    isInitialLoad.current = true // scroll to bottom after sending
  }

  function handleKeyDown(e) {
    if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); handleSend() }
  }

  function autoResize(e) {
    const el = e.target
    el.style.height = 'auto'
    el.style.height = Math.min(el.scrollHeight, 120) + 'px'
  }

  const messages = convo?.messages || []
  const displayName = contact.display_name || contact.phone

  // Group messages by date
  let lastDate = null

  return (
    <div style={{ display: 'flex', flexDirection: 'column', flex: 1, overflow: 'hidden' }}>
      {/* Header */}
      <div style={{
        flexShrink: 0, padding: '10px 16px',
        borderBottom: '1px solid var(--border)', background: 'var(--surface)',
        display: 'flex', alignItems: 'center', gap: 10,
      }}>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ fontSize: 13, fontWeight: 600, color: 'var(--text-bright)' }}>{displayName}</div>
          <div style={{ fontSize: 10, color: 'var(--text-dim)', fontFamily: 'var(--mono)' }}>{contact.phone}</div>
        </div>
        <div style={{
          fontSize: 9, fontWeight: 600, letterSpacing: '0.04em',
          padding: '3px 8px', borderRadius: 'var(--radius-sm)',
          background: contact.auto_reply ? 'rgba(45,212,191,0.12)' : 'rgba(255,255,255,0.04)',
          color: contact.auto_reply ? 'var(--accent)' : 'var(--text-dim)',
        }}>
          {contact.auto_reply ? 'AUTO-REPLY ON' : 'AUTO-REPLY OFF'}
        </div>
      </div>

      {/* Messages */}
      <div
        ref={scrollRef}
        onScroll={handleScroll}
        style={{
          flex: 1, overflowY: 'auto', padding: '12px 0',
          display: 'flex', flexDirection: 'column', gap: 0, minHeight: 0,
        }}
      >
        {convo?.loading && messages.length === 0 && (
          <div style={{ padding: 20, textAlign: 'center', color: 'var(--text-dim)', fontSize: 11 }}>loading...</div>
        )}
        {convo?.loading && messages.length > 0 && (
          <div style={{ padding: '8px 20px', textAlign: 'center', color: 'var(--text-dim)', fontSize: 10 }}>loading older...</div>
        )}
        {!convo?.loading && messages.length === 0 && (
          <div style={{ flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center', color: 'var(--text-dim)', fontSize: 11 }}>
            no messages yet
          </div>
        )}

        {messages.map((msg, i) => {
          const msgDate = new Date(msg.created_at).toLocaleDateString()
          let showDate = false
          if (msgDate !== lastDate) { showDate = true; lastDate = msgDate }

          const isOutbound = msg.role === 'assistant'
          return (
            <div key={msg.id || i}>
              {showDate && (
                <div style={{
                  textAlign: 'center', padding: '8px 0 4px',
                  fontSize: 9, color: 'var(--text-dim)', fontFamily: 'var(--mono)',
                  letterSpacing: '0.04em',
                }}>{msgDate}</div>
              )}
              <div style={{
                display: 'flex',
                justifyContent: isOutbound ? 'flex-end' : 'flex-start',
                padding: '3px 16px',
              }}>
                {isOutbound ? (
                  <div style={{ maxWidth: '70%' }}>
                    <div style={{
                      background: 'rgba(45,212,191,0.06)',
                      border: msg.failed ? '1px solid var(--red)' : '1px solid rgba(45,212,191,0.12)',
                      borderRadius: '14px 14px 4px 14px',
                      padding: '8px 12px',
                    }}>
                      <div className="ghost-md" style={{
                        color: 'var(--text)', lineHeight: 1.6,
                        fontFamily: 'var(--mono)', fontSize: 12,
                      }}>
                        <ReactMarkdown>{msg.content}</ReactMarkdown>
                      </div>
                    </div>
                    <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 6, marginTop: 2 }}>
                      {msg.manual && <span style={{ fontSize: 8, color: 'var(--text-dim)' }}>manual</span>}
                      {msg.failed && <span style={{ fontSize: 8, color: 'var(--red)' }}>failed</span>}
                      {msg.created_at && <span style={{ fontSize: 8, color: 'var(--text-dim)', fontFamily: 'var(--mono)' }}>
                        {new Date(msg.created_at).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}
                      </span>}
                    </div>
                  </div>
                ) : (
                  <div style={{ maxWidth: '80%' }}>
                    <div style={{
                      fontSize: 9, letterSpacing: '0.06em', marginBottom: 3,
                      color: 'var(--text-muted)', fontWeight: 600,
                    }}>{contact.display_name || contact.phone}</div>
                    <div className="ghost-md" style={{
                      color: 'var(--text)', lineHeight: 1.6,
                      fontFamily: 'var(--mono)', fontSize: 12,
                    }}>
                      <ReactMarkdown>{msg.content}</ReactMarkdown>
                    </div>
                    {msg.created_at && <div style={{ fontSize: 8, color: 'var(--text-dim)', fontFamily: 'var(--mono)', marginTop: 2 }}>
                      {new Date(msg.created_at).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}
                    </div>}
                  </div>
                )}
              </div>
            </div>
          )
        })}
      </div>

      {/* Input bar */}
      <div style={{
        flexShrink: 0, padding: '10px 16px 14px', background: 'var(--surface)',
      }}>
        <div style={{ display: 'flex', gap: 8, alignItems: 'flex-end' }}>
          <textarea
            ref={textareaRef}
            rows={1}
            placeholder={`Send SMS to ${displayName}...`}
            value={input}
            onChange={e => { setInput(e.target.value); autoResize(e) }}
            onKeyDown={handleKeyDown}
            style={{
              flex: 1, fontFamily: 'var(--mono)', fontSize: 12,
              background: 'var(--bg)', color: 'var(--text)',
              border: '1px solid var(--border)', borderRadius: 'var(--radius)',
              padding: '10px 14px', outline: 'none', resize: 'none',
              minHeight: 42, maxHeight: 120, lineHeight: 1.6,
              transition: 'border-color var(--transition)',
            }}
            onFocus={e => e.target.style.borderColor = 'var(--accent)'}
            onBlur={e => e.target.style.borderColor = 'var(--border)'}
          />
          <button
            onClick={handleSend}
            disabled={!input.trim()}
            style={{
              flexShrink: 0, width: 40, height: 40, padding: 0, fontSize: 16,
              fontFamily: 'var(--mono)',
              background: !input.trim() ? 'var(--border)' : 'var(--accent)',
              color: !input.trim() ? 'var(--text-dim)' : 'var(--bg)',
              cursor: !input.trim() ? 'default' : 'pointer',
              borderRadius: 'var(--radius)',
              display: 'flex', alignItems: 'center', justifyContent: 'center',
              border: 'none', transition: 'all var(--transition)',
            }}
          >{'\u2191'}</button>
        </div>
      </div>
    </div>
  )
}

function SmsSchedulePanel({ daemonKey, entries, setEntries, onClose }) {
  const [newPersistent, setNewPersistent] = useState('')
  const [newDaily, setNewDaily] = useState('')
  const [selectedDate, setSelectedDate] = useState(() => new Date().toISOString().slice(0, 10))
  const [adding, setAdding] = useState(false)

  const persistent = entries.filter(e => e.kind === 'persistent')
  const daily = entries.filter(e => e.kind === 'daily')
  const dailyForDate = daily.filter(e => e.day_date === selectedDate)
  const today = new Date().toISOString().slice(0, 10)

  async function addEntry(kind, content, dayDate) {
    setAdding(true)
    try {
      const body = { kind, content }
      if (dayDate) body.day_date = dayDate
      await apiFetch('/schedule', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
      }, daemonKey)
      // Reload
      const data = await apiFetch('/schedule', {}, daemonKey)
      setEntries(data)
    } catch { /* ignore */ }
    setAdding(false)
  }

  async function deleteEntry(id) {
    try {
      await apiFetch(`/schedule/${id}`, { method: 'DELETE' }, daemonKey)
      setEntries(prev => prev.filter(e => e.id !== id))
    } catch { /* ignore */ }
  }

  return (
    <>
      {/* Backdrop */}
      <div
        onClick={onClose}
        style={{
          position: 'absolute', inset: 0,
          background: 'rgba(0,0,0,0.4)',
          zIndex: 10,
        }}
      />
      {/* Panel */}
      <div style={{
        position: 'absolute', top: 0, right: 0, bottom: 0,
        width: 360, background: 'var(--surface)',
        borderLeft: '1px solid var(--border)',
        zIndex: 11, display: 'flex', flexDirection: 'column',
        overflow: 'hidden',
      }}>
        {/* Header */}
        <div style={{
          flexShrink: 0, padding: '14px 16px',
          borderBottom: '1px solid var(--border)',
          display: 'flex', alignItems: 'center', justifyContent: 'space-between',
        }}>
          <span style={{ fontSize: 13, fontWeight: 600, color: 'var(--text-bright)' }}>Schedule</span>
          <span
            onClick={onClose}
            style={{ color: 'var(--text-dim)', cursor: 'pointer', fontSize: 14, lineHeight: 1 }}
            onMouseEnter={e => e.target.style.color = 'var(--text)'}
            onMouseLeave={e => e.target.style.color = 'var(--text-dim)'}
          >x</span>
        </div>

        <div style={{ flex: 1, overflowY: 'auto', padding: '16px' }}>
          {/* Persistent */}
          <div style={{
            fontSize: 9, fontWeight: 600, color: 'var(--text-dim)',
            letterSpacing: '0.08em', marginBottom: 8,
          }}>RECURRING</div>
          {persistent.length === 0 && (
            <div style={{ fontSize: 11, color: 'var(--text-dim)', marginBottom: 8 }}>no recurring commitments</div>
          )}
          {persistent.map(e => (
            <div key={e.id} style={{
              display: 'flex', alignItems: 'center', gap: 8,
              padding: '6px 0', borderBottom: '1px solid rgba(255,255,255,0.03)',
            }}>
              <span style={{ flex: 1, fontSize: 12, color: 'var(--text)', lineHeight: 1.5 }}>{e.content}</span>
              <span
                onClick={() => deleteEntry(e.id)}
                style={{ color: 'var(--text-dim)', cursor: 'pointer', fontSize: 12, lineHeight: 1, flexShrink: 0 }}
                onMouseEnter={e2 => e2.target.style.color = 'var(--red)'}
                onMouseLeave={e2 => e2.target.style.color = 'var(--text-dim)'}
              >x</span>
            </div>
          ))}
          <div style={{ display: 'flex', gap: 6, marginTop: 8, marginBottom: 24 }}>
            <input
              placeholder="Add recurring..."
              value={newPersistent}
              onChange={e => setNewPersistent(e.target.value)}
              onKeyDown={e => { if (e.key === 'Enter' && newPersistent.trim()) { addEntry('persistent', newPersistent.trim()); setNewPersistent('') } }}
              style={{
                flex: 1, fontFamily: 'var(--sans)', fontSize: 11,
                background: 'var(--bg)', color: 'var(--text)',
                border: '1px solid var(--border)', borderRadius: 'var(--radius-sm)',
                padding: '6px 8px', outline: 'none',
              }}
            />
            <button
              onClick={() => { if (newPersistent.trim()) { addEntry('persistent', newPersistent.trim()); setNewPersistent('') } }}
              disabled={!newPersistent.trim() || adding}
              style={{
                padding: '4px 10px', fontSize: 10, fontWeight: 600,
                background: newPersistent.trim() ? 'var(--accent)' : 'var(--border)',
                color: newPersistent.trim() ? 'var(--bg)' : 'var(--text-dim)',
                border: 'none', borderRadius: 'var(--radius-sm)',
                cursor: newPersistent.trim() ? 'pointer' : 'default',
              }}
            >Add</button>
          </div>

          {/* Daily */}
          <div style={{
            fontSize: 9, fontWeight: 600, color: 'var(--text-dim)',
            letterSpacing: '0.08em', marginBottom: 8,
          }}>DAILY</div>
          <input
            type="date"
            value={selectedDate}
            onChange={e => setSelectedDate(e.target.value)}
            style={{
              fontFamily: 'var(--mono)', fontSize: 11,
              background: 'var(--bg)', color: 'var(--text)',
              border: '1px solid var(--border)', borderRadius: 'var(--radius-sm)',
              padding: '6px 8px', outline: 'none', marginBottom: 8,
              colorScheme: 'dark',
            }}
          />
          {dailyForDate.length === 0 && (
            <div style={{ fontSize: 11, color: 'var(--text-dim)', marginBottom: 8 }}>no entries for this date</div>
          )}
          {dailyForDate.map(e => (
            <div key={e.id} style={{
              display: 'flex', alignItems: 'center', gap: 8,
              padding: '6px 0', borderBottom: '1px solid rgba(255,255,255,0.03)',
              opacity: e.day_date < today ? 0.5 : 1,
            }}>
              <span style={{ flex: 1, fontSize: 12, color: 'var(--text)', lineHeight: 1.5 }}>{e.content}</span>
              <span
                onClick={() => deleteEntry(e.id)}
                style={{ color: 'var(--text-dim)', cursor: 'pointer', fontSize: 12, lineHeight: 1, flexShrink: 0 }}
                onMouseEnter={e2 => e2.target.style.color = 'var(--red)'}
                onMouseLeave={e2 => e2.target.style.color = 'var(--text-dim)'}
              >x</span>
            </div>
          ))}
          <div style={{ display: 'flex', gap: 6, marginTop: 8 }}>
            <input
              placeholder="Add for this date..."
              value={newDaily}
              onChange={e => setNewDaily(e.target.value)}
              onKeyDown={e => { if (e.key === 'Enter' && newDaily.trim()) { addEntry('daily', newDaily.trim(), selectedDate); setNewDaily('') } }}
              style={{
                flex: 1, fontFamily: 'var(--sans)', fontSize: 11,
                background: 'var(--bg)', color: 'var(--text)',
                border: '1px solid var(--border)', borderRadius: 'var(--radius-sm)',
                padding: '6px 8px', outline: 'none',
              }}
            />
            <button
              onClick={() => { if (newDaily.trim()) { addEntry('daily', newDaily.trim(), selectedDate); setNewDaily('') } }}
              disabled={!newDaily.trim() || adding}
              style={{
                padding: '4px 10px', fontSize: 10, fontWeight: 600,
                background: newDaily.trim() ? 'var(--accent)' : 'var(--border)',
                color: newDaily.trim() ? 'var(--bg)' : 'var(--text-dim)',
                border: 'none', borderRadius: 'var(--radius-sm)',
                cursor: newDaily.trim() ? 'pointer' : 'default',
              }}
            >Add</button>
          </div>
        </div>
      </div>
    </>
  )
}

// ─── No Chat Selected ────────────────────────────────────────────────────────

function NoChatSelected() {
  return (
    <div style={{
      flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center',
      flexDirection: 'column', gap: 8,
    }}>
      <div style={{ fontSize: 20, fontWeight: 700, color: 'var(--accent)', letterSpacing: '-0.02em' }}>
        GHOST
      </div>
      <div style={{ color: 'var(--text-dim)', fontSize: 12 }}>
        select or create a chat to begin
      </div>
    </div>
  )
}

// ─── Main Chat Area (with per-chat tabs) ─────────────────────────────────────

function ChatArea({
  chat, alive, selectedAgents, setSelectedAgents, running,
  onSendMessage, onClearMessages, agentsCollapsed, onAgentsCollapseToggle,
}) {
  const [innerTab, setInnerTab] = useState('chat')
  const tabs = ['Chat', 'Preview', 'Context', 'Thinking']

  function handleAgentToggle(agentId) {
    setSelectedAgents(prev => {
      if (agentId === 'echo') {
        // Toggling Echo: if it's the only one, keep it. If others are on, toggle echo off/on.
        if (prev.includes('echo')) {
          const without = prev.filter(id => id !== 'echo')
          return without.length === 0 ? ['echo'] : without
        }
        return [...prev, 'echo']
      }
      // Specialist toggle
      if (prev.includes(agentId)) {
        const without = prev.filter(id => id !== agentId)
        return without.length === 0 ? ['echo'] : without
      }
      // Max: echo + one specialist
      const specialists = prev.filter(id => id !== 'echo')
      if (specialists.length >= 1) {
        // Replace the specialist
        const hasEcho = prev.includes('echo')
        return hasEcho ? ['echo', agentId] : [agentId]
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

// ─── Main App ────────────────────────────────────────────────────────────────

export default function App() {
  // Auth
  const [daemonKey, setDaemonKey] = useState(() => {
    try { return localStorage.getItem(STORAGE_KEY) || '' } catch { return '' }
  })
  const [authed, setAuthed] = useState(() => !!daemonKey)

  // Daemon state
  const [alive, setAlive] = useState(false)
  const [status, setStatus] = useState(null)

  // Projects + chats (localStorage)
  const [projects, setProjects] = useState(loadProjects)
  const [activeChatId, setActiveChatId] = useState(null)
  const [openTabs, setOpenTabs] = useState([]) // [{id, name, projectId}]

  // Sidebar
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false)
  const [bottomRatio, setBottomRatio] = useState(0.25)
  const [activeNav, setActiveNav] = useState(null) // 'settings' | 'statistics' | 'about' | null

  // Chat state
  const [selectedAgents, setSelectedAgents] = useState(['echo'])
  const [agentsCollapsed, setAgentsCollapsed] = useState(false)
  const [running, setRunning] = useState(false)

  // Job banner
  const [activeJob, setActiveJob] = useState(null)

  const mountedRef = useRef(true)
  const promptAbortRef = useRef(null)

  // Persist projects
  useEffect(() => { saveProjects(projects) }, [projects])

  // Cleanup
  useEffect(() => {
    mountedRef.current = true
    return () => {
      mountedRef.current = false
      if (promptAbortRef.current) promptAbortRef.current.abort()
    }
  }, [])

  // Health poll
  const poll = useCallback(async () => {
    if (!daemonKey) return
    try {
      const s = await apiFetch('/status', { signal: AbortSignal.timeout(5_000) }, daemonKey)
      if (!mountedRef.current) return
      setStatus(s)
      setAlive(true)
    } catch {
      if (!mountedRef.current) return
      setAlive(false)
      setStatus(null)
    }
  }, [daemonKey])

  useEffect(() => {
    if (!authed) return
    poll()
    const id = setInterval(poll, 10_000)
    return () => clearInterval(id)
  }, [authed, poll])

  // Find active chat data
  const activeChat = useMemo(() => {
    for (const p of projects) {
      const c = p.chats.find(ch => ch.id === activeChatId)
      if (c) return c
    }
    return null
  }, [projects, activeChatId])

  // Open a chat (from sidebar click)
  function handleOpenChat(projectId, chatId) {
    setActiveChatId(chatId)
    setActiveNav(null) // close nav panels when opening a chat

    // Find chat name
    const project = projects.find(p => p.id === projectId)
    const chat = project?.chats.find(c => c.id === chatId)
    if (!chat) return

    setOpenTabs(prev => {
      if (prev.some(t => t.id === chatId)) return prev
      return [...prev, { id: chatId, name: chat.name, projectId }]
    })
  }

  function handleSelectTab(tabId) {
    setActiveChatId(tabId)
    setActiveNav(null)
  }

  function handleCloseTab(tabId) {
    setOpenTabs(prev => prev.filter(t => t.id !== tabId))
    if (activeChatId === tabId) {
      // Switch to the next tab or null
      setOpenTabs(prev => {
        const remaining = prev.filter(t => t.id !== tabId)
        if (remaining.length > 0) setActiveChatId(remaining[remaining.length - 1].id)
        else setActiveChatId(null)
        return remaining
      })
    }
  }

  // Send message
  async function handleSendMessage(text) {
    if (!text || running) return
    if (promptAbortRef.current) promptAbortRef.current.abort()
    const controller = new AbortController()
    promptAbortRef.current = controller

    // Add user message
    setProjects(prev => prev.map(p => ({
      ...p,
      chats: p.chats.map(c => {
        if (c.id !== activeChatId) return c
        return { ...c, messages: [...c.messages, { role: 'user', content: text }] }
      }),
    })))

    setRunning(true)
    const agentLabel = selectedAgents[0] || 'echo'

    // Show job banner
    const startTime = Date.now()
    setActiveJob({ agent: AGENTS.find(a => a.id === agentLabel)?.label ?? 'Echo', status: 'running', elapsed: 0 })
    const jobInterval = setInterval(() => {
      setActiveJob(prev => prev ? { ...prev, elapsed: Math.floor((Date.now() - startTime) / 1000) } : null)
    }, 1000)

    try {
      // Get current messages for history
      let currentMessages = []
      for (const p of projects) {
        const c = p.chats.find(ch => ch.id === activeChatId)
        if (c) { currentMessages = c.messages; break }
      }
      const history = currentMessages.slice(-10).map(m => ({ role: m.role, content: m.content }))

      const data = await apiFetch('/chat', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ message: text, history }),
        signal: controller.signal,
      }, daemonKey)

      if (!mountedRef.current) return

      setProjects(prev => prev.map(p => ({
        ...p,
        chats: p.chats.map(c => {
          if (c.id !== activeChatId) return c
          return {
            ...c,
            messages: [...c.messages, {
              role: 'assistant',
              content: data.response,
              job_id: data.job_id,
              agent: agentLabel,
            }],
          }
        }),
      })))

      setActiveJob(prev => prev ? { ...prev, status: 'done' } : null)
      setTimeout(() => setActiveJob(null), 2000)
    } catch (e) {
      if (e.name === 'AbortError' || !mountedRef.current) return
      setProjects(prev => prev.map(p => ({
        ...p,
        chats: p.chats.map(c => {
          if (c.id !== activeChatId) return c
          return { ...c, messages: [...c.messages, { role: 'error', content: e.message }] }
        }),
      })))
      setActiveJob(null)
    } finally {
      clearInterval(jobInterval)
      if (mountedRef.current) setRunning(false)
      if (promptAbortRef.current === controller) promptAbortRef.current = null
    }
  }

  function handleClearMessages() {
    setProjects(prev => prev.map(p => ({
      ...p,
      chats: p.chats.map(c => {
        if (c.id !== activeChatId) return c
        return { ...c, messages: [] }
      }),
    })))
  }

  // Nav panel overrides main area
  function handleNav(name) {
    setActiveNav(prev => prev === name ? null : name)
  }

  // ── Auth gate ──
  if (!authed) {
    return <AuthScreen onAuth={key => { setDaemonKey(key); setAuthed(true) }} />
  }

  // ── Main layout ──
  const showBanner = !!activeJob

  return (
    <div style={{
      display: 'flex',
      flexDirection: 'column',
      height: '100vh',
      overflow: 'hidden',
      background: 'var(--bg)',
    }}>
      {/* Animations */}
      <style>{`
        @keyframes pulse { 0%, 100% { opacity: 1; } 50% { opacity: 0.35; } }
        @keyframes blink { 0%, 100% { opacity: 1; } 50% { opacity: 0; } }
      `}</style>

      <TopBar
        alive={alive}
        status={status}
        openTabs={openTabs}
        activeTabId={activeChatId}
        onSelectTab={handleSelectTab}
        onCloseTab={handleCloseTab}
      />

      {showBanner && <JobBanner job={activeJob} onDismiss={() => setActiveJob(null)} />}

      <div style={{
        display: 'flex',
        flex: 1,
        overflow: 'hidden',
        minHeight: 0,
      }}>
        <Sidebar
          collapsed={sidebarCollapsed}
          onToggle={() => setSidebarCollapsed(!sidebarCollapsed)}
          projects={projects}
          setProjects={setProjects}
          onOpenChat={handleOpenChat}
          activeChatId={activeChatId}
          activeNav={activeNav}
          onNav={handleNav}
          bottomRatio={bottomRatio}
          onBottomResize={setBottomRatio}
        />

        {/* Main area */}
        <div style={{
          flex: 1,
          display: 'flex',
          flexDirection: 'column',
          overflow: 'hidden',
          minWidth: 0,
          background: 'var(--bg)',
        }}>
          {activeNav === 'sms' && <SmsPanel daemonKey={daemonKey} />}
          {activeNav === 'settings' && <SettingsPanel />}
          {activeNav === 'statistics' && <StatisticsPanel />}
          {activeNav === 'about' && <AboutPanel />}
          {!activeNav && activeChat && (
            <ChatArea
              chat={activeChat}
              alive={alive}
              selectedAgents={selectedAgents}
              setSelectedAgents={setSelectedAgents}
              running={running}
              onSendMessage={handleSendMessage}
              onClearMessages={handleClearMessages}
              agentsCollapsed={agentsCollapsed}
              onAgentsCollapseToggle={() => setAgentsCollapsed(!agentsCollapsed)}
            />
          )}
          {!activeNav && !activeChat && <NoChatSelected />}
        </div>
      </div>
    </div>
  )
}
