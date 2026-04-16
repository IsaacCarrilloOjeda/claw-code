import { StrictMode, Component } from 'react'
import { createRoot } from 'react-dom/client'
import './index.css'
import App from './App.jsx'

class ErrorBoundary extends Component {
  constructor(props) {
    super(props)
    this.state = { error: null }
  }

  static getDerivedStateFromError(error) {
    return { error }
  }

  componentDidCatch(error, info) {
    console.error('[ghost-dashboard] uncaught render error', error, info)
  }

  render() {
    if (this.state.error) {
      return (
        <div style={{
          fontFamily: "'IBM Plex Mono', monospace",
          background: '#08090b',
          color: '#c9d1d9',
          minHeight: '100vh',
          display: 'flex',
          flexDirection: 'column',
          alignItems: 'center',
          justifyContent: 'center',
          padding: 40,
          gap: 20,
        }}>
          <div style={{ color: '#f43f5e', fontSize: 14, fontWeight: 600, letterSpacing: '0.05em' }}>
            GHOST DASHBOARD CRASHED
          </div>
          <pre style={{
            background: '#0c0e12',
            border: '1px solid #1a1f2e',
            padding: 20,
            borderRadius: 6,
            fontSize: 12,
            whiteSpace: 'pre-wrap',
            wordBreak: 'break-word',
            maxWidth: 600,
            width: '100%',
            color: '#c9d1d9',
          }}>
            {String(this.state.error?.stack ?? this.state.error)}
          </pre>
          <button
            onClick={() => window.location.reload()}
            style={{
              background: '#2dd4bf',
              color: '#08090b',
              border: 'none',
              padding: '8px 24px',
              borderRadius: 4,
              fontSize: 12,
              fontWeight: 600,
              letterSpacing: '0.06em',
              textTransform: 'uppercase',
              cursor: 'pointer',
            }}
          >
            reload
          </button>
        </div>
      )
    }
    return this.props.children
  }
}

createRoot(document.getElementById('root')).render(
  <StrictMode>
    <ErrorBoundary>
      <App />
    </ErrorBoundary>
  </StrictMode>,
)
