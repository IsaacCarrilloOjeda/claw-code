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
    console.error('[dashboard] uncaught render error', error, info)
  }

  render() {
    if (this.state.error) {
      return (
        <div style={{
          fontFamily: 'IBM Plex Mono, monospace',
          background: '#0d0d0f',
          color: '#c8d0e0',
          minHeight: '100vh',
          padding: 32,
        }}>
          <h1 style={{ color: '#ff3d6b', fontSize: 16, marginBottom: 16 }}>
            dashboard crashed
          </h1>
          <pre style={{
            background: '#080a0d',
            border: '1px solid #1e2330',
            padding: 16,
            borderRadius: 4,
            fontSize: 12,
            whiteSpace: 'pre-wrap',
            wordBreak: 'break-word',
            marginBottom: 16,
          }}>
            {String(this.state.error?.stack ?? this.state.error)}
          </pre>
          <button
            onClick={() => window.location.reload()}
            style={{
              background: '#00e5ff',
              color: '#000',
              border: 'none',
              padding: '8px 18px',
              borderRadius: 3,
              fontFamily: 'inherit',
              fontSize: 12,
              fontWeight: 600,
              letterSpacing: '0.08em',
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
