import { useState, useEffect, useRef, useCallback, useMemo } from 'react'
import { STORAGE_KEY, AGENTS, apiFetch, loadProjects, saveProjects } from './lib/api.js'
import AuthScreen from './panels/AuthScreen.jsx'
import TopBar from './panels/TopBar.jsx'
import JobBanner from './panels/JobBanner.jsx'
import Sidebar from './panels/Sidebar.jsx'
import ChatArea from './panels/ChatArea.jsx'
import SmsPanel from './panels/SmsPanel.jsx'
import SettingsPanel from './panels/SettingsPanel.jsx'
import StatisticsPanel from './panels/StatisticsPanel.jsx'
import AboutPanel from './panels/AboutPanel.jsx'
import EventsPanel from './panels/EventsPanel.jsx'
import BudgetPanel from './panels/BudgetPanel.jsx'
import AgentsPanel from './panels/AgentsPanel.jsx'
import NoChatSelected from './panels/NoChatSelected.jsx'
import CoderPanel from './panels/CoderPanel.jsx'
import PinnedChats from './components/PinnedChats.jsx'

const PINNED_KEY = 'ghost-pinned-chats'
function loadPinned() {
  try {
    const raw = localStorage.getItem(PINNED_KEY)
    if (raw) return JSON.parse(raw)
  } catch { /* ignore */ }
  return { main: null, code: null }
}

export default function App() {
  // Auth
  const [daemonKey, setDaemonKey] = useState(() => {
    try { return localStorage.getItem(STORAGE_KEY) || '' } catch { return '' }
  })
  const [authed, setAuthed] = useState(() => !!daemonKey)

  // Daemon state
  const [alive, setAlive] = useState(false)
  const [status, setStatus] = useState(null)

  // Projects + chats (localStorage)
  const [projects, setProjects] = useState(loadProjects)
  const [activeChatId, setActiveChatId] = useState(null)
  const [openTabs, setOpenTabs] = useState([]) // [{id, name, projectId}]

  // Sidebar
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false)
  const [bottomRatio, setBottomRatio] = useState(0.25)
  const [activeNav, setActiveNav] = useState(null) // 'code' | 'sms' | 'events' | 'budget' | 'agents' | 'settings' | 'statistics' | 'about' | null

  // Pinned-chat bar (two slots: one main, one code). Bar only renders when both are filled.
  const [pinnedChats, setPinnedChats] = useState(loadPinned)
  const [activePinSlot, setActivePinSlot] = useState(null) // which pinned pill is currently showing
  const coderFirstEntrySeen = useRef(false)

  // Chat state
  const [selectedAgents, setSelectedAgents] = useState(['chat'])
  const [agentsCollapsed, setAgentsCollapsed] = useState(false)
  const [running, setRunning] = useState(false)

  // Job banner
  const [activeJob, setActiveJob] = useState(null)

  const mountedRef = useRef(true)
  const promptAbortRef = useRef(null)

  useEffect(() => { saveProjects(projects) }, [projects])

  useEffect(() => {
    try { localStorage.setItem(PINNED_KEY, JSON.stringify(pinnedChats)) } catch { /* ignore */ }
  }, [pinnedChats])

  useEffect(() => {
    mountedRef.current = true
    return () => {
      mountedRef.current = false
      if (promptAbortRef.current) promptAbortRef.current.abort()
    }
  }, [])

  const poll = useCallback(async () => {
    if (!daemonKey) return
    try {
      const s = await apiFetch('/status', { signal: AbortSignal.timeout(5_000) }, daemonKey)
      if (!mountedRef.current) return
      setStatus(s)
      setAlive(true)
    } catch {
      if (!mountedRef.current) return
      setAlive(false)
      setStatus(null)
    }
  }, [daemonKey])

  useEffect(() => {
    if (!authed) return
    poll()
    const id = setInterval(poll, 10_000)
    return () => clearInterval(id)
  }, [authed, poll])

  const activeChat = useMemo(() => {
    for (const p of projects) {
      const c = p.chats.find(ch => ch.id === activeChatId)
      if (c) return c
    }
    return null
  }, [projects, activeChatId])

  function handleOpenChat(projectId, chatId) {
    setActiveChatId(chatId)
    setActiveNav(null)
    const project = projects.find(p => p.id === projectId)
    const chat = project?.chats.find(c => c.id === chatId)
    if (!chat) return
    setOpenTabs(prev => {
      if (prev.some(t => t.id === chatId)) return prev
      return [...prev, { id: chatId, name: chat.name, projectId }]
    })
    // Pin this as the current "main" chat. Overwrites whatever was there —
    // spec: clicking a main-chat entry sets pinnedChats.main = {...}.
    setPinnedChats(prev => ({ ...prev, main: { id: chatId, kind: 'main', title: chat.name, projectId } }))
    setActivePinSlot('main')
  }

  function handleSelectPinned(slot) {
    const entry = pinnedChats?.[slot]
    if (!entry) return
    setActivePinSlot(slot)
    if (slot === 'main') {
      setActiveChatId(entry.id)
      setActiveNav(null)
    } else if (slot === 'code') {
      // Code panel owns its own activeChatId; surfacing the panel is enough.
      setActiveNav('code')
    }
  }

  function handleUnpin(slot) {
    setPinnedChats(prev => ({ ...prev, [slot]: null }))
    if (activePinSlot === slot) setActivePinSlot(null)
  }

  function handleSelectTab(tabId) {
    setActiveChatId(tabId)
    setActiveNav(null)
  }

  function handleCloseTab(tabId) {
    setOpenTabs(prev => prev.filter(t => t.id !== tabId))
    if (activeChatId === tabId) {
      setOpenTabs(prev => {
        const remaining = prev.filter(t => t.id !== tabId)
        if (remaining.length > 0) setActiveChatId(remaining[remaining.length - 1].id)
        else setActiveChatId(null)
        return remaining
      })
    }
  }

  async function handleSendMessage(text) {
    if (!text || running) return
    if (promptAbortRef.current) promptAbortRef.current.abort()
    const controller = new AbortController()
    promptAbortRef.current = controller

    setProjects(prev => prev.map(p => ({
      ...p,
      chats: p.chats.map(c => {
        if (c.id !== activeChatId) return c
        return { ...c, messages: [...c.messages, { role: 'user', content: text }] }
      }),
    })))

    setRunning(true)
    const agentLabel = selectedAgents[0] || 'chat'

    const startTime = Date.now()
    setActiveJob({ agent: AGENTS.find(a => a.id === agentLabel)?.label ?? 'Chat', status: 'running', elapsed: 0 })
    const jobInterval = setInterval(() => {
      setActiveJob(prev => prev ? { ...prev, elapsed: Math.floor((Date.now() - startTime) / 1000) } : null)
    }, 1000)

    try {
      let currentMessages = []
      for (const p of projects) {
        const c = p.chats.find(ch => ch.id === activeChatId)
        if (c) { currentMessages = c.messages; break }
      }
      const history = currentMessages.slice(-10).map(m => ({ role: m.role, content: m.content }))

      const data = await apiFetch('/chat', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ message: text, history }),
        signal: controller.signal,
      }, daemonKey)

      if (!mountedRef.current) return

      setProjects(prev => prev.map(p => ({
        ...p,
        chats: p.chats.map(c => {
          if (c.id !== activeChatId) return c
          return {
            ...c,
            messages: [...c.messages, {
              role: 'assistant',
              content: data.response,
              job_id: data.job_id,
              agent: data.agent ?? agentLabel,
              tokens: data.tokens,
            }],
          }
        }),
      })))

      setActiveJob(prev => prev ? { ...prev, status: 'done' } : null)
      setTimeout(() => setActiveJob(null), 2000)
    } catch (e) {
      if (e.name === 'AbortError' || !mountedRef.current) return
      setProjects(prev => prev.map(p => ({
        ...p,
        chats: p.chats.map(c => {
          if (c.id !== activeChatId) return c
          return { ...c, messages: [...c.messages, { role: 'error', content: e.message }] }
        }),
      })))
      setActiveJob(null)
    } finally {
      clearInterval(jobInterval)
      if (mountedRef.current) setRunning(false)
      if (promptAbortRef.current === controller) promptAbortRef.current = null
    }
  }

  function handleClearMessages() {
    setProjects(prev => prev.map(p => ({
      ...p,
      chats: p.chats.map(c => {
        if (c.id !== activeChatId) return c
        return { ...c, messages: [] }
      }),
    })))
  }

  function handleNav(name) {
    setActiveNav(prev => prev === name ? null : name)
  }

  if (!authed) {
    return <AuthScreen onAuth={key => { setDaemonKey(key); setAuthed(true) }} />
  }

  return (
    <div style={{
      display: 'flex', flexDirection: 'column',
      height: '100vh', overflow: 'hidden', background: 'var(--bg)',
    }}>
      <style>{`
        @keyframes pulse { 0%, 100% { opacity: 1; } 50% { opacity: 0.35; } }
        @keyframes blink { 0%, 100% { opacity: 1; } 50% { opacity: 0; } }
      `}</style>

      <TopBar
        alive={alive}
        status={status}
        openTabs={openTabs}
        activeTabId={activeChatId}
        onSelectTab={handleSelectTab}
        onCloseTab={handleCloseTab}
      />

      {activeJob && <JobBanner job={activeJob} onDismiss={() => setActiveJob(null)} />}

      <PinnedChats
        pinnedChats={pinnedChats}
        activeSlot={activePinSlot}
        onSelect={handleSelectPinned}
        onUnpin={handleUnpin}
      />

      <div style={{ display: 'flex', flex: 1, overflow: 'hidden', minHeight: 0 }}>
        <Sidebar
          collapsed={sidebarCollapsed}
          onToggle={() => setSidebarCollapsed(!sidebarCollapsed)}
          projects={projects}
          setProjects={setProjects}
          onOpenChat={handleOpenChat}
          activeChatId={activeChatId}
          activeNav={activeNav}
          onNav={handleNav}
          bottomRatio={bottomRatio}
          onBottomResize={setBottomRatio}
        />

        <div style={{
          flex: 1, display: 'flex', flexDirection: 'column',
          overflow: 'hidden', minWidth: 0, background: 'var(--bg)',
        }}>
          {activeNav === 'code' && (
            <CoderPanel
              daemonKey={daemonKey}
              alive={alive}
              pinnedChats={pinnedChats}
              setPinnedChats={setPinnedChats}
              setActivePinSlot={setActivePinSlot}
              onFirstEntry={() => {
                if (!coderFirstEntrySeen.current) {
                  coderFirstEntrySeen.current = true
                  setSidebarCollapsed(true)
                }
              }}
            />
          )}
          {activeNav === 'sms' && <SmsPanel daemonKey={daemonKey} />}
          {activeNav === 'events' && <EventsPanel daemonKey={daemonKey} />}
          {activeNav === 'budget' && <BudgetPanel daemonKey={daemonKey} />}
          {activeNav === 'agents' && <AgentsPanel daemonKey={daemonKey} />}
          {activeNav === 'settings' && <SettingsPanel />}
          {activeNav === 'statistics' && <StatisticsPanel />}
          {activeNav === 'about' && <AboutPanel />}
          {!activeNav && activeChat && (
            <ChatArea
              chat={activeChat}
              alive={alive}
              selectedAgents={selectedAgents}
              setSelectedAgents={setSelectedAgents}
              running={running}
              onSendMessage={handleSendMessage}
              onClearMessages={handleClearMessages}
              agentsCollapsed={agentsCollapsed}
              onAgentsCollapseToggle={() => setAgentsCollapsed(!agentsCollapsed)}
            />
          )}
          {!activeNav && !activeChat && <NoChatSelected />}
        </div>
      </div>
    </div>
  )
}
