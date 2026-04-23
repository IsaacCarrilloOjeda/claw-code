import { useState, useEffect, useRef } from 'react'
import ReactMarkdown from 'react-markdown'
import { apiFetch, timeAgo, uid, QUICK_REPLIES } from '../lib/api.js'

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

function SmsContactRow({ contact, active, onSelect, onToggleAutoReply, onRename, onCycleSlot }) {
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
      {/* Schedule slot indicator: click to cycle None → A → B → C → None. */}
      <div
        onClick={e => { e.stopPropagation(); onCycleSlot() }}
        title={contact.schedule_slot
          ? `Schedule ${contact.schedule_slot} — click to change`
          : 'No schedule — click to assign A/B/C'}
        style={{
          width: 20, height: 20, flexShrink: 0,
          display: 'flex', alignItems: 'center', justifyContent: 'center',
          fontSize: 10, fontWeight: 700, fontFamily: 'var(--mono)',
          color: contact.schedule_slot ? 'var(--bg)' : 'var(--text-dim)',
          background: contact.schedule_slot ? 'var(--accent)' : 'transparent',
          border: '1px solid ' + (contact.schedule_slot ? 'var(--accent)' : 'var(--border)'),
          borderRadius: 'var(--radius-sm)', cursor: 'pointer',
          transition: 'all var(--transition)',
        }}
      >{contact.schedule_slot || '—'}</div>

      {/* Auto-reply toggle */}
      <div
        onClick={e => { e.stopPropagation(); onToggleAutoReply(!contact.auto_reply) }}
        style={{
          width: 28, height: 16, flexShrink: 0,
          background: contact.auto_reply ? 'var(--accent)' : 'var(--border)',
          borderRadius: 8, cursor: 'pointer',
          position: 'relative', transition: 'background var(--transition)',
        }}
        title={contact.auto_reply ? 'Auto-reply ON (manual toggle clears slot)' : 'Auto-reply OFF (manual toggle clears slot)'}
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

      {/* Unread badge + time ago */}
      <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'flex-end', gap: 3, flexShrink: 0 }}>
        {contact.unread_count > 0 && (
          <span style={{
            minWidth: 16, height: 16, borderRadius: 8,
            background: 'var(--accent)', color: 'var(--bg)',
            fontSize: 9, fontWeight: 700, fontFamily: 'var(--mono)',
            display: 'flex', alignItems: 'center', justifyContent: 'center',
            padding: '0 4px',
          }}>{contact.unread_count}</span>
        )}
        {contact.last_message_at && (
          <span style={{ fontSize: 9, color: 'var(--text-dim)', fontFamily: 'var(--mono)' }}>
            {timeAgo(contact.last_message_at)}
          </span>
        )}
      </div>
    </div>
  )
}

function SmsConversation({ contact, convo, daemonKey, onSend, onLoadMore, onUpdateNotes }) {
  const [input, setInput] = useState('')
  const [showQuickReplies, setShowQuickReplies] = useState(false)
  const [editingNotes, setEditingNotes] = useState(false)
  const [notesValue, setNotesValue] = useState(contact.notes || '')
  const [summary, setSummary] = useState(null)
  const [summaryLoading, setSummaryLoading] = useState(false)
  const scrollRef = useRef(null)
  const textareaRef = useRef(null)
  const notesRef = useRef(null)
  const prevHeightRef = useRef(0)
  const isInitialLoad = useRef(true)

  // Reset notes when contact changes
  useEffect(() => { setNotesValue(contact.notes || ''); setEditingNotes(false); setSummary(null) }, [contact.phone])

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

  async function loadSummary() {
    setSummaryLoading(true)
    try {
      const data = await apiFetch(`/sms/contacts/${encodeURIComponent(contact.phone)}/summary`, {}, daemonKey)
      setSummary(data.summary || 'No summary available.')
    } catch { setSummary('Failed to load summary.') }
    setSummaryLoading(false)
  }

  function commitNotes() {
    const trimmed = notesValue.trim()
    if (trimmed !== (contact.notes || '').trim()) onUpdateNotes(trimmed)
    setEditingNotes(false)
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
      }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
          <div style={{ flex: 1, minWidth: 0 }}>
            <div style={{ fontSize: 13, fontWeight: 600, color: 'var(--text-bright)' }}>{displayName}</div>
            <div style={{ fontSize: 10, color: 'var(--text-dim)', fontFamily: 'var(--mono)' }}>{contact.phone}</div>
          </div>
          <span
            onClick={loadSummary}
            style={{
              fontSize: 9, fontWeight: 600, letterSpacing: '0.04em',
              padding: '3px 8px', borderRadius: 'var(--radius-sm)',
              background: 'rgba(255,255,255,0.04)', color: 'var(--text-dim)',
              cursor: 'pointer', transition: 'all var(--transition)',
            }}
            onMouseEnter={e => { e.target.style.color = 'var(--accent)'; e.target.style.background = 'rgba(45,212,191,0.12)' }}
            onMouseLeave={e => { e.target.style.color = 'var(--text-dim)'; e.target.style.background = 'rgba(255,255,255,0.04)' }}
          >{summaryLoading ? '...' : 'SUMMARY'}</span>
          <div style={{
            fontSize: 9, fontWeight: 600, letterSpacing: '0.04em',
            padding: '3px 8px', borderRadius: 'var(--radius-sm)',
            background: contact.auto_reply ? 'rgba(45,212,191,0.12)' : 'rgba(255,255,255,0.04)',
            color: contact.auto_reply ? 'var(--accent)' : 'var(--text-dim)',
          }}>
            {contact.auto_reply ? 'AUTO-REPLY ON' : 'AUTO-REPLY OFF'}
          </div>
        </div>

        {/* Contact notes */}
        <div style={{ marginTop: 6 }}>
          {editingNotes ? (
            <div style={{ display: 'flex', gap: 6, alignItems: 'flex-start' }}>
              <textarea
                ref={notesRef}
                autoFocus
                value={notesValue}
                onChange={e => setNotesValue(e.target.value)}
                onBlur={commitNotes}
                onKeyDown={e => { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); commitNotes() }; if (e.key === 'Escape') { setNotesValue(contact.notes || ''); setEditingNotes(false) } }}
                placeholder="Add notes about this contact..."
                rows={2}
                style={{
                  flex: 1, fontSize: 10, fontFamily: 'var(--mono)',
                  background: 'var(--bg)', color: 'var(--text)',
                  border: '1px solid var(--accent)', borderRadius: 'var(--radius-sm)',
                  padding: '4px 8px', outline: 'none', resize: 'none',
                  lineHeight: 1.5,
                }}
              />
            </div>
          ) : (
            <div
              onClick={() => setEditingNotes(true)}
              style={{
                fontSize: 10, color: contact.notes ? 'var(--text-dim)' : 'var(--text-muted)',
                fontFamily: 'var(--mono)', cursor: 'pointer',
                padding: '2px 0', fontStyle: contact.notes ? 'normal' : 'italic',
              }}
            >{contact.notes || 'click to add notes...'}</div>
          )}
        </div>

        {/* Summary overlay */}
        {summary && (
          <div style={{
            marginTop: 8, padding: '8px 10px',
            background: 'var(--bg)', border: '1px solid var(--border)',
            borderRadius: 'var(--radius-sm)', fontSize: 11, color: 'var(--text)',
            lineHeight: 1.5, fontFamily: 'var(--mono)', position: 'relative',
          }}>
            <span
              onClick={() => setSummary(null)}
              style={{
                position: 'absolute', top: 4, right: 8, cursor: 'pointer',
                color: 'var(--text-dim)', fontSize: 12,
              }}
            >{'\u00D7'}</span>
            {summary}
          </div>
        )}
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

      {/* Quick replies */}
      <div style={{ flexShrink: 0, background: 'var(--surface)', borderTop: '1px solid var(--border)' }}>
        <div
          onClick={() => setShowQuickReplies(!showQuickReplies)}
          style={{
            padding: '4px 16px', cursor: 'pointer', fontSize: 9, fontWeight: 600,
            color: showQuickReplies ? 'var(--accent)' : 'var(--text-dim)',
            letterSpacing: '0.04em', userSelect: 'none',
          }}
        >{showQuickReplies ? '\u25BC QUICK' : '\u25B6 QUICK'}</div>
        {showQuickReplies && (
          <div style={{ padding: '0 16px 8px', display: 'flex', gap: 6, flexWrap: 'wrap' }}>
            {QUICK_REPLIES.map(qr => (
              <button
                key={qr}
                onClick={() => onSend(qr)}
                style={{
                  fontSize: 10, fontFamily: 'var(--mono)', padding: '4px 10px',
                  background: 'var(--bg)', color: 'var(--text)',
                  border: '1px solid var(--border)', borderRadius: 12,
                  cursor: 'pointer', transition: 'all var(--transition)',
                }}
                onMouseEnter={e => { e.target.style.borderColor = 'var(--accent)'; e.target.style.color = 'var(--accent)' }}
                onMouseLeave={e => { e.target.style.borderColor = 'var(--border)'; e.target.style.color = 'var(--text)' }}
              >{qr}</button>
            ))}
          </div>
        )}
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
      setEntries(Array.isArray(data) ? data : data.entries || [])
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

function SmsFactsPanel({ daemonKey, onClose }) {
  const [content, setContent] = useState('')
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [saved, setSaved] = useState(false)
  const [dirty, setDirty] = useState(false)

  useEffect(() => {
    let cancelled = false
    ;(async () => {
      try {
        const data = await apiFetch('/facts', {}, daemonKey)
        if (!cancelled) setContent(data?.content || '')
      } catch { /* ignore */ }
      if (!cancelled) setLoading(false)
    })()
    return () => { cancelled = true }
  }, [daemonKey])

  async function save() {
    setSaving(true)
    setSaved(false)
    try {
      await apiFetch('/facts', {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ content }),
      }, daemonKey)
      setSaved(true)
      setDirty(false)
      setTimeout(() => setSaved(false), 1500)
    } catch { /* ignore */ }
    setSaving(false)
  }

  return (
    <>
      <div
        onClick={onClose}
        style={{ position: 'absolute', inset: 0, background: 'rgba(0,0,0,0.4)', zIndex: 10 }}
      />
      <div style={{
        position: 'absolute', top: 0, right: 0, bottom: 0,
        width: 360, background: 'var(--surface)',
        borderLeft: '1px solid var(--border)',
        zIndex: 11, display: 'flex', flexDirection: 'column',
        overflow: 'hidden',
      }}>
        <div style={{
          flexShrink: 0, padding: '14px 16px',
          borderBottom: '1px solid var(--border)',
          display: 'flex', alignItems: 'center', justifyContent: 'space-between',
        }}>
          <span style={{ fontSize: 13, fontWeight: 600, color: 'var(--text-bright)' }}>Facts</span>
          <span
            onClick={onClose}
            style={{ color: 'var(--text-dim)', cursor: 'pointer', fontSize: 14, lineHeight: 1 }}
            onMouseEnter={e => e.target.style.color = 'var(--text)'}
            onMouseLeave={e => e.target.style.color = 'var(--text-dim)'}
          >x</span>
        </div>

        <div style={{ flex: 1, display: 'flex', flexDirection: 'column', padding: '16px', gap: 10, overflow: 'hidden' }}>
          <div style={{ fontSize: 11, color: 'var(--text-dim)', lineHeight: 1.5 }}>
            Shareable facts about you. GHOST uses these to answer questions on your behalf,
            and the outbound guard treats anything here as approved to share.
            <br /><br />
            Do NOT put passwords, addresses, or financial info — those are blocked regardless.
          </div>
          <textarea
            value={content}
            onChange={e => { setContent(e.target.value); setDirty(true) }}
            disabled={loading}
            placeholder={loading ? 'loading...' : 'Example:\n- In school until 3:30 on weekdays\n- Usually free after 4pm\n- Works on RC Concrete job sites Saturdays\n- Prefers text over call'}
            style={{
              flex: 1, minHeight: 0, resize: 'none',
              fontFamily: 'var(--sans)', fontSize: 12, lineHeight: 1.5,
              background: 'var(--bg)', color: 'var(--text)',
              border: '1px solid var(--border)', borderRadius: 'var(--radius-sm)',
              padding: '10px 12px', outline: 'none',
            }}
          />
          <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
            <button
              onClick={save}
              disabled={saving || loading || !dirty}
              style={{
                padding: '6px 14px', fontSize: 11, fontWeight: 600,
                background: (dirty && !saving) ? 'var(--accent)' : 'var(--border)',
                color: (dirty && !saving) ? 'var(--bg)' : 'var(--text-dim)',
                border: 'none', borderRadius: 'var(--radius-sm)',
                cursor: (dirty && !saving) ? 'pointer' : 'default',
              }}
            >{saving ? 'saving...' : 'SAVE'}</button>
            {saved && <span style={{ fontSize: 10, color: 'var(--accent)' }}>saved</span>}
            {dirty && !saved && !saving && <span style={{ fontSize: 10, color: 'var(--text-dim)' }}>unsaved</span>}
            <span style={{ marginLeft: 'auto', fontSize: 10, color: 'var(--text-dim)' }}>
              {content.length}/16384
            </span>
          </div>
        </div>
      </div>
    </>
  )
}

// ---------------------------------------------------------------------------
// Availability schedules (slots A/B/C) + sleep mode
// ---------------------------------------------------------------------------

// Weekday mask bits: Sun=0, Mon=1, ..., Sat=6 (matches PostgreSQL EXTRACT(DOW)).
const WEEKDAYS = [
  { label: 'Mon', bit: 1 },
  { label: 'Tue', bit: 2 },
  { label: 'Wed', bit: 3 },
  { label: 'Thu', bit: 4 },
  { label: 'Fri', bit: 5 },
  { label: 'Sat', bit: 6 },
  { label: 'Sun', bit: 0 },
]

function maskToLabel(mask) {
  if (mask == null) return ''
  const weekdayBits = (1 << 1) | (1 << 2) | (1 << 3) | (1 << 4) | (1 << 5)
  const weekendBits = (1 << 0) | (1 << 6)
  if (mask === weekdayBits) return 'Mon–Fri'
  if (mask === weekendBits) return 'Sat–Sun'
  if (mask === weekdayBits | weekendBits) return 'Every day'
  return WEEKDAYS.filter(d => (mask & (1 << d.bit)) !== 0).map(d => d.label).join(' ')
}

function WindowForm({ slot, kind, onAdd, onCancel }) {
  const [mask, setMask] = useState((1 << 1) | (1 << 2) | (1 << 3) | (1 << 4) | (1 << 5))
  const [date, setDate] = useState(() => new Date().toISOString().slice(0, 10))
  const [start, setStart] = useState('08:00')
  const [end, setEnd] = useState('15:00')
  const valid = start < end && (kind === 'oneoff' || mask > 0)

  function toggleBit(bit) {
    setMask(m => m ^ (1 << bit))
  }

  return (
    <div style={{
      display: 'flex', flexDirection: 'column', gap: 6,
      padding: 8, marginTop: 6,
      background: 'var(--bg)', border: '1px solid var(--border)',
      borderRadius: 'var(--radius-sm)',
    }}>
      {kind === 'weekly' ? (
        <div style={{ display: 'flex', gap: 4, flexWrap: 'wrap' }}>
          {WEEKDAYS.map(d => {
            const on = (mask & (1 << d.bit)) !== 0
            return (
              <span
                key={d.label}
                onClick={() => toggleBit(d.bit)}
                style={{
                  padding: '3px 6px', fontSize: 10, fontWeight: 600,
                  fontFamily: 'var(--mono)', cursor: 'pointer',
                  background: on ? 'var(--accent)' : 'transparent',
                  color: on ? 'var(--bg)' : 'var(--text-dim)',
                  border: '1px solid ' + (on ? 'var(--accent)' : 'var(--border)'),
                  borderRadius: 'var(--radius-sm)',
                }}
              >{d.label}</span>
            )
          })}
        </div>
      ) : (
        <input
          type="date" value={date} onChange={e => setDate(e.target.value)}
          style={{
            fontFamily: 'var(--mono)', fontSize: 11,
            background: 'var(--surface)', color: 'var(--text)',
            border: '1px solid var(--border)', borderRadius: 'var(--radius-sm)',
            padding: '4px 6px', outline: 'none', colorScheme: 'dark',
          }}
        />
      )}
      <div style={{ display: 'flex', gap: 6, alignItems: 'center' }}>
        <input
          type="time" value={start} onChange={e => setStart(e.target.value)}
          style={{
            flex: 1, fontFamily: 'var(--mono)', fontSize: 11,
            background: 'var(--surface)', color: 'var(--text)',
            border: '1px solid var(--border)', borderRadius: 'var(--radius-sm)',
            padding: '4px 6px', outline: 'none', colorScheme: 'dark',
          }}
        />
        <span style={{ color: 'var(--text-dim)', fontSize: 10 }}>to</span>
        <input
          type="time" value={end} onChange={e => setEnd(e.target.value)}
          style={{
            flex: 1, fontFamily: 'var(--mono)', fontSize: 11,
            background: 'var(--surface)', color: 'var(--text)',
            border: '1px solid var(--border)', borderRadius: 'var(--radius-sm)',
            padding: '4px 6px', outline: 'none', colorScheme: 'dark',
          }}
        />
      </div>
      <div style={{ display: 'flex', gap: 6 }}>
        <button
          disabled={!valid}
          onClick={() => onAdd({
            slot, kind,
            weekday_mask: kind === 'weekly' ? mask : null,
            day_date: kind === 'oneoff' ? date : null,
            start_time: start, end_time: end,
          })}
          style={{
            flex: 1, padding: '5px 0', fontSize: 10, fontWeight: 600,
            background: valid ? 'var(--accent)' : 'var(--border)',
            color: valid ? 'var(--bg)' : 'var(--text-dim)',
            border: 'none', borderRadius: 'var(--radius-sm)',
            cursor: valid ? 'pointer' : 'default',
          }}
        >ADD</button>
        <button
          onClick={onCancel}
          style={{
            padding: '5px 10px', fontSize: 10,
            background: 'transparent', color: 'var(--text-dim)',
            border: '1px solid var(--border)', borderRadius: 'var(--radius-sm)',
            cursor: 'pointer',
          }}
        >cancel</button>
      </div>
    </div>
  )
}

function SlotCard({ slot, onRename, onAddWindow, onDeleteWindow }) {
  const [editingName, setEditingName] = useState(false)
  const [nameValue, setNameValue] = useState(slot.name || '')
  const [adding, setAdding] = useState(null) // 'weekly' | 'oneoff' | null

  useEffect(() => { setNameValue(slot.name || '') }, [slot.name])

  function commitName() {
    const trimmed = nameValue.trim()
    if (trimmed !== (slot.name || '').trim()) onRename(trimmed)
    setEditingName(false)
  }

  return (
    <div style={{
      marginBottom: 14, padding: 10,
      background: 'var(--bg)', border: '1px solid var(--border)',
      borderRadius: 'var(--radius-sm)',
    }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 6 }}>
        <span style={{
          width: 22, height: 22, display: 'flex',
          alignItems: 'center', justifyContent: 'center',
          background: 'var(--accent)', color: 'var(--bg)',
          fontSize: 12, fontWeight: 700, fontFamily: 'var(--mono)',
          borderRadius: 'var(--radius-sm)', flexShrink: 0,
        }}>{slot.slot}</span>
        {editingName ? (
          <input
            autoFocus value={nameValue}
            onChange={e => setNameValue(e.target.value)}
            onBlur={commitName}
            onKeyDown={e => { if (e.key === 'Enter') commitName(); if (e.key === 'Escape') { setNameValue(slot.name || ''); setEditingName(false) } }}
            placeholder="e.g. School hours"
            style={{
              flex: 1, fontFamily: 'var(--sans)', fontSize: 12,
              background: 'var(--surface)', color: 'var(--text)',
              border: '1px solid var(--accent)', borderRadius: 'var(--radius-sm)',
              padding: '3px 6px', outline: 'none',
            }}
          />
        ) : (
          <span
            onClick={() => setEditingName(true)}
            style={{
              flex: 1, fontSize: 12, fontWeight: 600, cursor: 'pointer',
              color: slot.name ? 'var(--text-bright)' : 'var(--text-dim)',
              fontStyle: slot.name ? 'normal' : 'italic',
            }}
          >{slot.name || 'click to name...'}</span>
        )}
      </div>

      {slot.windows.length === 0 && (
        <div style={{ fontSize: 10, color: 'var(--text-dim)', padding: '4px 0' }}>no windows</div>
      )}
      {slot.windows.map(w => (
        <div key={w.id} style={{
          display: 'flex', alignItems: 'center', gap: 8,
          padding: '3px 0', borderBottom: '1px solid rgba(255,255,255,0.03)',
        }}>
          <span style={{ flex: 1, fontSize: 11, fontFamily: 'var(--mono)', color: 'var(--text)' }}>
            {w.kind === 'weekly' ? maskToLabel(w.weekday_mask) : w.day_date}
            {' '}
            <span style={{ color: 'var(--accent)' }}>{w.start_time}–{w.end_time}</span>
          </span>
          <span
            onClick={() => onDeleteWindow(w.id)}
            style={{ color: 'var(--text-dim)', cursor: 'pointer', fontSize: 12, lineHeight: 1 }}
            onMouseEnter={e => e.target.style.color = 'var(--red)'}
            onMouseLeave={e => e.target.style.color = 'var(--text-dim)'}
          >x</span>
        </div>
      ))}

      {adding ? (
        <WindowForm
          slot={slot.slot} kind={adding}
          onAdd={async (payload) => { await onAddWindow(payload); setAdding(null) }}
          onCancel={() => setAdding(null)}
        />
      ) : (
        <div style={{ display: 'flex', gap: 6, marginTop: 8 }}>
          <button
            onClick={() => setAdding('weekly')}
            style={{
              flex: 1, padding: '5px 0', fontSize: 10, fontWeight: 600,
              background: 'transparent', color: 'var(--text-dim)',
              border: '1px dashed var(--border)', borderRadius: 'var(--radius-sm)',
              cursor: 'pointer', letterSpacing: '0.04em',
            }}
          >+ WEEKLY</button>
          <button
            onClick={() => setAdding('oneoff')}
            style={{
              flex: 1, padding: '5px 0', fontSize: 10, fontWeight: 600,
              background: 'transparent', color: 'var(--text-dim)',
              border: '1px dashed var(--border)', borderRadius: 'var(--radius-sm)',
              cursor: 'pointer', letterSpacing: '0.04em',
            }}
          >+ ONE-OFF</button>
        </div>
      )}
    </div>
  )
}

function SmsAvailabilityPanel({ daemonKey, onClose }) {
  const [slots, setSlots] = useState([])
  const [loading, setLoading] = useState(true)

  async function reload() {
    try {
      const data = await apiFetch('/sms/availability', {}, daemonKey)
      setSlots(data.slots || [])
    } catch { /* ignore */ }
    setLoading(false)
  }

  useEffect(() => { reload() }, [daemonKey])

  async function renameSlot(slot, name) {
    try {
      await apiFetch(`/sms/availability/slots/${slot}`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ name }),
      }, daemonKey)
      reload()
    } catch { /* ignore */ }
  }

  async function addWindow(payload) {
    try {
      await apiFetch('/sms/availability/windows', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(payload),
      }, daemonKey)
      reload()
    } catch { /* ignore */ }
  }

  async function deleteWindow(id) {
    try {
      await apiFetch(`/sms/availability/windows/${id}`, { method: 'DELETE' }, daemonKey)
      reload()
    } catch { /* ignore */ }
  }

  return (
    <>
      <div
        onClick={onClose}
        style={{ position: 'absolute', inset: 0, background: 'rgba(0,0,0,0.4)', zIndex: 10 }}
      />
      <div style={{
        position: 'absolute', top: 0, right: 0, bottom: 0,
        width: 380, background: 'var(--surface)',
        borderLeft: '1px solid var(--border)',
        zIndex: 11, display: 'flex', flexDirection: 'column', overflow: 'hidden',
      }}>
        <div style={{
          flexShrink: 0, padding: '14px 16px',
          borderBottom: '1px solid var(--border)',
          display: 'flex', alignItems: 'center', justifyContent: 'space-between',
        }}>
          <div>
            <div style={{ fontSize: 13, fontWeight: 600, color: 'var(--text-bright)' }}>Availability</div>
            <div style={{ fontSize: 9, color: 'var(--text-dim)', marginTop: 2 }}>
              times evaluated in Mountain Time · rep turns ON inside windows
            </div>
          </div>
          <span
            onClick={onClose}
            style={{ color: 'var(--text-dim)', cursor: 'pointer', fontSize: 14, lineHeight: 1 }}
            onMouseEnter={e => e.target.style.color = 'var(--text)'}
            onMouseLeave={e => e.target.style.color = 'var(--text-dim)'}
          >x</span>
        </div>

        <div style={{ flex: 1, overflowY: 'auto', padding: 14 }}>
          {loading && <div style={{ color: 'var(--text-dim)', fontSize: 11 }}>loading...</div>}
          {!loading && slots.map(s => (
            <SlotCard
              key={s.slot}
              slot={s}
              onRename={name => renameSlot(s.slot, name)}
              onAddWindow={addWindow}
              onDeleteWindow={deleteWindow}
            />
          ))}
        </div>
      </div>
    </>
  )
}

function SmsSleepPanel({ daemonKey, contacts, onClose }) {
  const [sleep, setSleep] = useState(null)
  const [loading, setLoading] = useState(true)
  const [sleepList, setSleepList] = useState([])
  const [awakeBy, setAwakeBy] = useState('07:00')
  const [addPhone, setAddPhone] = useState('')

  async function reload() {
    try {
      const [s, l] = await Promise.all([
        apiFetch('/sms/sleep', {}, daemonKey),
        apiFetch('/sms/sleep/contacts', {}, daemonKey),
      ])
      setSleep(s)
      setSleepList(l.phones || [])
    } catch { /* ignore */ }
    setLoading(false)
  }

  useEffect(() => { reload() }, [daemonKey])

  async function start() {
    try {
      await apiFetch('/sms/sleep/start', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ awake_by_local: awakeBy }),
      }, daemonKey)
      reload()
    } catch { /* ignore */ }
  }

  async function end() {
    try {
      await apiFetch('/sms/sleep/end', { method: 'POST' }, daemonKey)
      reload()
    } catch { /* ignore */ }
  }

  async function addContact() {
    const phone = addPhone.trim()
    if (!phone) return
    try {
      await apiFetch('/sms/sleep/contacts', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ phone }),
      }, daemonKey)
      setAddPhone('')
      reload()
    } catch { /* ignore */ }
  }

  async function removeContact(phone) {
    try {
      await apiFetch(`/sms/sleep/contacts/${encodeURIComponent(phone)}`, { method: 'DELETE' }, daemonKey)
      reload()
    } catch { /* ignore */ }
  }

  const active = sleep?.active
  const awakeAt = sleep?.awake_by
    ? new Date(sleep.awake_by).toLocaleString([], { weekday: 'short', hour: '2-digit', minute: '2-digit' })
    : null

  // Suggest contacts not already in sleep list.
  const candidates = contacts.filter(c => !sleepList.includes(c.phone))

  return (
    <>
      <div
        onClick={onClose}
        style={{ position: 'absolute', inset: 0, background: 'rgba(0,0,0,0.4)', zIndex: 10 }}
      />
      <div style={{
        position: 'absolute', top: 0, right: 0, bottom: 0,
        width: 360, background: 'var(--surface)',
        borderLeft: '1px solid var(--border)',
        zIndex: 11, display: 'flex', flexDirection: 'column', overflow: 'hidden',
      }}>
        <div style={{
          flexShrink: 0, padding: '14px 16px',
          borderBottom: '1px solid var(--border)',
          display: 'flex', alignItems: 'center', justifyContent: 'space-between',
        }}>
          <span style={{ fontSize: 13, fontWeight: 600, color: 'var(--text-bright)' }}>Sleep mode</span>
          <span
            onClick={onClose}
            style={{ color: 'var(--text-dim)', cursor: 'pointer', fontSize: 14, lineHeight: 1 }}
            onMouseEnter={e => e.target.style.color = 'var(--text)'}
            onMouseLeave={e => e.target.style.color = 'var(--text-dim)'}
          >x</span>
        </div>

        <div style={{ flex: 1, overflowY: 'auto', padding: 14 }}>
          {loading && <div style={{ color: 'var(--text-dim)', fontSize: 11 }}>loading...</div>}

          {!loading && active && (
            <div style={{
              padding: 12, marginBottom: 16,
              background: 'var(--accent-dim)',
              border: '1px solid var(--accent)',
              borderRadius: 'var(--radius-sm)',
            }}>
              <div style={{ fontSize: 12, fontWeight: 600, color: 'var(--accent)', marginBottom: 4 }}>
                SLEEPING
              </div>
              <div style={{ fontSize: 11, color: 'var(--text)', marginBottom: 10 }}>
                Rep is covering {sleepList.length} contact{sleepList.length === 1 ? '' : 's'}
                {awakeAt && <> until <strong>{awakeAt}</strong></>}.
              </div>
              <button
                onClick={end}
                style={{
                  padding: '6px 14px', fontSize: 11, fontWeight: 600,
                  background: 'var(--surface)', color: 'var(--text)',
                  border: '1px solid var(--border)', borderRadius: 'var(--radius-sm)',
                  cursor: 'pointer',
                }}
              >Wake now</button>
            </div>
          )}

          {!loading && !active && (
            <div style={{ marginBottom: 16 }}>
              <div style={{ fontSize: 9, fontWeight: 600, color: 'var(--text-dim)', letterSpacing: '0.08em', marginBottom: 6 }}>
                AWAKE BY
              </div>
              <div style={{ display: 'flex', gap: 6, alignItems: 'center' }}>
                <input
                  type="time" value={awakeBy} onChange={e => setAwakeBy(e.target.value)}
                  style={{
                    flex: 1, fontFamily: 'var(--mono)', fontSize: 12,
                    background: 'var(--bg)', color: 'var(--text)',
                    border: '1px solid var(--border)', borderRadius: 'var(--radius-sm)',
                    padding: '6px 8px', outline: 'none', colorScheme: 'dark',
                  }}
                />
                <button
                  onClick={start}
                  disabled={sleepList.length === 0}
                  title={sleepList.length === 0 ? 'add at least one contact to the sleep list' : ''}
                  style={{
                    padding: '6px 14px', fontSize: 11, fontWeight: 600,
                    background: sleepList.length > 0 ? 'var(--accent)' : 'var(--border)',
                    color: sleepList.length > 0 ? 'var(--bg)' : 'var(--text-dim)',
                    border: 'none', borderRadius: 'var(--radius-sm)',
                    cursor: sleepList.length > 0 ? 'pointer' : 'default',
                  }}
                >SLEEP NOW</button>
              </div>
              <div style={{ fontSize: 10, color: 'var(--text-dim)', marginTop: 6, lineHeight: 1.5 }}>
                Time is Mountain Time. Press immediately activates sleep mode; wake-by is when it auto-ends.
              </div>
            </div>
          )}

          <div style={{ fontSize: 9, fontWeight: 600, color: 'var(--text-dim)', letterSpacing: '0.08em', marginBottom: 6 }}>
            SLEEP LIST
          </div>
          {sleepList.length === 0 && (
            <div style={{ fontSize: 11, color: 'var(--text-dim)', marginBottom: 8 }}>
              no contacts — add some so they get auto-reply while you sleep
            </div>
          )}
          {sleepList.map(phone => {
            const c = contacts.find(x => x.phone === phone)
            return (
              <div key={phone} style={{
                display: 'flex', alignItems: 'center', gap: 8,
                padding: '5px 0', borderBottom: '1px solid rgba(255,255,255,0.03)',
              }}>
                <span style={{ flex: 1, fontSize: 11, color: 'var(--text)' }}>
                  {c?.display_name || phone}
                  {c?.display_name && <span style={{ color: 'var(--text-dim)', marginLeft: 6, fontFamily: 'var(--mono)', fontSize: 9 }}>{phone}</span>}
                </span>
                <span
                  onClick={() => removeContact(phone)}
                  style={{ color: 'var(--text-dim)', cursor: 'pointer', fontSize: 12, lineHeight: 1 }}
                  onMouseEnter={e => e.target.style.color = 'var(--red)'}
                  onMouseLeave={e => e.target.style.color = 'var(--text-dim)'}
                >x</span>
              </div>
            )
          })}

          <div style={{ display: 'flex', gap: 6, marginTop: 10 }}>
            <select
              value={addPhone}
              onChange={e => setAddPhone(e.target.value)}
              style={{
                flex: 1, fontFamily: 'var(--mono)', fontSize: 11,
                background: 'var(--bg)', color: 'var(--text)',
                border: '1px solid var(--border)', borderRadius: 'var(--radius-sm)',
                padding: '6px 8px', outline: 'none',
              }}
            >
              <option value="">Pick a contact...</option>
              {candidates.map(c => (
                <option key={c.phone} value={c.phone}>{c.display_name || c.phone}</option>
              ))}
            </select>
            <button
              onClick={addContact} disabled={!addPhone}
              style={{
                padding: '4px 10px', fontSize: 10, fontWeight: 600,
                background: addPhone ? 'var(--accent)' : 'var(--border)',
                color: addPhone ? 'var(--bg)' : 'var(--text-dim)',
                border: 'none', borderRadius: 'var(--radius-sm)',
                cursor: addPhone ? 'pointer' : 'default',
              }}
            >Add</button>
          </div>
        </div>
      </div>
    </>
  )
}

export default function SmsPanel({ daemonKey }) {
  const [contacts, setContacts] = useState([])
  const [loading, setLoading] = useState(true)
  const [selectedPhone, setSelectedPhone] = useState(null)
  const [search, setSearch] = useState('')
  const [showSchedule, setShowSchedule] = useState(false)
  const [showFacts, setShowFacts] = useState(false)
  const [showAvailability, setShowAvailability] = useState(false)
  const [showSleep, setShowSleep] = useState(false)
  const [showAddForm, setShowAddForm] = useState(false)
  const [convos, setConvos] = useState({}) // { [phone]: { messages: [], hasMore: bool, loading: bool } }
  const [scheduleEntries, setScheduleEntries] = useState([])
  const selectedPhoneRef = useRef(null)

  // Keep ref in sync for polling callback
  useEffect(() => { selectedPhoneRef.current = selectedPhone }, [selectedPhone])

  useEffect(() => { loadContacts() }, [])

  // Auto-poll every 20s for new messages
  useEffect(() => {
    const interval = setInterval(() => {
      loadContacts()
      if (selectedPhoneRef.current) loadConversation(selectedPhoneRef.current)
    }, 20000)
    return () => clearInterval(interval)
  }, [])

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
      // Only show loading spinner if no messages cached yet (prevents flash on poll/refresh)
      setConvos(prev => {
        const existing = prev[key]?.messages || []
        return { ...prev, [key]: { messages: existing, hasMore: true, loading: existing.length === 0 } }
      })
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

  async function updateContactNotes(phone, notes) {
    try {
      await apiFetch(`/sms/contacts/${encodeURIComponent(phone)}/notes`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ notes }),
      }, daemonKey)
      setContacts(prev => prev.map(c => c.phone === phone ? { ...c, notes } : c))
    } catch { /* ignore */ }
  }

  async function loadSchedule() {
    try {
      const data = await apiFetch('/schedule', {}, daemonKey)
      setScheduleEntries(Array.isArray(data) ? data : data.entries || [])
    } catch { /* ignore */ }
  }

  async function cycleSlot(phone) {
    const current = contacts.find(c => c.phone === phone)?.schedule_slot || null
    // None → A → B → C → None
    const next = current === null ? 'A'
      : current === 'A' ? 'B'
      : current === 'B' ? 'C'
      : null
    // Optimistic update: set new slot and also flip auto_reply ON when assigning
    // a slot so the UI feels live (tick will re-evaluate in <=60s regardless).
    setContacts(prev => prev.map(c => c.phone === phone ? { ...c, schedule_slot: next } : c))
    try {
      await apiFetch(`/sms/contacts/${encodeURIComponent(phone)}/schedule-slot`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ slot: next }),
      }, daemonKey)
      // Refresh to reflect whatever auto_reply the scheduler decided.
      loadContacts()
    } catch {
      // Revert
      setContacts(prev => prev.map(c => c.phone === phone ? { ...c, schedule_slot: current } : c))
    }
  }

  function handleSelectContact(phone) {
    setSelectedPhone(phone)
    if (!convos[phone]) loadConversation(phone)
    // Mark as read
    apiFetch(`/sms/contacts/${encodeURIComponent(phone)}/read`, { method: 'POST' }, daemonKey).catch(() => {})
    setContacts(prev => prev.map(c => c.phone === phone ? { ...c, unread_count: 0 } : c))
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
                flex: 1, minWidth: 0, fontFamily: 'var(--mono)', fontSize: 11,
                background: 'var(--surface)', color: 'var(--text)',
                border: '1px solid var(--border)', borderRadius: 'var(--radius-sm)',
                padding: '6px 8px', outline: 'none',
              }}
              onFocus={e => e.target.style.borderColor = 'var(--accent)'}
              onBlur={e => e.target.style.borderColor = 'var(--border)'}
            />
            <span
              onClick={() => { loadContacts(); if (selectedPhone) loadConversation(selectedPhone) }}
              style={{ color: 'var(--text-dim)', cursor: 'pointer', fontSize: 12, padding: '4px', lineHeight: 1, flexShrink: 0 }}
              onMouseEnter={e => e.target.style.color = 'var(--accent)'}
              onMouseLeave={e => e.target.style.color = 'var(--text-dim)'}
              title="Refresh"
            >{'\u21BB'}</span>
            <span
              onClick={() => setShowAddForm(!showAddForm)}
              style={{ color: 'var(--text-dim)', cursor: 'pointer', fontSize: 16, fontWeight: 600, padding: '2px 4px', lineHeight: 1, flexShrink: 0 }}
              onMouseEnter={e => e.target.style.color = 'var(--accent)'}
              onMouseLeave={e => e.target.style.color = 'var(--text-dim)'}
              title="Add contact"
            >+</span>
          </div>
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 6 }}>
            <button
              onClick={() => { setShowSchedule(!showSchedule); if (!showSchedule) loadSchedule() }}
              style={{
                fontSize: 10, fontWeight: 600, padding: '5px 0',
                color: showSchedule ? 'var(--accent)' : 'var(--text-dim)',
                cursor: 'pointer', letterSpacing: '0.04em',
                background: showSchedule ? 'var(--accent-dim)' : 'transparent',
                border: '1px solid var(--border)',
                borderRadius: 'var(--radius-sm)',
                transition: 'all var(--transition)',
                fontFamily: 'var(--sans)',
              }}
              onMouseEnter={e => { if (!showSchedule) e.currentTarget.style.color = 'var(--accent)' }}
              onMouseLeave={e => { if (!showSchedule) e.currentTarget.style.color = 'var(--text-dim)' }}
            >SCHEDULE</button>
            <button
              onClick={() => setShowFacts(!showFacts)}
              style={{
                fontSize: 10, fontWeight: 600, padding: '5px 0',
                color: showFacts ? 'var(--accent)' : 'var(--text-dim)',
                cursor: 'pointer', letterSpacing: '0.04em',
                background: showFacts ? 'var(--accent-dim)' : 'transparent',
                border: '1px solid var(--border)',
                borderRadius: 'var(--radius-sm)',
                transition: 'all var(--transition)',
                fontFamily: 'var(--sans)',
              }}
              onMouseEnter={e => { if (!showFacts) e.currentTarget.style.color = 'var(--accent)' }}
              onMouseLeave={e => { if (!showFacts) e.currentTarget.style.color = 'var(--text-dim)' }}
            >FACTS</button>
            <button
              onClick={() => setShowAvailability(!showAvailability)}
              title="Slot A/B/C schedules — when inside a window, the rep auto-replies for contacts on that slot."
              style={{
                fontSize: 10, fontWeight: 600, padding: '5px 0',
                color: showAvailability ? 'var(--accent)' : 'var(--text-dim)',
                cursor: 'pointer', letterSpacing: '0.04em',
                background: showAvailability ? 'var(--accent-dim)' : 'transparent',
                border: '1px solid var(--border)',
                borderRadius: 'var(--radius-sm)',
                transition: 'all var(--transition)',
                fontFamily: 'var(--sans)',
              }}
              onMouseEnter={e => { if (!showAvailability) e.currentTarget.style.color = 'var(--accent)' }}
              onMouseLeave={e => { if (!showAvailability) e.currentTarget.style.color = 'var(--text-dim)' }}
            >AVAIL</button>
            <button
              onClick={() => setShowSleep(!showSleep)}
              title="Manual sleep mode — rep covers a chosen contact list until your wake-by time."
              style={{
                fontSize: 10, fontWeight: 600, padding: '5px 0',
                color: showSleep ? 'var(--accent)' : 'var(--text-dim)',
                cursor: 'pointer', letterSpacing: '0.04em',
                background: showSleep ? 'var(--accent-dim)' : 'transparent',
                border: '1px solid var(--border)',
                borderRadius: 'var(--radius-sm)',
                transition: 'all var(--transition)',
                fontFamily: 'var(--sans)',
              }}
              onMouseEnter={e => { if (!showSleep) e.currentTarget.style.color = 'var(--accent)' }}
              onMouseLeave={e => { if (!showSleep) e.currentTarget.style.color = 'var(--text-dim)' }}
            >SLEEP</button>
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
              onCycleSlot={() => cycleSlot(c.phone)}
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
            daemonKey={daemonKey}
            onSend={text => sendMessage(selectedPhone, text)}
            onLoadMore={() => {
              const msgs = convo?.messages || []
              if (msgs.length > 0 && convo?.hasMore) loadConversation(selectedPhone, msgs[0].id)
            }}
            onUpdateNotes={notes => updateContactNotes(selectedPhone, notes)}
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

      {/* Facts overlay */}
      {showFacts && (
        <SmsFactsPanel
          daemonKey={daemonKey}
          onClose={() => setShowFacts(false)}
        />
      )}

      {/* Availability (slot A/B/C schedules) overlay */}
      {showAvailability && (
        <SmsAvailabilityPanel
          daemonKey={daemonKey}
          onClose={() => { setShowAvailability(false); loadContacts() }}
        />
      )}

      {/* Sleep mode overlay */}
      {showSleep && (
        <SmsSleepPanel
          daemonKey={daemonKey}
          contacts={contacts}
          onClose={() => { setShowSleep(false); loadContacts() }}
        />
      )}
    </div>
  )
}
