import { useState, useEffect, useRef, useCallback } from 'react'
import { fetchAgents } from '../lib/api.js'

const TIER_COLOR = {
  fast: '#34d399',
  code: '#22d3ee',
  mid:  '#a78bfa',
  full: '#f59e0b',
}

function TierBadge({ tier }) {
  const color = TIER_COLOR[tier] || 'var(--text-dim)'
  return (
    <span style={{
      display: 'inline-block', padding: '1px 8px',
      fontSize: 10, fontWeight: 600, fontFamily: 'var(--mono)',
      color, border: `1px solid ${color}`, borderRadius: 999,
      textTransform: 'uppercase', letterSpacing: '0.04em',
    }}>{tier}</span>
  )
}

function StatusDot({ implemented }) {
  return (
    <span
      title={implemented ? 'implemented' : 'planned'}
      style={{
        width: 8, height: 8, borderRadius: '50%',
        background: implemented ? '#34d399' : 'var(--border)',
        flexShrink: 0,
      }}
    />
  )
}

function AgentCard({ agent }) {
  return (
    <div style={{
      padding: 16,
      background: 'var(--surface-2)',
      border: '1px solid var(--border)',
      borderRadius: 'var(--radius)',
      display: 'flex', flexDirection: 'column', gap: 10,
      opacity: agent.implemented ? 1 : 0.7,
    }}>
      <div style={{
        display: 'flex', alignItems: 'center', justifyContent: 'space-between',
      }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
          <StatusDot implemented={agent.implemented} />
          <span style={{ fontSize: 13, fontWeight: 600, color: 'var(--text)' }}>
            {agent.name}
          </span>
        </div>
        <TierBadge tier={agent.tier} />
      </div>

      <div style={{
        fontSize: 11, color: 'var(--text-dim)',
        fontFamily: 'var(--mono)',
      }}>
        <span style={{ color: 'var(--text-dim)' }}>trigger: </span>
        <span style={{ color: 'var(--text)' }}>{agent.trigger}</span>
      </div>

      <div style={{
        fontSize: 10, fontFamily: 'var(--mono)',
        color: agent.implemented ? '#34d399' : 'var(--text-dim)',
        textTransform: 'uppercase', letterSpacing: '0.04em', fontWeight: 600,
      }}>
        {agent.implemented ? 'implemented' : 'planned'}
      </div>
    </div>
  )
}

export default function AgentsPanel({ daemonKey }) {
  const [agents, setAgents] = useState([])
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState(null)
  const mountedRef = useRef(true)

  const load = useCallback(async () => {
    setLoading(true)
    setError(null)
    try {
      const data = await fetchAgents(daemonKey)
      if (!mountedRef.current) return
      setAgents(Array.isArray(data?.agents) ? data.agents : [])
    } catch (e) {
      if (!mountedRef.current) return
      setError(e.message || String(e))
    } finally {
      if (mountedRef.current) setLoading(false)
    }
  }, [daemonKey])

  useEffect(() => {
    mountedRef.current = true
    load()
    return () => { mountedRef.current = false }
  }, [load])

  return (
    <div style={{ flex: 1, padding: 24, overflowY: 'auto' }}>
      <div style={{
        display: 'flex', alignItems: 'center', justifyContent: 'space-between',
        marginBottom: 16,
      }}>
        <div style={{ fontSize: 14, fontWeight: 600, color: 'var(--text-bright)' }}>
          Agents
        </div>
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

      {error && (
        <div style={{
          padding: 12, marginBottom: 12,
          background: 'rgba(244, 63, 94, 0.1)',
          border: '1px solid rgba(244, 63, 94, 0.4)',
          borderRadius: 'var(--radius)',
          color: '#f43f5e', fontSize: 12, fontFamily: 'var(--mono)',
        }}>error: {error}</div>
      )}

      {!error && agents.length === 0 && !loading && (
        <div style={{ color: 'var(--text-dim)', fontSize: 12, fontFamily: 'var(--mono)' }}>
          No agents registered.
        </div>
      )}

      {agents.length > 0 && (
        <div style={{
          display: 'grid',
          gridTemplateColumns: 'repeat(auto-fill, minmax(220px, 1fr))',
          gap: 12,
        }}>
          {agents.map(a => <AgentCard key={a.name} agent={a} />)}
        </div>
      )}
    </div>
  )
}
