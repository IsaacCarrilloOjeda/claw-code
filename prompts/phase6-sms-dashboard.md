# Phase 6: Dashboard SMS Tab -- Conversation History, Contact Management, Schedule UI

## Goal

Add a new "SMS" section to the GHOST dashboard that lets Isaac:

1. **View all SMS conversation histories** organized by contact, with lazy-loading
2. **Send outbound SMS** from the dashboard to any contact
3. **Toggle auto-reply on/off** per contact
4. **Manage his schedule/commitments** (daily + persistent entries) that GHOST uses to tell people his availability

---

## Prerequisite

Phase 5 (backend endpoints) must be completed. This phase consumes the following API endpoints:

| Method | Path | Purpose |
|--------|------|---------|
| GET | `/sms/contacts` | List contacts with auto-reply status |
| GET | `/sms/history/{phone}?limit=30&before={id}` | Paginated message history |
| POST | `/sms/contacts/{phone}/auto-reply` | Toggle auto-reply `{"enabled": bool}` |
| PUT | `/sms/contacts/{phone}/name` | Set display name `{"name": "..."}` |
| POST | `/sms/send` | Send SMS `{"to":"+1...","body":"..."}` (bearer auth) |
| GET | `/schedule` | List schedule entries |
| POST | `/schedule` | Add entry `{"kind":"daily/persistent","day_date":"...","content":"..."}` |
| DELETE | `/schedule/{id}` | Delete entry |

All protected endpoints require `Authorization: Bearer <key>` (the `daemonKey` already stored in the dashboard state).

---

## Context: Dashboard architecture

### Single-file component
Everything lives in `dashboard/src/App.jsx`. The dashboard is a single-file React app -- do NOT split it into separate files. Add new components to the same file, following the existing section comment pattern:

```jsx
// --- Section Name ---------------------------------------------------
function MyComponent() { ... }
```

### Existing layout structure
```
<App>
  <TopBar />                    -- health dot, GHOST, uptime, chat tabs
  <JobBanner />                 -- optional in-flight job status
  <div (flex row)>
    <Sidebar>                   -- left panel
      <ProjectTree />           -- top: scrollable project/chat tree
      <divider (draggable) />
      <SidebarNav />            -- bottom: Settings, Statistics, About
    </Sidebar>
    <main area>                 -- right panel
      {activeNav === 'settings' && <SettingsPanel />}
      {activeNav === 'statistics' && <StatisticsPanel />}
      {activeNav === 'about' && <AboutPanel />}
      {!activeNav && activeChat && <ChatArea />}
      {!activeNav && !activeChat && <NoChatSelected />}
    </main area>
  </div>
</App>
```

### SidebarNav (line ~631)
Currently has items: `['Settings', 'Statistics', 'About']`. These render as clickable rows in the bottom section of the sidebar. Clicking one sets `activeNav` to the lowercase name, which shows the corresponding panel in the main area.

### Styling conventions
- All styling is **inline `style={{}}`** -- no CSS modules, no className (except `ghost-md` for markdown rendering)
- CSS variables from `index.css`: `--bg`, `--surface`, `--surface-2`, `--border`, `--accent`, `--accent-dim`, `--text`, `--text-bright`, `--text-muted`, `--text-dim`, `--mono`, `--sans`, `--radius`, `--radius-sm`, `--transition`
- Color scheme is dark teal-accented (`--accent: #2dd4bf`)
- Font sizes: 10-12px for UI chrome, 12-13px for content
- The `apiFetch(path, opts, token)` helper handles auth headers + error throwing

### State management
- All state lives in `App()` via `useState` hooks
- `activeNav` controls which panel shows in the main area
- `daemonKey` is the bearer token from localStorage

---

## What to build

### 1. Add "SMS" to SidebarNav

In `SidebarNav`, add `'SMS'` as the **first item** in the items array (above Settings):

```jsx
const items = ['SMS', 'Settings', 'Statistics', 'About']
```

When clicked, `activeNav` becomes `'sms'`, and the main area renders an `<SmsPanel />` component.

In the main `App` render, add before the settings check:

```jsx
{activeNav === 'sms' && <SmsPanel daemonKey={daemonKey} />}
```

### 2. SmsPanel component

This is the main container. It has a **two-column layout**:

```
+--------------------------------------+
| [Search bar]  [+ Add]   [Schedule]   |  <- header row
+-------------+------------------------+
| Contact List | Conversation View     |
| (scrollable) | (messages + input)    |
|              |                       |
| [*] Mom      |  [user] hey           |
| [ ] Dad      |  [ghost] Hi! Isaac... |
| [*] Alex     |  [user] is he free?   |
|              |  [ghost] He's in...   |
|              |                       |
|              | [___input________][>]  |
+-------------+------------------------+
```

**Left column** (~240px wide):
- **Search bar** at top -- filters contacts by name/phone as you type
- **Add button** (+) -- opens a small inline form to add a phone number + optional name
- Scrollable contact list below
- Each contact row shows:
  - Auto-reply toggle (small switch, like the ones in `SettingsPanel`)
  - Contact name (or phone if no name set)
  - Last message preview (truncated to ~40 chars)
  - Time ago (e.g., "2h", "3d")
  - Unread indicator? No -- there's no read tracking. Skip this.
- Clicking a contact loads their conversation in the right column
- Active contact is highlighted with `var(--accent-dim)` background

**Right column** (flex: 1):
- **Conversation header** at top showing contact name, phone number, and auto-reply status
- **Message thread** (scrollable, oldest at top, newest at bottom)
  - User messages (inbound SMS) aligned right, teal bubble background (matching existing chat bubbles)
  - GHOST messages aligned left, no bubble, with "GHOST" label
  - Messages render markdown using `<ReactMarkdown>` (same as chat tab) wrapped in `className="ghost-md"`
  - Timestamp under each message cluster (group by date)
  - **Lazy loading**: when scrolled to top, load older messages via `before` cursor. Show a small "loading..." indicator. Stop when `has_more` is false.
- **Input bar** at bottom (matches chat tab input bar style)
  - Textarea + send button
  - Sends via `POST /sms/send` with the contact's phone number
  - After send, append the message to the local thread AND scroll to bottom
  - Placeholder: "Send SMS to {name}..."

### 3. Contact list behavior

- **Initial load**: `GET /sms/contacts` when `SmsPanel` mounts. Store in local state.
- **Search**: client-side filter on the fetched contacts array (filter by display_name or phone containing the search string, case-insensitive).
- **Add contact**: small inline form below the search bar (appears when + is clicked):
  - Phone input (required) -- just a text field, user types `+1...`
  - Name input (optional)
  - "Add" button -- calls `PUT /sms/contacts/{phone}/name` to set the name, then refreshes the contact list
  - No need to validate phone format on the frontend
- **Auto-reply toggle**: inline toggle switch on each contact row. Calls `POST /sms/contacts/{phone}/auto-reply` with `{"enabled": !current}`. Update local state optimistically.
- **Rename contact**: double-click the contact name to edit inline (same pattern as project rename in `ProjectItem`). Calls `PUT /sms/contacts/{phone}/name`.

### 4. Conversation loading

When a contact is selected:
1. Call `GET /sms/history/{encodeURIComponent(phone)}?limit=30`
2. Store messages in component state keyed by phone number (cache across tab switches)
3. Auto-scroll to bottom
4. When user scrolls to top, if `has_more` was true, call the endpoint again with `before={oldest_message_id}` and prepend results. Preserve scroll position (don't jump to top after prepend).

### 5. Schedule panel

Add a **"Schedule"** button in the header row of `SmsPanel`. Clicking it toggles a **slide-out panel** or **modal overlay** on the right side showing Isaac's schedule management UI.

The schedule panel has two sections:

**Persistent commitments** (top):
- Header: "RECURRING" (small caps, `--text-dim`)
- List of persistent entries, each with:
  - The content text
  - A delete (x) button
- Add form at bottom: text input + "Add" button
- These persist until manually deleted

**Daily schedule** (bottom):
- Header: "DAILY" (small caps)
- A **date picker** (native `<input type="date">`) defaulting to today
- List of entries for the selected date, each with content + delete button
- Add form: text input + "Add" button, sends with `day_date` = selected date
- Past dates' entries still show but are visually dimmed

API calls:
- `GET /schedule` on mount -- populate both sections
- `POST /schedule` with `kind` + optional `day_date` to add
- `DELETE /schedule/{id}` to remove

The schedule panel should feel lightweight -- not a full calendar, just a quick text-entry system.

### 6. Sending SMS from dashboard

When the user types a message in the conversation input and hits Enter (or clicks send):

1. Optimistically append the message to the thread as `{role: "user", content: text}` (note: from the dashboard, the role is "assistant" since Isaac is speaking through GHOST -- but actually, when Isaac sends manually, it should look different than GHOST auto-reply. Use role `"assistant"` since it's going out from GHOST's phone.)
   
   Actually, think about this carefully: when Isaac sends from the dashboard, the SMS goes out from GHOST's phone number. The recipient sees it as a message from the same number GHOST uses. So in `sms_history`, it should be stored as role `"assistant"`. The dashboard should render it as a sent message (right-aligned, teal bubble) but with a small "manual" indicator to distinguish from auto-replies.

2. Call `POST /sms/send` with `{"to": phone, "body": text}`. The backend stores it in `sms_history` and sends via SMS gateway.

3. On success, update the message with the returned `message_id`.

4. On failure, show the message with an error indicator (red border or "failed to send" text).

---

## Component structure (all in App.jsx)

Add these components after the existing `AboutPanel` component (~line 1441):

```
// --- SMS Panel -------------------------------------------------------
function SmsPanel({ daemonKey }) { ... }           // main container
function SmsContactList({ ... }) { ... }           // left column
function SmsContactRow({ ... }) { ... }            // single contact row
function SmsConversation({ ... }) { ... }          // right column: messages + input
function SmsSchedulePanel({ ... }) { ... }         // schedule overlay
```

### State in SmsPanel

```jsx
function SmsPanel({ daemonKey }) {
  const [contacts, setContacts] = useState([])
  const [loading, setLoading] = useState(true)
  const [selectedPhone, setSelectedPhone] = useState(null)
  const [search, setSearch] = useState('')
  const [showSchedule, setShowSchedule] = useState(false)
  const [showAddForm, setShowAddForm] = useState(false)

  // Conversation cache: { [phone]: { messages: [], hasMore: bool, loading: bool } }
  const [convos, setConvos] = useState({})

  // Schedule state
  const [scheduleEntries, setScheduleEntries] = useState([])

  useEffect(() => {
    // Load contacts on mount
    loadContacts()
  }, [])

  async function loadContacts() { ... }
  async function loadConversation(phone, before = null) { ... }
  async function sendMessage(phone, text) { ... }
  async function toggleAutoReply(phone, enabled) { ... }

  // ... render
}
```

---

## Styling guidelines

Match the existing dashboard aesthetic exactly:

- **Contact list background**: `var(--surface)` with `borderRight: '1px solid var(--border)'`
- **Active contact**: `background: 'var(--accent-dim)'`
- **Auto-reply toggle**: same as the toggle in `SettingRow` (36x20px pill switch)
- **Message bubbles**: reuse the same styles from `ChatThread` (line ~930-984):
  - User/inbound: right-aligned, `rgba(45,212,191,0.06)` background, `14px 14px 4px 14px` border-radius
  - GHOST/outbound: left-aligned, no bubble background, agent label above
- **Input bar**: same as chat input (textarea + round send button)
- **Search bar**: same style as the auth key input (mono font, `var(--surface)` background, `var(--border)` border)
- **Schedule panel**: slide-out from right, `var(--surface)` background, 360px wide, full height, with a semi-transparent backdrop

### Time formatting helper

Add a helper for "time ago" display:

```jsx
function timeAgo(isoString) {
  const secs = Math.floor((Date.now() - new Date(isoString).getTime()) / 1000)
  if (secs < 60) return 'now'
  if (secs < 3600) return `${Math.floor(secs / 60)}m`
  if (secs < 86400) return `${Math.floor(secs / 3600)}h`
  if (secs < 604800) return `${Math.floor(secs / 86400)}d`
  return new Date(isoString).toLocaleDateString()
}
```

---

## Wire it into the main App

### In the main render (App component, ~line 1830):

Add the SMS panel alongside the other nav panels:

```jsx
{activeNav === 'sms' && <SmsPanel daemonKey={daemonKey} />}
{activeNav === 'settings' && <SettingsPanel />}
{activeNav === 'statistics' && <StatisticsPanel />}
{activeNav === 'about' && <AboutPanel />}
```

### In handleNav:

No changes needed -- `handleNav` already toggles `activeNav` generically.

---

## Edge cases to handle

1. **Empty state**: No contacts yet. Show centered text: "No SMS conversations yet. Messages will appear here when GHOST receives texts."
2. **No conversation selected**: Right column shows "Select a contact to view messages" (similar to `NoChatSelected`).
3. **Send to new number**: The Add form creates a contact row. Selecting it loads an empty conversation. User can type and send the first message.
4. **Phone number encoding**: When calling `/sms/history/{phone}`, use `encodeURIComponent(phone)` since phone numbers contain `+`.
5. **Scroll preservation on lazy load**: When prepending older messages, calculate the scroll height difference and adjust `scrollTop` to keep the user's viewport in place.
6. **Optimistic updates**: Toggle auto-reply immediately in local state, revert on API error.
7. **Schedule date default**: The date picker in the daily section defaults to today. Past dates' entries are dimmed (`opacity: 0.5`) but still deletable.
8. **Refresh contacts**: Add a small refresh button (circular arrow icon or just "refresh" text) next to the search bar. Clicking it re-fetches `/sms/contacts`.

---

## Verification

1. `cd dashboard && npm run dev` -- dashboard loads without errors
2. Navigate to SMS tab -- contacts load from API
3. Click a contact -- conversation loads with messages
4. Scroll to top -- older messages lazy-load
5. Type and send a message -- SMS goes out, message appears in thread
6. Toggle auto-reply -- switch flips, API call succeeds
7. Open schedule panel -- entries load, can add/delete
8. Search contacts -- filters correctly
9. Add new contact -- appears in list
10. Rename contact -- inline edit works

---

## Files touched

- `dashboard/src/App.jsx` -- add `SmsPanel`, `SmsContactList`, `SmsContactRow`, `SmsConversation`, `SmsSchedulePanel` components + update `SidebarNav` items array + add rendering in main `App`
- No other files need changes (CSS variables are already defined in `index.css`)
