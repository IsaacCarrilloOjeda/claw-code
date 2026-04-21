import { useState, useEffect, useRef, useCallback } from 'react'
import { fetchBudget } from '../lib/api.js'

function dollars(cents) {
  return `$${((cents ?? 0) / 100).toFixed(2)}`
}

function barColor(pct, calls) {
  if (!calls || calls === 0) return 'var(--border)'
  if (pct >= 0.9) return '#f43f5e'
  if (pct >= 0.5) return '#f59e0b'
  return '#34d399'
}

function BudgetRow({ row }) {
  const pct = row.cap_cents > 0
    ? Math.max(0, Math.min(1, row.spent_cents / row.cap_cents))
    : 0
  const color = barColor(pct, row.calls_today)

  return (
    <div style={{
      padding: '12px 16px',
      background: 'var(--surface-2)',
      border: '1px solid var(--border)',
      borderRadius: 'var(--radius)',
      display: 'flex', flexDirection: 'column', gap: 8,
    }}>
      <div style={{
        display: 'flex', alignItems: 'center', justifyContent: 'space-between',
        gap: 12, fontSize: 12,
      }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
          <span style={{ color: 'var(--text)', fontWeight: 500 }}>{row.agent}</span>
          {row.is_blown && (
            <span style={{
              fontSize: 9, fontFamily: 'var(--mono)', fontWeight: 600,
              color: '#f43f5e', border: '1px solid #f43f5e',
              padding: '1px 6px', borderRadius: 999,
              textTransform: 'uppercase', letterSpacing: '0.04em',
            }}>blown</span>
          )}
        </div>
        <div style={{
          color: 'var(--text-dim)', fontSize: 11, fontFamily: 'var(--mono)',
          display: 'flex', gap: 12,
        }}>
          <span>{row.calls_today} calls</span>
          <span style={{ color: 'var(--text)' }}>
            {dollars(row.spent_cents)} / {dollars(row.cap_cents)}
          </span>
        </div>
      </div>

      <div style={{
        height: 6, background: 'var(--bg)',
        borderRadius: 3, overflow: 'hidden',
      }}>
        <div style={{
          width: `${pct * 100}%`, height: '100%',
          background: color,
          transition: 'width var(--transition), background var(--transition)',
        }} />
      </div>
    </div>
  )
}

export default function BudgetPanel({ daemonKey }) {
  const [today, setToday] = useState([])
  const [date, setDate] = useState(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState(null)
  const mountedRef = useRef(true)

  const load = useCallback(async () => {
    setLoading(true)
    setError(null)
    try {
      const data = await fetchBudget(daemonKey)
      if (!mountedRef.current) return
      setToday(Array.isArray(data?.today) ? data.today : [])
      setDate(data?.date || null)
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
    const id = setInterval(load, 15_000)
    return () => { mountedRef.current = false; clearInterval(id) }
  }, [load])

  return (
    <div style={{ flex: 1, padding: 24, overflowY: 'auto' }}>
      <div style={{
        display: 'flex', alignItems: 'center', justifyContent: 'space-between',
        marginBottom: 16,
      }}>
        <div>
          <div style={{ fontSize: 14, fontWeight: 600, color: 'var(--text-bright)' }}>
            Budget
          </div>
          <div style={{ fontSize: 11, color: 'var(--text-dim)', fontFamily: 'var(--mono)', marginTop: 2 }}>
            {date ? `today — ${date}` : 'today'}
          </div>
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

      {!error && today.length === 0 && !loading && (
        <div style={{ color: 'var(--text-dim)', fontSize: 12, fontFamily: 'var(--mono)' }}>
          No spend recorded today.
        </div>
      )}

      {today.length > 0 && (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
          {today.map(row => <BudgetRow key={row.agent} row={row} />)}
        </div>
      )}
    </div>
  )
}
