// Shared config + fetch helpers used across panels.

export const API = import.meta.env.VITE_DAEMON_URL || 'http://127.0.0.1:7878'
export const STORAGE_KEY = 'ghost-daemon-key'
export const PROJECTS_KEY = 'ghost-projects'

// IDs match backend `Intent` variants in `rust/crates/rusty-claude-cli/src/agents/intent.rs`.
// Triggers match the prefix parser in the same file.
export const AGENTS = [
  { id: 'chat',           label: 'Chat',           trigger: '(no prefix)', color: '#2dd4bf' },
  { id: 'director',       label: 'Director',       trigger: '!',           color: '#a78bfa' },
  { id: 'research',       label: 'Research',       trigger: '?',           color: '#3b82f6' },
  { id: 'calendar',       label: 'Calendar',       trigger: '@',           color: '#f59e0b' },
  { id: 'chief_of_staff', label: 'Chief of Staff', trigger: '#',           color: '#34d399' },
  { id: 'docs',           label: 'Docs',           trigger: '&',           color: '#22d3ee' },
  { id: 'dreamer',        label: 'Dreamer',        trigger: '~',           color: '#f43f5e' },
  { id: 'scheduled',      label: 'Scheduled',      trigger: '>',           color: '#fb923c' },
]

export const QUICK_REPLIES = ["On my way", "In a meeting", "Call you back", "Got it", "Running late"]

export function uid() {
  return crypto.randomUUID?.() ?? Math.random().toString(36).slice(2, 10)
}

export function fmtUptime(secs) {
  if (secs == null) return '--'
  const d = Math.floor(secs / 86400)
  const h = Math.floor((secs % 86400) / 3600)
  const m = Math.floor((secs % 3600) / 60)
  if (d > 0) return `${d}d ${h}h`
  if (h > 0) return `${h}h ${m}m`
  if (m > 0) return `${m}m`
  return `${secs}s`
}

export async function apiFetch(path, opts = {}, token = null) {
  const headers = { ...(opts.headers || {}) }
  if (token) headers['Authorization'] = `Bearer ${token}`
  const signal = opts.signal ?? AbortSignal.timeout(10_000)
  const r = await fetch(`${API}${path}`, { ...opts, headers, signal })
  if (!r.ok) throw new Error(`${r.status}`)
  return r.json()
}

export function timeAgo(isoString) {
  const secs = Math.floor((Date.now() - new Date(isoString).getTime()) / 1000)
  if (secs < 60) return 'now'
  if (secs < 3600) return `${Math.floor(secs / 60)}m`
  if (secs < 86400) return `${Math.floor(secs / 3600)}h`
  if (secs < 604800) return `${Math.floor(secs / 86400)}d`
  return new Date(isoString).toLocaleDateString()
}

export function loadProjects() {
  try {
    const raw = localStorage.getItem(PROJECTS_KEY)
    if (raw) return JSON.parse(raw)
  } catch { /* ignore */ }
  return [{ id: uid(), name: 'Default', expanded: true, chats: [{ id: uid(), name: 'General', messages: [] }] }]
}

export function saveProjects(projects) {
  localStorage.setItem(PROJECTS_KEY, JSON.stringify(projects))
}

export async function fetchEvents({ limit = 50, agent } = {}, token) {
  const qs = new URLSearchParams({ limit: String(limit) })
  if (agent) qs.set('agent', agent)
  return apiFetch(`/events?${qs.toString()}`, {}, token)
}

export async function fetchBudget(token) {
  return apiFetch('/agents/budget', {}, token)
}

export async function fetchAgents(token) {
  return apiFetch('/agents', {}, token)
}
