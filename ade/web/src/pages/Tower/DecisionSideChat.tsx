import { errorMessage } from '@iii-dev/console-ui/format'
import { Lock, X } from 'lucide-react'
import { type ReactNode, useEffect, useRef, useState } from 'react'
import { ChatView } from '@/components/chat/ChatView'
import { Button } from '@/components/ui/Button'
import { IconButton } from '@/components/ui/IconButton'
import { Select } from '@/components/ui/Select'
import { Skeleton } from '@/components/ui/Skeleton'
import { StatusPanel } from '@/components/ui/StatusPanel'
import {
  SIDE_CHAT_KEEP_KEY,
  type SideChatQuote,
  sideChatFrom,
  sideChatMessages,
  sideChatQuotes,
} from '@/lib/chat-aside'
import { useConversationsCtxOptional } from '@/lib/conversations-context'
import { withDecisionSideChatLock } from '@/lib/decision-side-chat-lifetime'
import type { SessionMeta } from '@/lib/sessions/types'
import { formatTokenCount } from '@/lib/token-estimate'
import type { AskQuestion, TowerQuestion } from './model'
import type { TowerActions } from './NeedsYou'
import { createDecisionSideChat, type DecisionNeed } from './side-chat'

export function DecisionSideChat({
  item,
  record,
  iii,
  onClose,
  onUseAnswer,
  editable,
  closed,
}: {
  item: DecisionNeed
  record?: TowerQuestion
  iii: TowerActions
  onClose: () => void
  onUseAnswer: (index: number, text: string) => void
  editable: boolean
  closed: boolean
}) {
  const ctx = useConversationsCtxOptional()
  const [copy, setCopy] = useState<SessionMeta | null>(null)
  const [quote, setQuote] = useState<SideChatQuote | null>(null)
  const [fresh, setFresh] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [attempt, setAttempt] = useState(0)
  const [keeping, setKeeping] = useState(false)
  const [kept, setKept] = useState(false)
  const [context, setContext] = useState<number | null>(null)
  const seed = useRef({ item, record })
  const contextUsage = ctx?.backend.contextUsage
  useEffect(() => {
    let active = true
    void contextUsage?.(item.sessionId)
      .then((report) => {
        if (active) setContext(report?.total ?? null)
      })
      .catch(() => {})
    return () => {
      active = false
    }
  }, [contextUsage, item.sessionId])
  // biome-ignore lint/correctness/useExhaustiveDependencies: attempt retries a failed preparation.
  useEffect(() => {
    let active = true
    setError(null)
    void createDecisionSideChat(
      iii,
      seed.current.item,
      seed.current.record,
    ).then(
      (result) => {
        if (!active) return
        setCopy(result.meta)
        setQuote(result.quote)
        setFresh(!result.meta.forked_from)
        setKept(result.meta.metadata?.[SIDE_CHAT_KEEP_KEY] === true)
      },
      (error) => {
        if (active) setError(errorMessage(error))
      },
    )
    return () => {
      active = false
    }
  }, [iii, attempt])
  const watch = ctx?.watchConversation
  const copyId = copy?.session_id
  useEffect(() => (copyId ? watch?.(copyId) : undefined), [copyId, watch])
  const conversation = ctx?.conversations.find((row) => row.id === copyId)
  const isKept =
    kept || conversation?.sessionMetadata?.[SIDE_CHAT_KEEP_KEY] === true
  useEffect(() => {
    if (closed && copy && !isKept) onClose()
  }, [closed, copy, isKept, onClose])
  const since = copy?.created_at ?? 0
  const from = sideChatFrom(conversation ?? { sessionMetadata: copy?.metadata })
  const own = conversation
    ? sideChatMessages(conversation.messages, since, from)
    : []
  const reply = own
    .filter((message) => message.role === 'assistant' && message.content.trim())
    .at(-1)
  const keep = async () => {
    if (!copy || keeping) return
    setKeeping(true)
    setError(null)
    try {
      await withDecisionSideChatLock(copy.session_id, async () => {
        const latest = await iii.call<{ meta: SessionMeta } | null>(
          'session::get',
          { session_id: copy.session_id },
        )
        if (!latest) throw new Error('The side chat is no longer available.')
        const metadata = { ...latest.meta.metadata, [SIDE_CHAT_KEEP_KEY]: true }
        await iii.call('session::set-meta', {
          session_id: copy.session_id,
          metadata,
        })
        setCopy({ ...latest.meta, metadata })
        setKept(true)
      })
    } catch (error) {
      setError(errorMessage(error))
    } finally {
      setKeeping(false)
    }
  }
  return (
    <DecisionSideChatPanel
      title={item.sessionTitle ?? item.sessionId}
      context={context}
      fresh={fresh}
      kept={isKept}
      keeping={keeping}
      onKeep={() => void keep()}
      onClose={onClose}
      proposal={reply?.role === 'assistant' ? reply.content : undefined}
      questions={item.kind === 'question' ? item.entry.questions : []}
      onUseAnswer={
        editable && conversation?.status !== 'working' ? onUseAnswer : undefined
      }
      canKeep={!!copy}
    >
      {error ? (
        <StatusPanel
          variant="alert"
          role="alert"
          headline="Side chat could not be prepared"
          detail={error}
          action={
            !copy ? (
              <Button
                variant="ghost"
                size="sm"
                onClick={() => setAttempt((value) => value + 1)}
              >
                Retry
              </Button>
            ) : undefined
          }
        />
      ) : null}
      {conversation && ctx ? (
        <ChatView
          key={conversation.id}
          conversation={conversation}
          side={{
            since,
            fromEntry: from,
            empty: (
              <p className="tower-muted">
                Ask the copy about this pending decision.
              </p>
            ),
            autoFocus: true,
            quote: own.some((message) => message.role === 'user')
              ? null
              : quote,
            onQuoteSent: () => setQuote(null),
            quotes: sideChatQuotes(conversation),
            onJumpToQuote: () => {},
            compact: true,
          }}
          backend={ctx.backend}
          modelOptions={ctx.modelOptions}
          catalogLoading={ctx.catalogLoading}
          density="dock"
          onUpdateModel={ctx.setModel}
          onUpdateThinkingLevel={ctx.setThinkingLevel}
          onUpdateWorkingDir={ctx.setWorkingDir}
          onAppendMessage={ctx.appendMessage}
          onPatchMessage={ctx.updateMessage}
          onCompactConversation={ctx.compactConversation}
        />
      ) : !error ? (
        <div
          role="status"
          aria-label="Preparing side chat"
          className="tower-stack p-3"
        >
          <Skeleton className="h-4 w-3/4" />
          <Skeleton className="h-4 w-1/2" />
        </div>
      ) : null}
    </DecisionSideChatPanel>
  )
}

export function DecisionSideChatPanel({
  title,
  context,
  fresh,
  kept,
  keeping,
  canKeep = true,
  onKeep,
  onClose,
  children,
  proposal,
  questions = [],
  onUseAnswer,
}: {
  title: string
  context?: number | null
  fresh?: boolean
  kept: boolean
  keeping?: boolean
  canKeep?: boolean
  onKeep: () => void
  onClose: () => void
  children: ReactNode
  proposal?: string
  questions?: AskQuestion[]
  onUseAnswer?: (index: number, text: string) => void
}) {
  const [target, setTarget] = useState('0')
  const panel = useRef<HTMLElement>(null)
  return (
    <aside
      ref={panel}
      className="tower-side-chat"
      aria-label="Decision side chat"
    >
      <div className="tower-side-chat-header">
        <h2>Side chat</h2>
        <span className="tower-meta">read-only copy of {title}</span>
        <Button
          variant="ghost"
          size="sm"
          disabled={!canKeep || kept || keeping}
          aria-pressed={kept}
          onClick={onKeep}
        >
          {kept ? 'Kept' : keeping ? 'Keeping…' : 'Keep this side chat'}
        </Button>
        <IconButton label="Close the side chat" onClick={onClose}>
          <X className="size-4" />
        </IconButton>
      </div>
      <p className="tower-side-chat-note tower-inline-note">
        <Lock className="size-4" />
        <span>
          {fresh
            ? 'Session gone · a fresh copy has the question, card and trail.'
            : 'Forked before the pending call.'}
          {context !== null && context !== undefined
            ? ` ${formatTokenCount(context)} tokens.`
            : ''}{' '}
          It can read, but cannot act or answer for you.
          {kept
            ? ' Kept in your chat list.'
            : ' Deleted when the decision closes.'}
        </span>
      </p>
      {children}
      {proposal && questions.length && onUseAnswer ? (
        <div className="tower-use-answer">
          {questions.length > 1 ? (
            <Select
              aria-label="Question to fill"
              value={target}
              onChange={setTarget}
              options={questions.map((question, index) => ({
                value: String(index),
                label: question.header,
              }))}
            />
          ) : null}
          <Button
            variant="pill"
            size="sm"
            title="Uses selected text, or the latest reply."
            onClick={() => {
              const selection = window.getSelection()
              const selected =
                selection?.anchorNode &&
                panel.current?.contains(selection.anchorNode) &&
                selection.focusNode &&
                panel.current.contains(selection.focusNode)
                  ? selection.toString().trim()
                  : ''
              onUseAnswer(Number(target), selected || proposal)
            }}
          >
            Use this answer
          </Button>
          <span className="tower-muted">
            Fills the decision · you still send it with Answer.
          </span>
        </div>
      ) : null}
    </aside>
  )
}
