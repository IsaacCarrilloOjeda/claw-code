import { AGENTS } from '../lib/api.js'

export default function AboutPanel() {
  return (
    <div style={{ flex: 1, padding: 24, overflowY: 'auto' }}>
      <div style={{ fontSize: 14, fontWeight: 600, color: 'var(--text-bright)', marginBottom: 20 }}>
        About GHOST
      </div>

      <div style={{ display: 'flex', flexDirection: 'column', gap: 12, fontSize: 12, color: 'var(--text)' }}>
        <p style={{ lineHeight: 1.6 }}>
          GHOST is a personal AI operating system. Routes requests through a Director AI to specialist agents for code, email, calendar, research, and more.
        </p>

        <div style={{
          background: 'var(--surface-2)',
          border: '1px solid var(--border)',
          borderRadius: 'var(--radius)',
          padding: 14,
        }}>
          <div style={{ fontSize: 10, fontWeight: 600, color: 'var(--text-dim)', letterSpacing: '0.06em', textTransform: 'uppercase', marginBottom: 8 }}>
            Agents
          </div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 4, fontFamily: 'var(--mono)', fontSize: 11 }}>
            {AGENTS.map(a => (
              <div key={a.id} style={{ display: 'flex', gap: 8 }}>
                <span style={{ color: a.color, width: 70 }}>{a.label}</span>
                <span style={{ color: 'var(--text-muted)' }}>
                  {a.id === 'echo' && 'General chat, default agent'}
                  {a.id === 'research' && 'Brave Search, web synthesis'}
                  {a.id === 'email' && 'Gmail read, draft, send'}
                  {a.id === 'calendar' && 'Google Calendar CRUD'}
                  {a.id === 'code' && 'DeepSeek + E2B sandbox'}
                  {a.id === 'itguide' && 'Step-by-step navigation'}
                  {a.id === 'law' && 'US legal research + citations'}
                </span>
              </div>
            ))}
          </div>
        </div>

        <div style={{ fontSize: 10, color: 'var(--text-dim)', lineHeight: 1.6, marginTop: 8 }}>
          AI disclaimer: GHOST provides AI-generated responses. Verify critical information independently.
          This system is built and operated by Isaac Carrillo / KYNE Systems.
        </div>
      </div>
    </div>
  )
}
