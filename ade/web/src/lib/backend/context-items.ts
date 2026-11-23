/**
 * What a conversation gathered, from the engine (`harness::context-items`,
 * chat-projects §1.2.4, D10): subagents, pull requests, links, attachments
 * and images, newest first, for the whole conversation — not only the window
 * of transcript this tab has loaded. The pane reads it when the engine has it
 * (`ChatCapabilities.contextItems`); without it the pane keeps parsing the
 * loaded transcript (`lib/chat-aside.ts`).
 *
 * Refresh rules (§1.2.4): when the conversation opens (for the badge count),
 * when the pane opens, at `harness::on-turn-completed`, on new messages
 * (debounced 2 s) while the pane is visible, and every 2 minutes while it is
 * visible (pull request states; the engine caches each for 120 s).
 */
import { useCallback, useEffect, useId, useRef, useState } from 'react'
import { getIiiClient } from '@/lib/iii-client'

export const CONTEXT_ITEMS_FUNCTION_ID = 'harness::context-items'
const TURN_COMPLETED_FUNCTION_ID = 'harness::on-turn-completed'

export type ItemDirection = 'sent' | 'received'

export interface ContextSubagentItem {
  session_id: string
  title: string
  status: string
  display?: string
  started_at_ms: number
  ended_at_ms?: number
  descendants: number
}

export interface ContextPullRequestItem {
  repo: string
  number: number
  url: string
  title?: string
  state?: 'open' | 'draft' | 'merged' | 'closed'
  checks?: 'passing' | 'failing' | 'pending'
  entry_id: string
  source: 'created' | 'mentioned'
}

export interface ContextLinkItem {
  url: string
  host: string
  title?: string
  entry_id: string
  count: number
}

export interface ContextAttachmentItem {
  attachment_id?: string
  name: string
  mime: string
  size?: number
  direction: ItemDirection
  entry_id: string
  /** A file a function wrote, when the engine names it. */
  path?: string
}

export interface ContextImageItem {
  attachment_id?: string
  mime: string
  direction: ItemDirection
  entry_id: string
  block_index: number
}

export interface ContextItems {
  session_id: string
  message_count: number
  generated_at_ms: number
  subagents: ContextSubagentItem[]
  pull_requests: ContextPullRequestItem[]
  links: ContextLinkItem[]
  attachments: ContextAttachmentItem[]
  images: ContextImageItem[]
  /** Phase 2; counted, not drawn, in phase 1. */
  side_chats: unknown[]
  truncated: { links?: boolean; attachments?: boolean; images?: boolean }
}

function record(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object'
    ? (value as Record<string, unknown>)
    : {}
}
function str(value: unknown): string | undefined {
  return typeof value === 'string' && value.length > 0 ? value : undefined
}
function num(value: unknown): number | undefined {
  return typeof value === 'number' && Number.isFinite(value) ? value : undefined
}
function list(value: unknown): Record<string, unknown>[] {
  return Array.isArray(value) ? value.map(record) : []
}
function direction(value: unknown): ItemDirection {
  return value === 'sent' ? 'sent' : 'received'
}
function oneOf<T extends string>(
  value: unknown,
  options: readonly T[],
): T | undefined {
  return options.includes(value as T) ? (value as T) : undefined
}

/** The response, defensively: unknown fields dropped, rows without ids skipped. */
export function parseContextItems(raw: unknown): ContextItems {
  const r = record(raw)
  return {
    session_id: str(r.session_id) ?? '',
    message_count: num(r.message_count) ?? 0,
    generated_at_ms: num(r.generated_at_ms) ?? 0,
    subagents: list(r.subagents).flatMap((s) => {
      const id = str(s.session_id)
      if (!id) return []
      return [
        {
          session_id: id,
          title: str(s.title) ?? id,
          status: str(s.status) ?? 'unknown',
          display: str(s.display),
          started_at_ms: num(s.started_at_ms) ?? 0,
          ended_at_ms: num(s.ended_at_ms),
          descendants: num(s.descendants) ?? 0,
        },
      ]
    }),
    pull_requests: list(r.pull_requests).flatMap((p) => {
      const repo = str(p.repo)
      const number = num(p.number)
      const url = str(p.url)
      if (!repo || number === undefined || !url) return []
      return [
        {
          repo,
          number,
          url,
          title: str(p.title),
          state: oneOf(p.state, ['open', 'draft', 'merged', 'closed'] as const),
          checks: oneOf(p.checks, ['passing', 'failing', 'pending'] as const),
          entry_id: str(p.entry_id) ?? '',
          source: p.source === 'created' ? 'created' : 'mentioned',
        } satisfies ContextPullRequestItem,
      ]
    }),
    links: list(r.links).flatMap((l) => {
      const url = str(l.url)
      if (!url) return []
      let host = str(l.host)
      if (!host) {
        try {
          host = new URL(url).host
        } catch {
          host = url
        }
      }
      return [
        {
          url,
          host,
          title: str(l.title),
          entry_id: str(l.entry_id) ?? '',
          count: num(l.count) ?? 1,
        },
      ]
    }),
    attachments: list(r.attachments).flatMap((a) => {
      const name = str(a.name)
      if (!name) return []
      return [
        {
          attachment_id: str(a.attachment_id),
          name,
          mime: str(a.mime) ?? 'application/octet-stream',
          size: num(a.size),
          direction: direction(a.direction),
          entry_id: str(a.entry_id) ?? '',
          path: str(a.path),
        },
      ]
    }),
    images: list(r.images).map((i) => ({
      attachment_id: str(i.attachment_id),
      mime: str(i.mime) ?? 'image/png',
      direction: direction(i.direction),
      entry_id: str(i.entry_id) ?? '',
      block_index: num(i.block_index) ?? 0,
    })),
    side_chats: Array.isArray(r.side_chats) ? r.side_chats : [],
    truncated: {
      ...(record(r.truncated).links === true ? { links: true } : {}),
      ...(record(r.truncated).attachments === true
        ? { attachments: true }
        : {}),
      ...(record(r.truncated).images === true ? { images: true } : {}),
    },
  }
}

export async function fetchContextItems(
  sessionId: string,
): Promise<ContextItems> {
  const client = await getIiiClient()
  return parseContextItems(
    await client.call(CONTEXT_ITEMS_FUNCTION_ID, { session_id: sessionId }),
  )
}

/**
 * Everything the pane would list, for the badge: subagents, pull requests
 * (less the dismissed ones), links, attachments, images and sidechats.
 */
export function contextItemCount(
  items: ContextItems,
  dismissedPrs: ReadonlySet<string> = new Set(),
): number {
  return (
    items.subagents.length +
    items.pull_requests.filter(
      (pr) => !dismissedPrs.has(`${pr.repo}#${pr.number}`),
    ).length +
    items.links.length +
    items.attachments.length +
    items.images.length +
    items.side_chats.length
  )
}

/** "16", "99+". */
export function badgeCount(count: number): string {
  return count > 99 ? '99+' : String(count)
}

export const MESSAGE_DEBOUNCE_MS = 2_000
export const PR_REFRESH_MS = 120_000

export interface UseContextItemsOptions {
  /** The engine has `harness::context-items` (and the backend is real). */
  enabled: boolean
  /** The pane or drawer is in sight. */
  visible: boolean
  /** Changes when a message lands (the transcript length, say). */
  messageTick: number
}

export interface ContextItemsState {
  items: ContextItems | null
  loading: boolean
  error: string | null
  refresh: () => void
}

/** The conversation's context items, kept current by the rules above. */
export function useContextItems(
  sessionId: string,
  { enabled, visible, messageTick }: UseContextItemsOptions,
): ContextItemsState {
  const [state, setState] = useState<{
    sessionId: string
    items: ContextItems | null
    loading: boolean
    error: string | null
  }>({ sessionId, items: null, loading: false, error: null })
  const seq = useRef(0)

  const refresh = useCallback(() => {
    if (!enabled) return
    const mine = ++seq.current
    setState((current) =>
      current.sessionId === sessionId
        ? { ...current, loading: true }
        : { sessionId, items: null, loading: true, error: null },
    )
    void fetchContextItems(sessionId).then(
      (items) => {
        if (mine !== seq.current) return
        setState({ sessionId, items, loading: false, error: null })
      },
      (err: unknown) => {
        if (mine !== seq.current) return
        setState((current) => ({
          sessionId,
          items: current.sessionId === sessionId ? current.items : null,
          loading: false,
          error: err instanceof Error ? err.message : String(err),
        }))
      },
    )
  }, [enabled, sessionId])

  // Opening the conversation (the badge count)…
  useEffect(() => {
    refresh()
  }, [refresh])
  // …and opening the pane (closing it reads nothing).
  const wasVisible = useRef(visible)
  useEffect(() => {
    if (visible && !wasVisible.current) refresh()
    wasVisible.current = visible
  }, [visible, refresh])

  // New messages while the pane is visible, debounced.
  const firstTick = useRef(true)
  // biome-ignore lint/correctness/useExhaustiveDependencies: messageTick is the trigger
  useEffect(() => {
    if (firstTick.current) {
      firstTick.current = false
      return
    }
    if (!visible || !enabled) return
    const timer = window.setTimeout(refresh, MESSAGE_DEBOUNCE_MS)
    return () => window.clearTimeout(timer)
  }, [messageTick])

  // Pull request states, every 2 minutes while visible.
  useEffect(() => {
    if (!visible || !enabled) return
    const timer = window.setInterval(refresh, PR_REFRESH_MS)
    return () => window.clearInterval(timer)
  }, [visible, enabled, refresh])

  // The end of each turn.
  const instanceId = useId().replace(/[^a-zA-Z0-9]/g, '')
  const refreshRef = useRef(refresh)
  refreshRef.current = refresh
  useEffect(() => {
    if (!enabled) return
    let cancelled = false
    const off: (() => void)[] = []
    void (async () => {
      try {
        const client = await getIiiClient()
        if (cancelled) return
        const localFnId = `ade::context-items-turn::${instanceId}`
        off.push(client.on(localFnId, () => refreshRef.current()))
        off.push(
          client.observe(
            {
              function_id: TURN_COMPLETED_FUNCTION_ID,
              filters: { 'input.session_id': { $eq: sessionId } },
            },
            `${localFnId}::${client.browserId}`,
          ),
        )
      } catch {
        // The status re-reads still refresh the pane.
      }
    })()
    return () => {
      cancelled = true
      for (const dispose of off) dispose()
    }
  }, [enabled, sessionId, instanceId])

  const current = state.sessionId === sessionId ? state : null
  return {
    items: enabled ? (current?.items ?? null) : null,
    loading: enabled && (current?.loading ?? true),
    error: current?.error ?? null,
    refresh,
  }
}
