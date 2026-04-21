import { useState, useEffect, useRef } from 'react'
import { uid } from '../lib/api.js'

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

function SidebarNav({ activeNav, onNav }) {
  const items = ['SMS', 'Events', 'Budget', 'Agents', 'Settings', 'Statistics', 'About']
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

export default function Sidebar({ collapsed, onToggle, projects, setProjects, onOpenChat, activeChatId, activeNav, onNav, bottomRatio, onBottomResize }) {
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
