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
  sideChatFrom,
  sideChatMessages,
  sideChatQuotes,
} from '@/lib/chat-aside'
import { useConversationsCtxOptional } from '@/lib/conversations-context'
import { withDecisionSideChatLock } from '@/lib/decision-side-chat-lifetime'
import { getIiiClient } from '@/lib/iii-client'
import type { SessionMeta } from '@/lib/sessions/types'
import type { DecisionChatRequest } from '@/types/injectable-ui'
import { useDecisionChatBinding } from './host'
import './decision-chat.css'
import { formatTokenCount } from '@/lib/token-estimate'
export function DecisionSideChat({
  sessionId,
  onClose,
}: {
  sessionId: string
  onClose: () => void
}) {
  const ctx = useConversationsCtxOptional()
  const binding = useDecisionChatBinding(sessionId)
  const request = binding?.request
  const [error, setError] = useState<string | null>(null)
  const [keeping, setKeeping] = useState(false)
  const [kept, setKept] = useState(false)
  const [context, setContext] = useState<number | null>(null)
  const sourceId = request?.sessionId
  const contextUsage = ctx?.backend.contextUsage
  useEffect(() => {
    let active = true
    if (!sourceId) return
    void contextUsage?.(sourceId)
      .then((report) => {
        if (active) setContext(report?.total ?? null)
      })
      .catch(() => {})
    return () => {
      active = false
    }
  }, [contextUsage, sourceId])
  const watch = ctx?.watchConversation
  useEffect(() => watch?.(sessionId), [sessionId, watch])
  const conversation = ctx?.conversations.find((row) => row.id === sessionId)
  const metadata = conversation?.sessionMetadata ?? binding?.copy.metadata
  const isKept = kept || metadata?.[SIDE_CHAT_KEEP_KEY] === true
  const closed = binding?.closed ?? false
  useEffect(() => {
    if (closed && !isKept) onClose()
  }, [closed, isKept, onClose])
  const since = binding?.copy.created_at ?? conversation?.createdAt ?? 0
  const from = sideChatFrom({ sessionMetadata: metadata })
  const quote = sideChatQuotes({ sessionMetadata: metadata })[0] ?? null
  const own = conversation
    ? sideChatMessages(conversation.messages, since, from)
    : []
  const reply = own
    .filter((message) => message.role === 'assistant' && message.content.trim())
    .at(-1)
  const keep = async () => {
    if (keeping) return
    setKeeping(true)
    setError(null)
    try {
      await withDecisionSideChatLock(sessionId, async () => {
        const iii = await getIiiClient()
        const latest = await iii.call<{ meta: SessionMeta } | null>(
          'session::get',
          { session_id: sessionId },
        )
        if (!latest) throw new Error('The side chat is no longer available.')
        const metadata = { ...latest.meta.metadata, [SIDE_CHAT_KEEP_KEY]: true }
        await iii.call('session::set-meta', {
          session_id: sessionId,
          metadata,
        })
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
      title={request?.sourceTitle ?? conversation?.title ?? sessionId}
      context={context}
      fresh={!from}
      kept={isKept}
      keeping={keeping}
      onKeep={() => void keep()}
      onClose={onClose}
      proposal={reply?.role === 'assistant' ? reply.content : undefined}
      questions={request?.questions}
      held={request?.kind === 'held'}
      onUseAnswer={
        binding?.editable && !closed && conversation?.status !== 'working'
          ? request?.onUseAnswer
          : undefined
      }
      canKeep={!!conversation}
    >
      {error ? (
        <StatusPanel
          variant="alert"
          role="alert"
          headline="Side chat could not be prepared"
          detail={error}
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
              <p className="decision-muted">
                Ask the copy about this pending decision.
              </p>
            ),
            autoFocus: true,
            quote: own.some((message) => message.role === 'user')
              ? null
              : quote,
            onQuoteSent: () => {},
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
          className="flex flex-col gap-3 p-3"
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
  held,
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
  questions?: DecisionChatRequest['questions']
  held?: boolean
  onUseAnswer?: DecisionChatRequest['onUseAnswer']
}) {
  const [target, setTarget] = useState('0')
  const [heldChoice, setHeldChoice] = useState('Deny')
  const panel = useRef<HTMLElement>(null)
  return (
    <aside
      ref={panel}
      className="decision-side-chat"
      aria-label="Decision side chat"
    >
      <div className="decision-side-chat-header">
        <h2>Side chat</h2>
        <span className="decision-meta">read-only copy of {title}</span>
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
      <p className="decision-side-chat-note decision-inline-note">
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
      {proposal || held || !onUseAnswer ? (
        <div className="decision-use-answer">
          {held ? (
            <Select
              aria-label="Held call decision to propose"
              value={heldChoice}
              onChange={setHeldChoice}
              options={['Deny', 'Allow once', 'Always allow'].map((label) => ({
                value: label,
                label,
              }))}
            />
          ) : questions.length > 1 ? (
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
            disabled={!onUseAnswer || (!held && !proposal)}
            onClick={() => {
              const selection = window.getSelection()
              const selected =
                selection?.anchorNode &&
                panel.current?.contains(selection.anchorNode) &&
                selection.focusNode &&
                panel.current.contains(selection.focusNode)
                  ? selection.toString().trim()
                  : ''
              if (held) onUseAnswer?.(0, { option: heldChoice })
              else {
                const text = selected || proposal || ''
                const option = questions[Number(target)]?.options.find(
                  (option) => option.label === text.trim(),
                )
                onUseAnswer?.(
                  Number(target),
                  option ? { option: option.label } : { text },
                )
              }
            }}
          >
            Use this answer
          </Button>
          <span className="decision-muted">
            Fills the decision · you still send it with Answer.
          </span>
        </div>
      ) : null}
    </aside>
  )
}
