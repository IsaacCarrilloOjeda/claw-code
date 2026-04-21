import { useState } from 'react'

function SettingRow({ label, description, defaultOn }) {
  const [on, setOn] = useState(defaultOn)
  return (
    <div style={{
      display: 'flex',
      alignItems: 'center',
      justifyContent: 'space-between',
      padding: '12px 16px',
      background: 'var(--surface-2)',
      border: '1px solid var(--border)',
      borderRadius: 'var(--radius)',
    }}>
      <div>
        <div style={{ fontSize: 12, fontWeight: 500, color: 'var(--text)' }}>{label}</div>
        <div style={{ fontSize: 11, color: 'var(--text-dim)', marginTop: 2 }}>{description}</div>
      </div>
      <div
        onClick={() => setOn(!on)}
        style={{
          width: 36, height: 20,
          background: on ? 'var(--accent)' : 'var(--border)',
          borderRadius: 10,
          cursor: 'pointer',
          position: 'relative',
          transition: 'background var(--transition)',
          flexShrink: 0,
          marginLeft: 16,
        }}
      >
        <div style={{
          width: 16, height: 16,
          borderRadius: '50%',
          background: '#fff',
          position: 'absolute',
          top: 2,
          left: on ? 18 : 2,
          transition: 'left var(--transition)',
        }} />
      </div>
    </div>
  )
}

export default function SettingsPanel() {
  return (
    <div style={{ flex: 1, padding: 24, overflowY: 'auto' }}>
      <div style={{ fontSize: 14, fontWeight: 600, color: 'var(--text-bright)', marginBottom: 20 }}>
        Settings
      </div>

      <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
        <SettingRow label="Job status banner" description="Show in-flight job status below the top bar" defaultOn={true} />
        <SettingRow label="Auto-switch tabs" description="Switch to agent preview tab on response" defaultOn={false} />
        <SettingRow label="Agent thinking (Echo)" description="Stream narrated reasoning for Echo agent" defaultOn={false} />
        <SettingRow label="Agent thinking (Research)" description="Stream narrated reasoning for Research agent" defaultOn={true} />
        <SettingRow label="Agent thinking (Code)" description="Stream narrated reasoning for Code agent" defaultOn={true} />
      </div>
    </div>
  )
}
