// Two-slot "isolated top" pill strip: one main chat + one code chat.
// Only renders when BOTH slots are filled — solo pins are redundant with the
// regular sidebar and CoderPanel sidebar.

const KIND_STYLES = {
  main: { bg: 'rgba(59,130,246,0.12)', border: 'rgba(59,130,246,0.45)', fg: '#60a5fa', label: 'MAIN' },
  code: { bg: 'rgba(34,197,94,0.12)',  border: 'rgba(34,197,94,0.45)',  fg: '#4ade80', label: 'CODE' },
}

function Pill({ slot, entry, active, onSelect, onUnpin }) {
  const s = KIND_STYLES[slot]
  return (
    <div
      onClick={() => onSelect(slot)}
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 8,
        padding: '4px 6px 4px 10px',
        background: s.bg,
        border: `1px solid ${active ? s.fg : s.border}`,
        borderRadius: 14,
        cursor: 'pointer',
        fontFamily: 'var(--mono)',
        fontSize: 11,
        color: s.fg,
        maxWidth: 280,
        transition: 'border-color var(--transition)',
      }}
      title={`${s.label}: ${entry.title || 'Untitled'}`}
    >
      <span style={{ fontSize: 9, fontWeight: 700, letterSpacing: '0.08em' }}>{s.label}</span>
      <span style={{
        overflow: 'hidden',
        textOverflow: 'ellipsis',
        whiteSpace: 'nowrap',
        color: 'var(--text)',
        fontSize: 11,
      }}>
        {entry.title || 'Untitled'}
      </span>
      <span
        onClick={e => { e.stopPropagation(); onUnpin(slot) }}
        style={{
          fontSize: 11,
          color: 'var(--text-dim)',
          cursor: 'pointer',
          padding: '0 4px',
          lineHeight: 1,
        }}
        onMouseEnter={e => e.currentTarget.style.color = 'var(--red)'}
        onMouseLeave={e => e.currentTarget.style.color = 'var(--text-dim)'}
        title="unpin"
      >
        x
      </span>
    </div>
  )
}

export default function PinnedChats({ pinnedChats, activeSlot, onSelect, onUnpin }) {
  const { main, code } = pinnedChats || {}
  if (!main || !code) return null

  return (
    <div style={{
      display: 'flex',
      gap: 8,
      padding: '6px 12px',
      background: 'var(--surface)',
      borderBottom: '1px solid var(--border)',
      flexShrink: 0,
      alignItems: 'center',
    }}>
      <span style={{
        fontSize: 9,
        color: 'var(--text-dim)',
        letterSpacing: '0.08em',
        fontFamily: 'var(--mono)',
        marginRight: 2,
      }}>
        PINNED
      </span>
      <Pill slot="main" entry={main} active={activeSlot === 'main'} onSelect={onSelect} onUnpin={onUnpin} />
      <Pill slot="code" entry={code} active={activeSlot === 'code'} onSelect={onSelect} onUnpin={onUnpin} />
    </div>
  )
}
