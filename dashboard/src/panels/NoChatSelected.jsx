export default function NoChatSelected() {
  return (
    <div style={{
      flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center',
      flexDirection: 'column', gap: 8,
    }}>
      <div style={{ fontSize: 20, fontWeight: 700, color: 'var(--accent)', letterSpacing: '-0.02em' }}>
        GHOST
      </div>
      <div style={{ color: 'var(--text-dim)', fontSize: 12 }}>
        select or create a chat to begin
      </div>
    </div>
  )
}
