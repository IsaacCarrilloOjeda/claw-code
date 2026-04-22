import { useState } from 'react'

// Unified-diff renderer. `search` lines render red-strikethrough, `replace`
// lines green. Naive line-by-line — good enough for v1; a real hunker can
// come later if long diffs get noisy.
function UnifiedDiff({ search, replace }) {
  const searchLines = (search ?? '').split('\n')
  const replaceLines = (replace ?? '').split('\n')
  return (
    <pre style={{
      margin: 0,
      padding: '8px 10px',
      fontFamily: 'var(--mono)',
      fontSize: 11,
      lineHeight: 1.5,
      background: 'var(--bg)',
      borderRadius: 'var(--radius-sm)',
      overflow: 'auto',
      maxHeight: 280,
      whiteSpace: 'pre',
    }}>
      {searchLines.map((l, i) => (
        <div key={`s${i}`} style={{
          color: '#f87171',
          background: 'rgba(248,113,113,0.06)',
          textDecoration: 'line-through',
          textDecorationColor: 'rgba(248,113,113,0.5)',
        }}>
          {'- '}{l || ' '}
        </div>
      ))}
      {replaceLines.map((l, i) => (
        <div key={`r${i}`} style={{
          color: '#4ade80',
          background: 'rgba(74,222,128,0.06)',
        }}>
          {'+ '}{l || ' '}
        </div>
      ))}
    </pre>
  )
}

function DiffCard({ diff, autoApply, onApply, onReject, busy }) {
  const [expanded, setExpanded] = useState(true)
  return (
    <div style={{
      border: '1px solid var(--border)',
      borderRadius: 'var(--radius)',
      background: 'var(--surface-2)',
      marginBottom: 10,
      overflow: 'hidden',
    }}>
      <header
        onClick={() => setExpanded(e => !e)}
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 6,
          padding: '6px 10px',
          cursor: 'pointer',
          fontSize: 11,
          fontFamily: 'var(--mono)',
          color: 'var(--text)',
          borderBottom: expanded ? '1px solid var(--border)' : 'none',
        }}
      >
        <span style={{ fontSize: 8, color: 'var(--text-dim)' }}>{expanded ? '▼' : '▶'}</span>
        <span style={{ flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
          {diff.path}
        </span>
      </header>
      {expanded && (
        <>
          <UnifiedDiff search={diff.search} replace={diff.replace} />
          <div style={{
            display: 'flex',
            gap: 6,
            padding: '8px 10px',
            borderTop: '1px solid var(--border)',
            alignItems: 'center',
          }}>
            <button
              onClick={() => onApply(diff.id)}
              disabled={autoApply || busy}
              style={{
                padding: '5px 12px',
                fontSize: 11,
                fontFamily: 'var(--mono)',
                fontWeight: 600,
                background: (autoApply || busy) ? 'var(--border)' : '#4ade80',
                color: (autoApply || busy) ? 'var(--text-dim)' : '#0b1f14',
                border: 'none',
                borderRadius: 'var(--radius-sm)',
                cursor: (autoApply || busy) ? 'default' : 'pointer',
              }}
            >
              APPLY
            </button>
            <button
              onClick={() => onReject(diff.id)}
              disabled={autoApply || busy}
              style={{
                padding: '5px 12px',
                fontSize: 11,
                fontFamily: 'var(--mono)',
                background: 'transparent',
                color: autoApply ? 'var(--text-dim)' : '#f87171',
                border: `1px solid ${autoApply ? 'var(--border)' : '#f8717155'}`,
                borderRadius: 'var(--radius-sm)',
                cursor: (autoApply || busy) ? 'default' : 'pointer',
              }}
            >
              REJECT
            </button>
            {autoApply && (
              <span style={{
                marginLeft: 'auto',
                fontSize: 10,
                fontFamily: 'var(--mono)',
                color: '#4ade80',
                background: 'rgba(74,222,128,0.1)',
                border: '1px solid rgba(74,222,128,0.3)',
                padding: '3px 8px',
                borderRadius: 10,
              }}>
                auto-applied
              </span>
            )}
          </div>
        </>
      )}
    </div>
  )
}

export default function DiffReview({ diffs, autoApply, onApply, onReject, collapsed, onToggleCollapsed }) {
  const count = diffs.length

  if (collapsed) {
    return (
      <div style={{
        width: 36,
        flexShrink: 0,
        borderLeft: '1px solid var(--border)',
        background: 'var(--surface)',
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        paddingTop: 10,
      }}>
        <div
          onClick={onToggleCollapsed}
          title={`${count} pending diff${count === 1 ? '' : 's'}`}
          style={{
            fontSize: 10,
            color: count > 0 ? 'var(--accent)' : 'var(--text-dim)',
            cursor: 'pointer',
            writingMode: 'vertical-rl',
            transform: 'rotate(180deg)',
            letterSpacing: '0.1em',
            fontFamily: 'var(--mono)',
            padding: '6px 0',
          }}
        >
          {count > 0 ? `◀ ${count} DIFFS` : '◀ DIFFS'}
        </div>
      </div>
    )
  }

  return (
    <div style={{
      width: 360,
      flexShrink: 0,
      borderLeft: '1px solid var(--border)',
      background: 'var(--surface)',
      display: 'flex',
      flexDirection: 'column',
      overflow: 'hidden',
    }}>
      <header style={{
        display: 'flex',
        alignItems: 'center',
        padding: '8px 12px',
        borderBottom: '1px solid var(--border)',
        flexShrink: 0,
      }}>
        <span style={{
          fontSize: 10,
          fontWeight: 600,
          letterSpacing: '0.08em',
          color: 'var(--text)',
        }}>
          PENDING DIFFS
        </span>
        <span style={{
          marginLeft: 8,
          fontSize: 10,
          color: 'var(--text-dim)',
          fontFamily: 'var(--mono)',
        }}>
          {count}
        </span>
        <span
          onClick={onToggleCollapsed}
          style={{
            marginLeft: 'auto',
            fontSize: 12,
            color: 'var(--text-dim)',
            cursor: 'pointer',
            padding: '0 4px',
          }}
          title="collapse"
        >
          {'▶'}
        </span>
      </header>
      <div style={{ flex: 1, overflowY: 'auto', padding: 10 }}>
        {count === 0 && (
          <div style={{ color: 'var(--text-dim)', fontSize: 11, fontFamily: 'var(--mono)', textAlign: 'center', padding: 20 }}>
            no pending diffs
          </div>
        )}
        {diffs.map(d => (
          <DiffCard key={d.id} diff={d} autoApply={autoApply} onApply={onApply} onReject={onReject} />
        ))}
      </div>
    </div>
  )
}
