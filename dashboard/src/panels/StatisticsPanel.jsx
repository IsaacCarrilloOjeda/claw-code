export default function StatisticsPanel() {
  return (
    <div style={{ flex: 1, padding: 24, overflowY: 'auto' }}>
      <div style={{ fontSize: 14, fontWeight: 600, color: 'var(--text-bright)', marginBottom: 20 }}>
        Statistics
      </div>
      <div style={{ color: 'var(--text-dim)', fontSize: 12, fontFamily: 'var(--mono)' }}>
        Usage statistics will appear here once jobs are tracked.
      </div>
    </div>
  )
}
