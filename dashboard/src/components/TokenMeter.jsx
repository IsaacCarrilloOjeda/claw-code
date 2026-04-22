import { useEffect, useRef, useState } from 'react'
import { API } from '../lib/api.js'

// Two modes:
//   - summary: reads { input, output, cost_cents } from the last response.
//     Compact pill. Used in SMS + Chat panels next to the send button.
//   - streaming: opens an SSE connection to /stream/tokens/:job_id and
//     accumulates tokens live. Used in CoderPanel during an in-flight turn.
//
// Cost is displayed as $0.012 (cost_cents / 100).

function fmt(n) {
  if (n == null) return '0'
  if (n < 1000) return String(n)
  if (n < 10_000) return (n / 1000).toFixed(1) + 'k'
  return Math.round(n / 1000) + 'k'
}

function money(cents) {
  if (cents == null) return '$0.00'
  return `$${(cents / 100).toFixed(Math.abs(cents) < 100 ? 3 : 2)}`
}

function Pill({ input, output, cost, compact }) {
  return (
    <span style={{
      display: 'inline-flex',
      alignItems: 'center',
      gap: 8,
      padding: compact ? '3px 8px' : '4px 10px',
      fontSize: compact ? 10 : 11,
      fontFamily: 'var(--mono)',
      color: 'var(--text-dim)',
      background: 'var(--surface-2)',
      border: '1px solid var(--border)',
      borderRadius: 12,
      whiteSpace: 'nowrap',
    }}>
      <span title="input tokens">↑ {fmt(input)}</span>
      <span title="output tokens">↓ {fmt(output)}</span>
      <span style={{ color: 'var(--text-muted)' }} title="cost">{money(cost)}</span>
    </span>
  )
}

export default function TokenMeter({ mode = 'summary', tokens, jobId, daemonKey, onStreamClose, compact = false }) {
  const [live, setLive] = useState(null)
  const esRef = useRef(null)

  useEffect(() => {
    if (mode !== 'streaming' || !jobId) return undefined

    const url = `${API}/stream/tokens/${jobId}?key=${encodeURIComponent(daemonKey || '')}`
    // EventSource can't set Authorization headers in the browser, so the
    // daemon accepts ?key= as a fallback on this endpoint. When the daemon
    // hasn't been updated to honor that, the connection 401s and we degrade
    // silently — the caller's summary pill still shows the final totals.
    const es = new EventSource(url)
    esRef.current = es

    const acc = { input: 0, output: 0, cost: 0 }
    es.onmessage = (ev) => {
      try {
        const data = JSON.parse(ev.data)
        acc.input += data.input ?? 0
        acc.output += data.output ?? 0
        acc.cost += data.cost_cents ?? 0
        setLive({ ...acc })
      } catch { /* heartbeat or malformed frame */ }
    }
    es.onerror = () => {
      es.close()
      esRef.current = null
      onStreamClose?.()
    }
    return () => {
      es.close()
      esRef.current = null
    }
  }, [mode, jobId, daemonKey, onStreamClose])

  if (mode === 'streaming') {
    const input = live?.input ?? 0
    const output = live?.output ?? 0
    const cost = live?.cost ?? 0
    return <Pill input={input} output={output} cost={cost} compact={compact} />
  }

  const input = tokens?.input ?? 0
  const output = tokens?.output ?? 0
  const cost = tokens?.cost_cents ?? 0
  return <Pill input={input} output={output} cost={cost} compact={compact} />
}

// Full budget bar used at the bottom of CoderPanel's main pane.
// Fetches /code/spend on mount and then every 10s. Turns red at >80%.
// Exports a boolean the caller uses to disable new turns at >=100%.

export function BudgetBar({ daemonKey, onOverCap }) {
  const [spend, setSpend] = useState(null)
  const [err, setErr] = useState(null)

  useEffect(() => {
    if (!daemonKey) return undefined
    let cancelled = false
    async function pull() {
      try {
        const r = await fetch(`${API}/code/spend`, {
          headers: { 'Authorization': `Bearer ${daemonKey}` },
          signal: AbortSignal.timeout(5_000),
        })
        if (!r.ok) throw new Error(String(r.status))
        const data = await r.json()
        if (!cancelled) { setSpend(data); setErr(null) }
      } catch (e) {
        if (!cancelled) setErr(e.message)
      }
    }
    pull()
    const id = setInterval(pull, 10_000)
    return () => { cancelled = true; clearInterval(id) }
  }, [daemonKey])

  useEffect(() => {
    if (!spend) return
    const ratio = spend.cap_cents > 0 ? spend.today_cents / spend.cap_cents : 0
    onOverCap?.(ratio >= 1)
  }, [spend, onOverCap])

  if (err && !spend) {
    return (
      <div style={{
        padding: '4px 12px',
        fontSize: 10,
        fontFamily: 'var(--mono)',
        color: 'var(--text-dim)',
        borderTop: '1px solid var(--border)',
        background: 'var(--surface)',
      }}>
        budget — unreachable
      </div>
    )
  }

  if (!spend) return null

  const pct = spend.cap_cents > 0 ? Math.min(1, spend.today_cents / spend.cap_cents) : 0
  const over80 = pct > 0.8
  const over100 = pct >= 1
  const barColor = over100 ? '#f43f5e' : over80 ? '#fb923c' : 'var(--accent)'

  return (
    <div style={{
      padding: '6px 12px',
      borderTop: '1px solid var(--border)',
      background: 'var(--surface)',
      flexShrink: 0,
    }}>
      <div style={{
        display: 'flex',
        justifyContent: 'space-between',
        alignItems: 'center',
        fontSize: 10,
        fontFamily: 'var(--mono)',
        color: 'var(--text-dim)',
        marginBottom: 3,
      }}>
        <span>
          BUDGET {money(spend.today_cents)} / {money(spend.cap_cents)}
        </span>
        <span style={{ color: over100 ? '#f43f5e' : over80 ? '#fb923c' : 'var(--text-dim)' }}>
          {Math.round(pct * 100)}%{over100 ? ' — over cap' : over80 ? ' — near cap' : ''}
        </span>
      </div>
      <div style={{
        height: 4,
        background: 'var(--border)',
        borderRadius: 2,
        overflow: 'hidden',
      }}>
        <div style={{
          width: `${Math.round(pct * 100)}%`,
          height: '100%',
          background: barColor,
          transition: 'width var(--transition), background var(--transition)',
        }} />
      </div>
    </div>
  )
}
