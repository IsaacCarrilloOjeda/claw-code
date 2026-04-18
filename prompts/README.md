# GHOST Implementation Prompts

Each file is a self-contained prompt for a Claude Code instance. Run them **in order** -- each phase builds on the previous one.

## Completed phases (SMS identity + safety)
- [Phase 1](phase1-sender-identity.md) -- Sender identity injection (contacts.rs)
- [Phase 2](phase2-outbound-guard.md) -- Outbound reply guard
- [Phase 3](phase3-ack-and-history.md) -- Auto-ack + conversation history

## Current phases (SMS polish + dashboard)

| Phase | File | Scope | Dependencies |
|-------|------|-------|-------------|
| **4** | [phase4-sms-polish.md](phase4-sms-polish.md) | Strip markdown from SMS + HTML `/read/{id}` endpoint | None (backend only) |
| **5** | [phase5-sms-backend.md](phase5-sms-backend.md) | SMS history API, auto-reply DB table, schedule storage, context injection | Phase 4 (same codebase, no code deps) |
| **6** | [phase6-sms-dashboard.md](phase6-sms-dashboard.md) | Dashboard SMS tab: conversations, contact management, schedule UI | Phase 5 (consumes its API endpoints) |

## How to use

1. Open a fresh Claude Code session
2. Paste the phase prompt (or reference the file)
3. Let it read the relevant source files and implement
4. Verify with the commands listed at the bottom of each prompt
5. Commit, then move to the next phase

Phases 4 and 5 can technically run in parallel (4 is frontend-independent, 5 is backend-only). Phase 6 requires Phase 5's endpoints to exist.
