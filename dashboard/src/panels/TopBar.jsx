import { fmtUptime } from '../lib/api.js'

export default function TopBar({ alive, status, openTabs, activeTabId, onSelectTab, onCloseTab }) {
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
