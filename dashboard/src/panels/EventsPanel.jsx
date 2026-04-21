import { useState, useEffect, useRef, useCallback } from 'react'
import { fetchEvents, timeAgo } from '../lib/api.js'

const OUTCOME_COLOR = {
  success:   '#34d399',
  fallback:  '#f59e0b',
  refused:   '#94a3b8',
  error:     '#f43f5e',
  escalated: '#a78bfa',
}

function OutcomePill({ outcome }) {
  const color = OUTCOME_COLOR[outcome] || 'var(--text-dim)'
  return (
    <span style={{
      display: 'inline-block',
      padding: '1px 8px',
      fontSize: 10,
      fontFamily: 'var(--mono)',
      fontWeight: 600,
      color,
      border: `1px solid ${color}`,
      borderRadius: 999,
      textTransform: 'uppercase',
      letterSpacing: '0.04em',
    }}>{outcome}</span>
  )
}

function dollars(cents) {
  if (cents == null) return '$0.00'
  return `$${(cents / 100).toFixed(2)}`
}

function truncate(s, n = 80) {
  if (!s) return ''
  return s.length > n ? s.slice(0, n) + '…' : s
}

export default function EventsPanel({ daemonKey }) {
  const [events, setEvents] = useState([])
  const [limit, setLimit] = useState(50)
  const [agentFilter, setAgentFilter] = useState('')
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState(null)
  const mountedRef = useRef(true)

  const load = useCallback(async () => {
    setLoading(true)
    setError(null)
    try {
      const data = await fetchEvents({ limit, agent: agentFilter.trim() || undefined }, daemonKey)
      if (!mountedRef.current) return
      setEvents(Array.isArray(data?.events) ? data.events : [])
    } catch (e) {
      if (!mountedRef.current) return
      setError(e.message || String(e))
    } finally {
      if (mountedRef.current) setLoading(false)
    }
  }, [limit, agentFilter, daemonKey])

  useEffect(() => {
    mountedRef.current = true
    load()
    const id = setInterval(load, 15_000)
    return () => { mountedRef.current = false; clearInterval(id) }
  }, [load])

  return (
    <div style={{ flex: 1, padding: 24, overflowY: 'auto' }}>
      <div style={{
        display: 'flex', alignItems: 'center', justifyContent: 'space-between',
        marginBottom: 16,
      }}>
        <div style={{ fontSize: 14, fontWeight: 600, color: 'var(--text-bright)' }}>
          Events
        </div>
        <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
          <input
            placeholder="filter by agent"
            value={agentFilter}
            onChange={e => setAgentFilter(e.target.value)}
            onKeyDown={e => { if (e.key === 'Enter') load() }}
            style={{
              fontFamily: 'var(--mono)', fontSize: 11,
              background: 'var(--bg)', color: 'var(--text)',
              border: '1px solid var(--border)', borderRadius: 'var(--radius-sm)',
              padding: '5px 8px', outline: 'none', width: 160,
            }}
          />
          <button
            onClick={load}
            disabled={loading}
            style={{
              padding: '5px 12px', fontSize: 11,
              background: 'transparent', color: 'var(--text-dim)',
              border: '1px solid var(--border)', borderRadius: 'var(--radius-sm)',
              cursor: loading ? 'default' : 'pointer',
            }}
          >{loading ? 'loading…' : 'refresh'}</button>
        </div>
      </div>

      {error && (
        <div style={{
          padding: 12, marginBottom: 12,
          background: 'rgba(244, 63, 94, 0.1)',
          border: '1px solid rgba(244, 63, 94, 0.4)',
          borderRadius: 'var(--radius)',
          color: '#f43f5e', fontSize: 12, fontFamily: 'var(--mono)',
        }}>error: {error}</div>
      )}

      {!error && events.length === 0 && !loading && (
        <div style={{ color: 'var(--text-dim)', fontSize: 12, fontFamily: 'var(--mono)' }}>
          No events recorded yet.
        </div>
      )}

      {events.length > 0 && (
        <div style={{
          background: 'var(--surface-2)',
          border: '1px solid var(--border)',
          borderRadius: 'var(--radius)',
          overflow: 'hidden',
        }}>
          <div style={{
            display: 'grid',
            gridTemplateColumns: '60px 120px 70px 90px 90px 80px 1fr',
            gap: 12, padding: '8px 12px',
            fontSize: 10, fontWeight: 600, letterSpacing: '0.04em',
            color: 'var(--text-dim)', textTransform: 'uppercase',
            borderBottom: '1px solid var(--border)',
            fontFamily: 'var(--mono)',
          }}>
            <div>Time</div>
            <div>Agent</div>
            <div>Tier</div>
            <div>Outcome</div>
            <div style={{ textAlign: 'right' }}>Tokens</div>
            <div style={{ textAlign: 'right' }}>Cost</div>
            <div>Input</div>
          </div>

          {events.map(ev => (
            <div
              key={ev.id}
              style={{
                display: 'grid',
                gridTemplateColumns: '60px 120px 70px 90px 90px 80px 1fr',
                gap: 12, padding: '8px 12px',
                fontSize: 11, color: 'var(--text)', fontFamily: 'var(--mono)',
                borderBottom: '1px solid var(--border)',
              }}
            >
              <div style={{ color: 'var(--text-dim)' }}>{timeAgo(ev.created_at)}</div>
              <div style={{ color: 'var(--text)' }}>{ev.agent}</div>
              <div style={{ color: 'var(--text-dim)' }}>{ev.tier}</div>
              <div><OutcomePill outcome={ev.outcome} /></div>
              <div style={{ textAlign: 'right', color: 'var(--text-dim)' }}>
                {ev.tokens_in}/{ev.tokens_out}
              </div>
              <div style={{ textAlign: 'right' }}>{dollars(ev.cost_cents)}</div>
              <div
                title={ev.input || ''}
                style={{
                  color: 'var(--text-dim)',
                  overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
                }}
              >{truncate(ev.input, 120)}</div>
            </div>
          ))}
        </div>
      )}

      {events.length > 0 && events.length >= limit && (
        <div style={{ display: 'flex', justifyContent: 'center', marginTop: 12 }}>
          <button
            onClick={() => setLimit(l => l + 50)}
            style={{
              padding: '6px 14px', fontSize: 11,
              background: 'transparent', color: 'var(--text-dim)',
              border: '1px solid var(--border)', borderRadius: 'var(--radius-sm)',
              cursor: 'pointer',
            }}
          >Load more</button>
        </div>
      )}
    </div>
  )
}
