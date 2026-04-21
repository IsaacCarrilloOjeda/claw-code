export default function JobBanner({ job, onDismiss }) {
  if (!job) return null
  return (
    <div style={{
      height: 'var(--banner-h)',
      background: 'var(--accent-dim)',
      borderBottom: '1px solid var(--border)',
      display: 'flex',
      alignItems: 'center',
      padding: '0 16px',
      fontSize: 11,
      fontFamily: 'var(--mono)',
      gap: 12,
      flexShrink: 0,
    }}>
      <span style={{
        width: 6, height: 6,
        borderRadius: '50%',
        background: 'var(--accent)',
        animation: 'pulse 1.5s ease-in-out infinite',
      }} />
      <span style={{ color: 'var(--accent)', fontWeight: 600 }}>{job.agent}</span>
      <span style={{ color: 'var(--text-muted)' }}>{job.status}</span>
      <span style={{ color: 'var(--text-muted)' }}>{job.elapsed != null ? `${job.elapsed}s` : ''}</span>
      <span style={{ flex: 1 }} />
      <span
        onClick={onDismiss}
        style={{ color: 'var(--text-dim)', cursor: 'pointer', fontSize: 14, lineHeight: 1 }}
        onMouseEnter={e => e.target.style.color = 'var(--text)'}
        onMouseLeave={e => e.target.style.color = 'var(--text-dim)'}
      >
        x
      </span>
    </div>
  )
}
