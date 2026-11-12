/**
 * The Traces screen: `@iii-dev/traces-ui` over this engine, with the ADE's
 * chat sessions, `@trace` focus requests, the `console` configuration entry
 * (saved views, follow turns, span filters) and the function-call renderers
 * workers inject.
 */

import {
  createEngineTraceSource,
  type TraceChatLink,
  type TracesSessions,
  type TracesSettingsStore,
  TracesView,
} from '@iii-dev/traces-ui'
import { useQueryClient } from '@tanstack/react-query'
import { GitBranch } from 'lucide-react'
import { useCallback, useMemo } from 'react'
import { useFunctionCallRenderers } from '@/components/function-call/renderer-registry'
import { PageHeader } from '@/components/ui/PageChrome'
import {
  CONSOLE_CONFIG_QUERY_KEY,
  consoleConfigWriter,
} from '@/hooks/lib/console-config-writer'
import { useConversationsCtxOptional } from '@/lib/conversations-context'
import { getIiiClient } from '@/lib/iii-client'
import { subscribeTraceFocus } from '@/lib/trace-focus'
import { requestChatMessageFocus } from '@/lib/trace-links'
import type { PageCommandsApi } from '@/types/injectable-ui'

const source = createEngineTraceSource(getIiiClient)

export interface TracesV2Props {
  /** mount with a trace's detail already expanded (stories/deep links) */
  initialTraceId?: string
  /** Close the hosting pane — the header's standard ✕ when present. */
  onRequestClose?: () => void
  /** The pane's command registrar: the page's verbs and keys. */
  commands?: PageCommandsApi
}

export function TracesV2({
  initialTraceId,
  onRequestClose,
  commands,
}: TracesV2Props) {
  // The traces settings share the `console` entry, and its one writer,
  // with every other feature that read-modify-writes it.
  const qc = useQueryClient()
  const settings = useMemo((): TracesSettingsStore => {
    const writer = consoleConfigWriter(qc)
    return {
      queryKey: CONSOLE_CONFIG_QUERY_KEY,
      read: () => writer.readForQuery(),
      update: (transform, fallback) => writer.enqueue(transform, fallback),
      settled: () => writer.whenIdle(),
    }
  }, [qc])

  const renderers = useFunctionCallRenderers()

  // Outside the app shell (Storybook) there is no chat: no session scope,
  // no follow toggle, no "open the chat" link.
  const conversationsCtx = useConversationsCtxOptional()
  const active = conversationsCtx?.active ?? null
  const activeId = conversationsCtx?.activeId ?? null
  const conversations = conversationsCtx?.conversations
  const openConversation = conversationsCtx?.openConversation
  const open = useCallback(
    (link: TraceChatLink) => {
      // Focus first, open second: the store holds the request before the
      // chat pane mounts, so a freshly placed ChatView reads it on render.
      if (link.turnId) {
        requestChatMessageFocus({
          sessionId: link.sessionId,
          turnId: link.turnId,
        })
      }
      openConversation?.(link.sessionId)
    },
    [openConversation],
  )
  const sessions = useMemo(
    (): TracesSessions | undefined =>
      conversationsCtx
        ? { active, activeId, sessions: conversations ?? [], open }
        : undefined,
    [conversationsCtx, active, activeId, conversations, open],
  )

  return (
    <TracesView
      source={source}
      settings={settings}
      functionCallRenderers={renderers}
      sessions={sessions}
      subscribeFocus={subscribeTraceFocus}
      commands={commands}
      initialTraceId={initialTraceId}
      // learn-iii-traces: tour anchor (workers/learn-iii) — a selector,
      // not styling. Do not remove.
      className="learn-iii-traces font-inter"
      header={
        <PageHeader
          icon={<GitBranch />}
          title="Traces"
          onClose={onRequestClose}
        />
      }
    />
  )
}
