import { SIDE_CHAT_KEEP_KEY, SIDE_CHAT_QUESTION_KEY } from '@/lib/chat-aside'
import type { IiiClient } from '@/lib/iii-client'
import type { SessionMeta } from '@/lib/sessions/types'
import type { Conversation } from '@/types/chat'

const QUESTION_KIND_KEY = 'side_chat_question_kind'
const OPEN_ASK_STATES = new Set(['open', 'expired', 'answering'])
const ASK_HANDLER = 'iii::console::decision_side_chat_ask_settled'
const HOLD_HANDLER = 'iii::console::decision_side_chat_hold_resolved'

type Client = Pick<
  IiiClient,
  'call' | 'on' | 'observe' | 'browserId' | 'addConnectionStateListener'
>
type Kind = 'question' | 'held'
interface DecisionCopy {
  id: string
  metadata?: Record<string, unknown>
}
interface AskEntry {
  session_id: string
  question_id: string
  state: string
}
interface HoldRecord {
  session_id: string
  function_call_id: string
}
interface BoardQuestion {
  key: string
  state: string
}

/** Serialize Keep and cleanup across tabs in this browser. */
export function withDecisionSideChatLock<T>(
  id: string,
  run: () => Promise<T>,
): Promise<T> {
  const locks = typeof navigator === 'undefined' ? undefined : navigator.locks
  return locks
    ? locks.request(`iii-decision-side-chat-lifetime:${id}`, run)
    : run()
}

function localCopies(conversations: readonly Conversation[]): DecisionCopy[] {
  return conversations.flatMap((conversation) => {
    const metadata = conversation.sessionMetadata
    return typeof metadata?.[SIDE_CHAT_QUESTION_KEY] === 'string' &&
      metadata[SIDE_CHAT_KEEP_KEY] !== true
      ? [{ id: conversation.id, metadata }]
      : []
  })
}

async function listCopies(client: Client): Promise<DecisionCopy[]> {
  const copies: DecisionCopy[] = []
  let cursor: string | undefined
  do {
    const page = await client.call<{
      sessions?: SessionMeta[]
      next_cursor?: string
    }>('session::list', {
      limit: 200,
      ...(cursor ? { cursor } : {}),
    })
    for (const session of page.sessions ?? []) {
      if (
        typeof session.metadata?.[SIDE_CHAT_QUESTION_KEY] === 'string' &&
        session.metadata[SIDE_CHAT_KEEP_KEY] !== true
      )
        copies.push({ id: session.session_id, metadata: session.metadata })
    }
    cursor = page.next_cursor
  } while (cursor)
  return copies
}

async function listAsks(client: Client): Promise<AskEntry[]> {
  const entries: AskEntry[] = []
  let cursor: string | undefined
  do {
    const page = await client.call<{ entries?: AskEntry[]; cursor?: string }>(
      'harness::ask::list',
      { limit: 200, ...(cursor ? { cursor } : {}) },
    )
    entries.push(...(page.entries ?? []))
    cursor = page.cursor
  } while (cursor)
  return entries
}

async function listHolds(client: Client): Promise<HoldRecord[]> {
  const pending: HoldRecord[] = []
  let cursor: string | undefined
  do {
    const page = await client.call<{
      pending?: HoldRecord[]
      next_cursor?: string
    }>('approval::list-pending', {
      limit: 200,
      ...(cursor ? { cursor } : {}),
    })
    pending.push(...(page.pending ?? []))
    cursor = page.next_cursor
  } while (cursor)
  return pending
}

async function listBoardQuestions(client: Client): Promise<BoardQuestion[]> {
  const questions: BoardQuestion[] = []
  let cursor: string | undefined
  do {
    const page = await client.call<{
      questions?: Array<{
        state: string
        comment?: {
          question?: { session_id?: string; question_id?: string }
        }
      }>
      cursor?: string
    }>('kanban::question::list', {
      limit: 500,
      ...(cursor ? { cursor } : {}),
    })
    for (const row of page.questions ?? []) {
      const question = row.comment?.question
      if (question?.session_id && question.question_id)
        questions.push({
          key: `${question.session_id}/${question.question_id}`,
          state: row.state,
        })
    }
    cursor = page.cursor
  } while (cursor)
  return questions
}

/**
 * Keep decision copies temporary across Tower unmounts. Settled events close
 * them immediately; successful inbox reads catch up after reload/reconnect.
 */
export function startDecisionSideChatLifetime(
  client: Client,
  conversations: () => readonly Conversation[],
  remove: (id: string) => Promise<void>,
): () => void {
  let active = true
  const deleting = new Set<string>()
  const knownCopies = new Map<string, DecisionCopy>()
  const currentCopies = () => {
    for (const copy of localCopies(conversations()))
      knownCopies.set(copy.id, copy)
    return [...knownCopies.values()]
  }
  const removeMatching = async (key: string, kind: Kind) => {
    const copies = currentCopies().filter(
      (copy) =>
        copy.metadata?.[SIDE_CHAT_QUESTION_KEY] === key &&
        copy.metadata[QUESTION_KIND_KEY] === kind,
    )
    await Promise.all(
      copies.map(async (copy) => {
        if (!active || deleting.has(copy.id)) return
        deleting.add(copy.id)
        try {
          await withDecisionSideChatLock(copy.id, async () => {
            const session = await client.call<{ meta: SessionMeta } | null>(
              'session::get',
              { session_id: copy.id },
            )
            if (
              session?.meta.metadata?.[SIDE_CHAT_KEEP_KEY] !== true &&
              typeof session?.meta.metadata?.[SIDE_CHAT_QUESTION_KEY] ===
                'string'
            ) {
              await remove(copy.id)
              knownCopies.delete(copy.id)
            }
          })
        } catch {
          // A later source event, list refresh or reconnect retries cleanup.
        } finally {
          deleting.delete(copy.id)
        }
      }),
    )
  }
  const reconcile = async () => {
    const [copiesResult, asksResult, holdsResult, boardResult] =
      await Promise.allSettled([
        listCopies(client),
        listAsks(client),
        listHolds(client),
        listBoardQuestions(client),
      ])
    if (!active) return
    if (copiesResult.status === 'fulfilled')
      for (const copy of copiesResult.value) knownCopies.set(copy.id, copy)
    const copies = currentCopies()
    if (asksResult.status === 'fulfilled') {
      const asks = new Map(
        asksResult.value.map((ask) => [
          `${ask.session_id}/${ask.question_id}`,
          ask,
        ]),
      )
      const board =
        boardResult.status === 'fulfilled'
          ? new Map(boardResult.value.map((row) => [row.key, row]))
          : null
      await Promise.all(
        copies.flatMap((copy) => {
          if (copy.metadata?.[QUESTION_KIND_KEY] !== 'question') return []
          const key = copy.metadata[SIDE_CHAT_QUESTION_KEY] as string
          const ask = asks.get(key)
          if (ask)
            return OPEN_ASK_STATES.has(ask.state)
              ? []
              : [removeMatching(key, 'question')]
          const mirrored = board?.get(key)
          return mirrored && mirrored.state !== 'open'
            ? [removeMatching(key, 'question')]
            : []
        }),
      )
    }
    if (holdsResult.status === 'fulfilled') {
      const open = new Set(
        holdsResult.value.map(
          (hold) => `${hold.session_id}/${hold.function_call_id}`,
        ),
      )
      await Promise.all(
        copies.flatMap((copy) => {
          if (copy.metadata?.[QUESTION_KIND_KEY] !== 'held') return []
          const key = copy.metadata[SIDE_CHAT_QUESTION_KEY] as string
          return open.has(key) ? [] : [removeMatching(key, 'held')]
        }),
      )
    }
  }
  const bind = <T extends object>(
    handler: string,
    source: string,
    settled: (event: T) => void,
  ) => {
    const off = client.on(handler, (payload: unknown) => {
      if (active && payload && typeof payload === 'object')
        settled(payload as T)
    })
    const unobserve = client.observe(
      { function_id: source },
      `${handler}::${client.browserId}`,
    )
    return () => {
      off()
      try {
        unobserve()
      } catch {}
    }
  }
  const stops = [
    bind<AskEntry>(ASK_HANDLER, 'harness::on-ask-settled', (entry) => {
      if (!OPEN_ASK_STATES.has(entry.state))
        void removeMatching(
          `${entry.session_id}/${entry.question_id}`,
          'question',
        )
    }),
    bind<HoldRecord>(
      HOLD_HANDLER,
      'approval::on-pending-resolved',
      (event) =>
        void removeMatching(
          `${event.session_id}/${event.function_call_id}`,
          'held',
        ),
    ),
    client.addConnectionStateListener((state) => {
      if (state === 'connected') void reconcile()
    }),
  ]
  void reconcile()
  return () => {
    active = false
    for (const stop of stops) stop()
  }
}
