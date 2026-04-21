import { useState, useEffect, useRef } from 'react'
import { API, STORAGE_KEY, apiFetch } from '../lib/api.js'

export default function AuthScreen({ onAuth }) {
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
