import {
  BottomSheet,
  BottomSheetContent,
  Button,
  Card,
  EmptyState,
  Eyebrow,
  Input,
  List,
  ListGroup,
  ListGroupLabel,
  ListItem,
  PageSidebar,
  PanelHeader,
  SearchField,
  Skeleton,
  StatusDot,
  StatusPanel,
  type DecisionChatHandle,
  type DecisionChatRequest,
  type Host,
} from '@iii-dev/console-ui'
import { errorMessage, formatRelative } from '@iii-dev/console-ui/format'
import { usePaneState } from '@iii-dev/console-ui/hooks'
import { ChevronLeft, ChevronRight, Clock, Info, MessageSquare } from 'lucide-react'
import { type ReactNode, type Ref, useCallback, useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { formatAge, formatTokenCount } from './format'
import { useControlPlaneHost } from './host'
import {
  type AnswerState,
  answerTransition,
  isRetryableAnswerError,
  recommendedSelection,
  type Selection,
  selectedAnswers,
  selectOption,
  typeAnswer,
} from './answer-state'
import type { ControlPlaneOverviewData as ControlPlaneData } from './useControlPlaneOverview'
import { type AskEntry, type NeedsYouItem, type QuestionNeed, type ControlPlaneQuestion } from '../../src/types'
import { decisionChatRequest, selectionFromSideChat } from './side-chat'

export type ControlPlaneActions = Pick<Host['iii'], 'call'>
const IDLE: AnswerState = { status: 'idle' }
export const needLabel = (item: NeedsYouItem) =>
  item.kind === 'question'
    ? `Question · ${item.questionClass.replaceAll('_', ' ')}`
    : 'Stalled run'

export function stallVersion(item: NeedsYouItem, runs: ControlPlaneData['runs']): string {
  const run = runs.find((row) => row.session.session_id === item.sessionId)
  return JSON.stringify([
    item.createdAt,
    run?.session.updated_at,
    run?.status.turn_id,
    item.kind === 'stalled' ? item.armedWakes : null,
  ])
}

export function AskAboutButton({
  item,
  onClick,
  disabled,
  active,
  available = false,
  tokens = null,
  compact = false,
}: {
  item: NeedsYouItem
  onClick: () => void
  disabled?: boolean
  active?: boolean
  available?: boolean
  tokens?: number | null
  /** The Overview shows the action alone, as the canvas does; the detail shows the context size. */
  compact?: boolean
}) {
  if (!available) return null
  return (
    <Button
      variant="ghost"
      size="sm"
      disabled={disabled || item.kind === 'stalled'}
      onClick={onClick}
      aria-pressed={active}
      title={
        tokens === null
          ? 'Context size unavailable'
          : `${tokens.toLocaleString()} context tokens; the copy rereads this context.`
      }
    >
      <MessageSquare className="control-plane-size-4" />
      Ask about this
      {compact ? null : tokens !== null ? (
        <span className="control-plane-meta">{formatTokenCount(tokens)}</span>
      ) : (
        <span className="control-plane-meta">
          <span aria-hidden>—</span>
          <span className="control-plane-sr-only">Context size unavailable</span>
        </span>
      )}
    </Button>
  )
}

export function NeedsYou({
  data,
  visible = true,
  narrow = false,
  onFocusDecision,
  iii,
  chat,
  user,
  paneId = 'control-plane',
  initialSelectedId,
  initialAskAboutId,
  onLeave,
  openRuns,
}: {
  data: ControlPlaneData
  visible?: boolean
  narrow?: boolean
  onFocusDecision?: () => void
  iii?: ControlPlaneActions
  chat?: Pick<NonNullable<Host['chat']>, 'openDecisionChat' | 'rebindDecisionChat'>
  user: string
  paneId?: string
  initialSelectedId?: string | null
  initialAskAboutId?: string | null
  onLeave: (item: NeedsYouItem) => void
  openRuns: () => void
}) {
  const [phoneViewport, setPhoneViewport] = useState(() => window.matchMedia('(max-width: 767px)').matches)
  useEffect(() => {
    // BottomSheet is host phone chrome and is hidden at the viewport's md breakpoint.
    const viewport = window.matchMedia('(max-width: 767px)')
    const update = () => setPhoneViewport(viewport.matches)
    update()
    viewport.addEventListener('change', update)
    return () => viewport.removeEventListener('change', update)
  }, [])
  const phone = narrow && phoneViewport
  const [focused, setFocused] = useState<NeedsYouItem | null>(
    () => data.items.find((item) => item.id === initialSelectedId) ?? (!phoneViewport ? data.items[0] : null) ?? null,
  )
  useEffect(() => {
    if (visible && initialSelectedId) setFocused(data.items.find((item) => item.id === initialSelectedId) ?? null)
  }, [initialSelectedId, visible])
  const [release, setRelease] = useState('all')
  const [query, setQuery] = useState('')
  const [drafts, setDrafts] = useState<Record<string, Selection[]>>({})
  const handles = useRef(new Map<string, DecisionChatHandle>())
  const chatRequests = useRef(new Map<string, symbol>())
  const [chatError, setChatError] = useState<string | null>(null)
  const [filledFromChat, setFilledFromChat] = useState<string | null>(null)
  const [states, setStates] = useState<Record<string, AnswerState>>({})
  const inFlight = useRef(new Set<string>())
  const retries = useRef(new Map<string, () => Promise<void>>())
  const active = useRef(true)
  const currentItems = useRef(new Set(data.items.map((item) => item.id)))
  currentItems.current = new Set(data.items.map((item) => item.id))
  const selected = data.items.find((item) => item.id === focused?.id) ?? focused
  const record =
    selected?.kind === 'question'
      ? data.questions.find((row) => row.sessionId === selected.sessionId && row.questionId === selected.questionId)
      : undefined
  const entry = record?.harness ?? (selected?.kind === 'question' ? selected.entry : undefined)
  const selectedKey = selected?.id
  const inbox = useRef<HTMLDivElement>(null)
  const detailHeading = useRef<HTMLHeadingElement>(null)
  const returnFocusTo = useRef<string | null>(null)
  useLayoutEffect(() => {
    if (!visible) return
    if (selectedKey) {
      detailHeading.current?.focus()
    } else if (returnFocusTo.current) {
      const rows = inbox.current?.querySelectorAll<HTMLButtonElement>('[data-need-id]')
      Array.from(rows ?? []).find((row) => row.dataset.needId === returnFocusTo.current)?.focus()
      returnFocusTo.current = null
    }
  }, [selectedKey, visible, phone])
  const closeDetail = () => {
    returnFocusTo.current = selectedKey ?? null
    setFocused(null)
  }
  useEffect(() => {
    active.current = true
    return () => {
      active.current = false
      chatRequests.current.clear()
      for (const handle of handles.current.values()) handle.detach()
      handles.current.clear()
    }
  }, [])
  useEffect(() => {
    if (!entry || !selectedKey) return
    setStates((current) => ({
      ...current,
      [selectedKey]: answerTransition(current[selectedKey] ?? IDLE, {
        type: 'settled',
        entry,
      }),
    }))
  }, [entry, selectedKey])
  const state = selected
    ? (states[selected.id] ?? (entry ? answerTransition(IDLE, { type: 'settled', entry }) : IDLE))
    : IDLE
  const items = data.items.filter(
    (item) =>
      (release === 'all' || (release === 'unlinked' ? !item.card : item.card?.release_id === release)) &&
      `${item.title} ${item.sessionId} ${item.card?.key ?? ''} ${item.card?.title ?? ''}`
        .toLowerCase()
        .includes(query.toLowerCase()),
  )
  const fillFromChat = useCallback(
    (item: QuestionNeed, index: number, proposal: { option?: string; text?: string }) => {
      const text = proposal.option ?? proposal.text ?? ''
      setDrafts((current) => {
        const next = [...(current[item.id] ?? item.entry.questions.map(recommendedSelection))]
        next[index] = selectionFromSideChat(item.entry.questions[index], text)
        return { ...current, [item.id]: next }
      })
      setFocused(item)
      onFocusDecision?.()
      setFilledFromChat(item.id)
      if (phone) handles.current.get(item.id)?.close()
    },
    [phone, onFocusDecision],
  )
  const requestFor = useCallback(
    (item: QuestionNeed): DecisionChatRequest =>
      decisionChatRequest(
        item,
        data.questions.find((row) => row.key === `${item.sessionId}/${item.questionId}`),
        (index, proposal) => fillFromChat(item, index, proposal),
      ),
    [data.questions, fillFromChat],
  )
  const liveChatStates = useRef(new Map<string, { editable: boolean; closed: boolean }>())
  const chatStates = useMemo(
    () =>
      new Map(
        data.items.map((item) => {
          const entry =
            item.kind === 'question'
              ? (data.questions.find((row) => row.key === `${item.sessionId}/${item.questionId}`)?.harness ??
                item.entry)
              : undefined
          const state = entry
            ? answerTransition(states[item.id] ?? IDLE, {
                type: 'settled',
                entry,
              })
            : (states[item.id] ?? IDLE)
          return [
            item.id,
            {
              editable: ['idle', 'failed'].includes(state.status),
              closed: ['delivered', 'already', 'withdrawn', 'not-delivered'].includes(state.status),
            },
          ]
        }),
      ),
    [data.items, data.questions, states],
  )
  liveChatStates.current = chatStates
  const updateChatHandle = useCallback((id: string, handle: DecisionChatHandle) => {
    const state = liveChatStates.current.get(id)
    if (state) {
      handle.update(state)
      return
    }
    handle.update({ editable: false, closed: true })
    if (handles.current.get(id) === handle) handles.current.delete(id)
    // Let the host render the closed state before removing the binding.
    setTimeout(() => handle.detach(), 0)
  }, [])
  useEffect(() => {
    let mounted = true
    for (const item of data.items) {
      if (item.kind !== 'question' || !chat?.rebindDecisionChat) continue
      const token = Symbol()
      chatRequests.current.set(item.id, token)
      void chat
        .rebindDecisionChat(requestFor(item))
        .then((handle) => {
          if (!handle) return
          if (!mounted || chatRequests.current.get(item.id) !== token) {
            if (active.current && !liveChatStates.current.has(item.id)) updateChatHandle(item.id, handle)
            else handle.detach()
            return
          }
          // A click may have opened this copy while the reload lookup was pending.
          handles.current.get(item.id)?.detach()
          handles.current.set(item.id, handle)
          updateChatHandle(item.id, handle)
        })
        .catch((error) => {
          if (mounted) setChatError(errorMessage(error))
        })
        .finally(() => {
          if (chatRequests.current.get(item.id) === token) chatRequests.current.delete(item.id)
        })
    }
    return () => {
      mounted = false
    }
  }, [chat, data.items, requestFor, updateChatHandle])
  useEffect(() => {
    for (const [id, handle] of handles.current) {
      updateChatHandle(id, handle)
    }
  }, [chatStates, updateChatHandle])
  const openChat = useCallback(
    async (item: QuestionNeed) => {
      if (!chat?.openDecisionChat) return
      setChatError(null)
      const token = Symbol()
      chatRequests.current.set(item.id, token)
      try {
        const handle = await chat.openDecisionChat(requestFor(item))
        if (!active.current || chatRequests.current.get(item.id) !== token) {
          handle.detach()
          return
        }
        handles.current.get(item.id)?.detach()
        handles.current.set(item.id, handle)
        if (phone) setFocused(null)
        updateChatHandle(item.id, handle)
      } catch (error) {
        if (active.current) setChatError(errorMessage(error))
      } finally {
        if (chatRequests.current.get(item.id) === token) chatRequests.current.delete(item.id)
      }
    },
    [chat, requestFor, phone, updateChatHandle],
  )
  const openedInitial = useRef<string | null>(null)
  useEffect(() => {
    if (!initialAskAboutId || openedInitial.current === initialAskAboutId) return
    const item = data.items.find(
      (item): item is QuestionNeed & { urgency: 'now' | 'no-rush' } =>
        item.id === initialAskAboutId && item.kind === 'question',
    )
    if (item && chat?.openDecisionChat) {
      openedInitial.current = initialAskAboutId
      void openChat(item)
    }
  }, [initialAskAboutId, chat, data.items, openChat])
  const updateState = (id: string, event: Parameters<typeof answerTransition>[1]) =>
    setStates((current) => ({
      ...current,
      [id]: answerTransition(current[id] ?? IDLE, event),
    }))
  const send = async (
    item: NeedsYouItem,
    summary: string,
    action: () => Promise<AskEntry | undefined>,
  ) => {
    if (inFlight.current.has(item.id)) return
    inFlight.current.add(item.id)
    updateState(item.id, { type: 'send', summary })
    retries.current.set(item.id, async () => {
      if (inFlight.current.has(item.id)) return
      if (item.kind === 'question' && iii) {
        inFlight.current.add(item.id)
        updateState(item.id, { type: 'send', summary })
        try {
          const { questions } = await iii.call<{ questions: ControlPlaneQuestion[] }>('control-plane::overview', { scope: data.scope })
          const entries = questions.flatMap((row) => (row.harness ? [row.harness] : []))
          const latest = entries.find(
            (entry) => entry.session_id === item.sessionId && entry.question_id === item.questionId,
          )
          if (!latest) throw new Error('The question is no longer available. Refresh the decision log.')
          if (latest.state !== 'open' && latest.state !== 'expired') {
            updateState(item.id, { type: 'settled', entry: latest })
            data.refresh()
            return
          }
        } catch (error) {
          updateState(item.id, { type: 'failed', error: errorMessage(error) })
          return
        } finally {
          inFlight.current.delete(item.id)
        }
      }
      await send(item, summary, action)
    })
    try {
      let result: AskEntry | undefined
      for (;;) {
        try {
          result = await action()
          break
        } catch (error) {
          const message = errorMessage(error)
          if (!isRetryableAnswerError(message)) throw error
          updateState(item.id, { type: 'failed', error: message })
          await new Promise((resolve) => setTimeout(resolve, 500))
          if (!active.current || !currentItems.current.has(item.id)) return
        }
      }
      if (item.kind === 'question' && result) {
        updateState(item.id, { type: 'settled', entry: result, own: true })
      } else updateState(item.id, { type: 'delivered' })
    } catch (error) {
      updateState(item.id, { type: 'failed', error: errorMessage(error) })
    } finally {
      inFlight.current.delete(item.id)
      data.refresh()
    }
  }
  const answer = (item: QuestionNeed, dismissed = false) => {
    if (!iii) return
    const questions = item.entry.questions
    const answers = selectedAnswers(questions, drafts[item.id] ?? questions.map(recommendedSelection))
    if (!dismissed && !answers) return
    const summary = dismissed
      ? 'Let the agent decide'
      : (answers
          ?.map((answer) => `${answer.header}: ${answer.other ?? answer.choice ?? answer.choices?.join(', ')}`)
          .join('\n') ?? '')
    void send(item, summary, () =>
      iii.call<AskEntry>('harness::ask::answer', {
        session_id: item.sessionId,
        question_id: item.questionId,
        ...(dismissed ? { dismissed: true } : { answers }),
        by: { channel: 'control-plane', user },
      }),
    )
  }
  const detail = selected ? (
    <NeedDetail
      headingRef={detailHeading}
      item={selected}
      entry={entry}
      record={record}
      state={state}
      data={data}
      selections={drafts[selected.id] ?? (entry?.questions ?? []).map(recommendedSelection)}
      onSelection={(index, selection) => {
        setFilledFromChat(null)
        setDrafts((current) => {
          const next = [...(current[selected.id] ?? (entry?.questions ?? []).map(recommendedSelection))]
          next[index] = selection
          return { ...current, [selected.id]: next }
        })
      }}
      canSend={!!iii}
      askAbout={
        chatError ? (
          <StatusPanel variant="alert" headline="Side chat could not be prepared" detail={chatError} />
        ) : selected.kind !== 'stalled' ? (
          <AskAboutButton
            item={selected}
            tokens={data.runs.find((run) => run.session.session_id === selected.sessionId)?.context_size ?? null}
            available={!!chat?.openDecisionChat}
            disabled={!iii || !['idle', 'failed', 'not-delivered'].includes(state.status)}
            onClick={() => void openChat(selected)}
          />
        ) : null
      }
      filledFromChat={filledFromChat === selected.id}
      onAnswer={(dismissed) => selected.kind === 'question' && answer(selected, dismissed)}
      onResume={() => {
        if (!iii || selected.kind !== 'stalled' || !currentItems.current.has(selected.id)) return
        void send(selected, 'continue', async () => {
          await iii.call('harness::send', {
            session_id: selected.sessionId,
            message: 'continue',
            idempotency_key: `control-plane:resume:${selected.sessionId}:${stallVersion(selected, data.runs)}`,
          })
          return undefined
        })
      }}
      onLeave={() => {
        onLeave(selected)
        setFocused(null)
      }}
      retry={() => {
        void retries.current.get(selected.id)?.()
      }}
      onBack={closeDetail}
    />
  ) : null
  if (!visible) return null
  return (
    <div ref={inbox} className="control-plane-inbox" data-phone={phone || undefined} data-detail-open={!!selected || undefined}>
      <PageSidebar label="Needs you releases" width={220} collapsible={false} className="control-plane-needs-rail">
        <List aria-label="Filter by release">
          <ListItem
            label="Everything in scope"
            trailing={String(data.items.length)}
            selected={release === 'all'}
            onClick={() => setRelease('all')}
          />
          <ListGroup>
            <ListGroupLabel>
              <Eyebrow>Releases</Eyebrow>
            </ListGroupLabel>
            {data.releaseCatalog.map((row) => (
              <ListItem
                key={row.id}
                label={row.name}
                trailing={String(data.items.filter((item) => item.card?.release_id === row.id).length)}
                selected={release === row.id}
                onClick={() => setRelease(row.id)}
              />
            ))}
          </ListGroup>
          <ListItem
            label="Not on the board"
            trailing={String(data.items.filter((item) => !item.card).length)}
            selected={release === 'unlinked'}
            onClick={() => setRelease('unlinked')}
          />
        </List>
        <div className="control-plane-sources">
          <Eyebrow>From</Eyebrow>
          {(['question', 'stalled'] as const).map((kind) => (
            <p key={kind}>
              <span>{kind === 'question' ? 'Questions' : 'Stalled runs'}</span>
              <span className="control-plane-meta">{data.items.filter((item) => item.kind === kind).length}</span>
            </p>
          ))}
        </div>
      </PageSidebar>
      <div className="control-plane-needs-list">
        <SearchField
          value={query}
          onChange={setQuery}
          aria-label="Filter Needs you"
          placeholder="Filter by card, session or text"
        />
        {data.errors.needsYou || data.errors.sessions || data.errors.board ? (
          <StatusPanel
            variant="warn"
            headline="A source is unavailable"
            detail={data.errors.needsYou ?? data.errors.sessions ?? data.errors.board}
            action={
              <Button variant="ghost" size="sm" onClick={data.refresh}>
                Retry
              </Button>
            }
          />
        ) : null}
        {data.loading.needsYou && !data.items.length ? (
          <div role="status" aria-label="Loading Needs you" className="control-plane-stack">
            <Skeleton className="control-plane-h-5 control-plane-w-28" />
            {[0, 1, 2].map((id) => (
              <Skeleton key={id} className="control-plane-h-16 control-plane-w-full" />
            ))}
          </div>
        ) : !data.items.length ? (
          <EmptyState
            title={
              data.errors.needsYou || data.errors.board || data.errors.sessions
                ? 'No items from the available sources'
                : 'Nothing needs you'
            }
            description="A question or a stalled run from any session shows up here."
            action={{ label: 'Open Runs', onClick: openRuns }}
          />
        ) : !items.length ? (
          <EmptyState title="No matching items" description="Choose another release or clear your filter." />
        ) : (
          <List aria-label="Needs you items">
            {(['now', 'no-rush'] as const).map((group) => {
              const rows = items.filter((item) => item.urgency === group)
              return rows.length ? (
                <ListGroup key={group}>
                  <ListGroupLabel>
                    <StatusDot tone={group === 'now' ? 'warn' : 'ink'} />
                    {group === 'now' ? 'Now' : 'No rush'}
                    <span className="control-plane-meta">{rows.length}</span>
                    {group === 'now' ? (
                      <span className="control-plane-meta control-plane-group-hint">most cards blocked first</span>
                    ) : null}
                  </ListGroupLabel>
                  {rows.map((item) => (
                    <ListItem
                      key={item.id}
                      data-need-id={item.id}
                      className="control-plane-need-row"
                      selected={selected?.id === item.id}
                      onClick={() => {
                        if (item.id === selectedKey) detailHeading.current?.focus()
                        setFocused(item)
                      }}
                      label={item.title}
                      description={[
                        item.kind === 'question' ? item.entry.asked_by : null,
                        item.card?.key ?? 'not on the board',
                        item.blockedCards ? `blocks ${item.blockedCards}` : null,
                      ]
                        .filter(Boolean)
                        .join(' · ')}
                      trailing={
                        <span className="control-plane-need-end">
                          <span className="control-plane-need-trailing">
                            <time className="control-plane-meta">{formatRelative(item.createdAt)}</time>
                            <span className="control-plane-status">{needLabel(item)}</span>
                          </span>
                          <ChevronRight className="control-plane-phone-chevron control-plane-size-4" />
                        </span>
                      }
                    />
                  ))}
                </ListGroup>
              ) : null
            })}
          </List>
        )}
      </div>
      {phone ? (
        <BottomSheet
          open={!!selected}
          onOpenChange={(open) => {
            if (!open) closeDetail()
          }}
        >
          <BottomSheetContent
            data-iii-ui="control-plane"
            className="control-plane control-plane-decision-sheet"
            heading={
              <span className="control-plane-meta">
                {selected
                  ? `${selected.card?.key ?? 'Not on the board'} · ${needLabel(selected)} · ${formatRelative(selected.createdAt)}`
                  : 'Needs you'}
              </span>
            }
          >
            {detail}
          </BottomSheetContent>
        </BottomSheet>
      ) : (
        <div className="control-plane-decision-layout">
          <div className="control-plane-needs-detail">
            {detail ?? (
              <EmptyState
                title="Choose an item"
                description="Open a question or a stalled run from the list."
              />
            )}
          </div>
        </div>
      )}
    </div>
  )
}

export function NeedDetail({
  headingRef,
  item,
  entry,
  record,
  data,
  selections,
  onSelection,
  state,
  canSend,
  onAnswer,
  onResume,
  onLeave,
  retry,
  onBack,
  askAbout,
  filledFromChat,
}: {
  headingRef: Ref<HTMLHeadingElement>
  item: NeedsYouItem
  entry?: AskEntry
  record?: ControlPlaneQuestion
  data: ControlPlaneData
  selections: Selection[]
  onSelection: (index: number, selection: Selection) => void
  state: AnswerState
  canSend: boolean
  onAnswer: (dismissed?: boolean) => void
  onResume: () => void
  onLeave: () => void
  retry: () => void
  onBack: () => void
  askAbout: ReactNode
  filledFromChat: boolean
}) {
  const host = useControlPlaneHost()
  const inputId = useId()
  const editable = state.status === 'idle' || state.status === 'failed'
  const run = data.runs.find((row) => row.session.session_id === item.sessionId)
  const questions = entry?.questions ?? []
  const answers = selectedAnswers(questions, selections)
  const movedOn =
    item.kind === 'question' &&
    entry?.turn_id &&
    (item.currentTurnId ?? run?.status.turn_id) &&
    (item.currentTurnId ?? run?.status.turn_id) !== entry.turn_id
  const statusTitles = {
    sending: 'Sending your answer…',
    delivered: 'Delivered',
    failed: 'Your answer did not reach the harness',
    already: `Already answered${state.channel ? ` by ${state.channel}` : ''}`,
    withdrawn: 'Withdrawn by the asker',
    'not-delivered': 'The session that asked is gone',
    idle: '',
  }
  return (
    <article className="control-plane-decision" onKeyDown={(event) => {
      if (event.key !== 'Escape' || event.defaultPrevented) return
      event.preventDefault()
      event.stopPropagation()
      onBack()
    }}>
      <div className="control-plane-decision-toolbar">
        <Button variant="ghost" size="sm" className="control-plane-detail-back" onClick={onBack}>
          <ChevronLeft className="control-plane-size-4" />
          Back
        </Button>
        {item.card ? (
          <button
            type="button"
            className="control-plane-link control-plane-meta"
            onClick={() => host.panels?.open({ pageId: 'kanban-ticket', context: { id: item.card?.key ?? '' } })}
          >
            {item.card.key}
          </button>
        ) : (
          <span className="control-plane-meta">Not on the board</span>
        )}
        <div className="control-plane-decision-links">
          {askAbout}
          {item.card ? (
            <button
              type="button"
              className="control-plane-link control-plane-header-link"
              onClick={() => host.panels?.open({ pageId: 'kanban-ticket', context: { id: item.card?.key ?? '' } })}
            >
              Open card
            </button>
          ) : null}
          <button
            type="button"
            className="control-plane-link control-plane-header-link"
            onClick={() => host.chat?.openConversation?.(item.sessionId)}
          >
            Open session
          </button>
        </div>
      </div>
      <div className="control-plane-decision-scroll">
        <Eyebrow className="control-plane-kind">{needLabel(item)}</Eyebrow>
        <h2 ref={headingRef} tabIndex={-1}>{item.title}</h2>
        <p className="control-plane-meta">
          {item.kind === 'question' ? `asked by ${entry?.asked_by ?? run?.profile ?? 'unknown profile'} · ` : ''}
          {item.kind === 'question'
            ? (item.sessionTitle ?? run?.session.title ?? item.sessionId)
            : item.sessionId}{' '}
          · {formatAge(item.createdAt)}
        </p>
        {item.kind === 'question' ? (
          <>
            {filledFromChat ? (
              <p className="control-plane-muted" role="status">
                Filled from the side chat · send it with Answer.
              </p>
            ) : null}
            {entry?.about.why ? <p>{entry.about.why}</p> : null}
            {item.blocks.length ? (
              <div className="control-plane-waiting">
                <Eyebrow>Waiting on this</Eyebrow>
                <div className="control-plane-blocks">
                  {item.blocks.map((key) => (
                    <button
                      key={key}
                      className="control-plane-link control-plane-meta"
                      type="button"
                      onClick={() => host.panels?.open({ pageId: 'kanban-ticket', context: { id: key } })}
                    >
                      {key}
                    </button>
                  ))}
                </div>
              </div>
            ) : (
              <p className="control-plane-muted">Only the session that asked waits on this.</p>
            )}
            {movedOn ? (
              <p className="control-plane-inline-note">
                <Info className="control-plane-size-4" />
                The session moved on since it asked; your answer arrives as a message.
              </p>
            ) : null}
            {entry?.state === 'expired' && editable ? (
              <StatusPanel
                variant="warn"
                headline="Expired · still open"
                detail="The asker was told nobody answered in time. Your answer still arrives as a message."
              />
            ) : null}
            {editable
              ? questions.map((question, index) => (
                  <fieldset key={`${item.id}:${question.header}`} className="control-plane-answer-field">
                    <legend>{questions.length > 1 ? question.question : 'Answer'}</legend>
                    <div className="control-plane-options">
                      {question.options.map((option) => (
                        <label
                          key={option.label}
                          className="control-plane-option"
                          data-selected={selections[index]?.picked.includes(option.label) || undefined}
                        >
                          <input
                            type={question.multi_select ? 'checkbox' : 'radio'}
                            name={`${inputId}:${index}`}
                            checked={selections[index]?.picked.includes(option.label) ?? false}
                            onChange={() => onSelection(index, selectOption(question, selections[index], option.label))}
                          />
                          <span>
                            <strong>{option.label}</strong>
                            {option.recommended ? <span className="control-plane-recommended">Recommended</span> : null}
                            {option.description ? (
                              <span className="control-plane-option-description">{option.description}</span>
                            ) : null}
                          </span>
                        </label>
                      ))}
                    </div>
                    <label className="control-plane-free-label" htmlFor={`${inputId}-${index}`}>
                      Or answer in your own words
                    </label>
                    <Input
                      id={`${inputId}-${index}`}
                      value={selections[index]?.text ?? ''}
                      onChange={(text) => onSelection(index, typeAnswer(question, text))}
                      placeholder="Type a different answer"
                    />
                  </fieldset>
                ))
              : null}
            <p className="control-plane-inline-note">
              <Clock className="control-plane-size-4" />
              {entry?.expires_at ? `Expires ${new Date(entry.expires_at).toLocaleString()}. ` : ''}
              {record && !record.harness
                ? 'Expiry policy is unavailable until the harness returns.'
                : entry && typeof entry.on_expiry === 'object'
                  ? `If nobody answers, apply: ${entry.on_expiry.answer}.`
                  : 'If nobody answers, the run waits.'}
            </p>
          </>
        ) : (
          <dl className="control-plane-stall-facts">
            <div>
              <dt>Last progress</dt>
              <dd>{formatAge(item.lastProgress)}</dd>
            </div>
            <div>
              <dt>Root session</dt>
              <dd>Done · no running children · no open questions</dd>
            </div>
            <div>
              <dt>Wakes</dt>
              <dd>{item.armedWakes?.length ?? 0} armed · none has a future deadline</dd>
            </div>
          </dl>
        )}
        {state.status !== 'idle' ? (
          <StatusPanel
            role="status"
            variant={
              state.status === 'failed' || state.status === 'not-delivered'
                ? 'alert'
                : state.status === 'delivered' || state.status === 'already'
                  ? 'success'
                  : 'info'
            }
            headline={statusTitles[state.status]}
            detail={
              <span className="control-plane-sent-summary">
                {state.summary}
                {state.error ? `\n${state.error}` : ''}
                {state.status === 'not-delivered' && item.card
                  ? '\nYour answer is kept on its card with the question.'
                  : ''}
              </span>
            }
            action={
              state.status === 'failed' ? (
                <Button variant="pill" size="sm" onClick={retry}>
                  Retry
                </Button>
              ) : state.status === 'not-delivered' && item.card ? (
                <Button
                  variant="pill"
                  size="sm"
                  onClick={() => host.panels?.open({ pageId: 'kanban-ticket', context: { id: item.card?.key ?? '' } })}
                >
                  Open card
                </Button>
              ) : undefined
            }
          />
        ) : null}
        {item.kind === 'question' ? (
          <div className="control-plane-trail">
            <Eyebrow>Trail</Eyebrow>
            <ol>
              <li>
                Asked in {item.sessionId}
                <time className="control-plane-meta">{formatAge(item.createdAt)}</time>
              </li>
              {record?.board ? (
                <li>
                  Put on {item.card?.key ?? record.ticketId} as a question
                  <time className="control-plane-meta">{formatAge(record.board.comment.created_at)}</time>
                </li>
              ) : null}
              {entry?.settled_at ? (
                <li>
                  {entry.state === 'withdrawn'
                    ? 'Withdrawn by the asker'
                    : `${entry.outcome?.replaceAll('_', ' ')} · ${entry.by?.user ?? entry.by?.channel ?? ''}`}
                  <time className="control-plane-meta">{formatAge(entry.settled_at)}</time>
                </li>
              ) : null}
            </ol>
          </div>
        ) : null}
        {item.kind !== 'stalled' ? (
          <div className="control-plane-phone-ask">
            {askAbout}
            {item.card ? (
              <Button
                variant="ghost"
                size="sm"
                onClick={() => host.panels?.open({ pageId: 'kanban-ticket', context: { id: item.card?.key ?? '' } })}
              >
                Open card {item.card.key}
              </Button>
            ) : null}
          </div>
        ) : null}
      </div>
      {editable ? (
        <div className="control-plane-decision-actions">
          {item.kind === 'question' ? (
            <>
              <Button variant="primary" disabled={!canSend || !answers} onClick={() => onAnswer()}>
                Answer
              </Button>
              {['scope', 'routine', 'run_health'].includes(item.questionClass) ? (
                <Button variant="ghost" disabled={!canSend} onClick={() => onAnswer(true)}>
                  Let the agent decide
                </Button>
              ) : null}
            </>
          ) : (
            <>
              <Button variant="primary" disabled={!canSend} onClick={onResume}>
                Resume
              </Button>
              <Button variant="ghost" onClick={onLeave}>
                Leave it
              </Button>
            </>
          )}
        </div>
      ) : null}
    </article>
  )
}

export function QuestionLog({
  data,
  view,
  iii,
  user,
  paneId = 'control-plane',
}: {
  data: ControlPlaneData
  view: 'assumptions' | 'decided'
  iii?: ControlPlaneActions
  user: string
  paneId?: string
}) {
  const host = useControlPlaneHost()
  const [kept, setKept] = usePaneState<string[]>(`control-plane:${paneId}:kept:${user}`, [])
  const [sending, setSending] = useState<string | null>(null)
  const [errors, setErrors] = useState<Record<string, string>>({})
  const [reopened, setReopened] = useState<string[]>([])
  const inFlight = useRef(new Set<string>())
  const rows = data.questions
    .filter((row) =>
      view === 'assumptions'
        ? row.recordedBy && !kept.includes(row.key)
        : !row.recordedBy && !['open', 'expired', 'answering'].includes(row.state),
    )
    .sort((a, b) => (b.settledAt ?? b.askedAt) - (a.settledAt ?? a.askedAt))
  const reopen = async (row: ControlPlaneQuestion) => {
    if (!iii || !row.ticketId || inFlight.current.has(row.key)) return
    inFlight.current.add(row.key)
    setSending(row.key)
    setErrors((current) => ({ ...current, [row.key]: '' }))
    try {
      await iii.call('kanban::comment::create', {
        ticket_id: row.ticketId,
        author: 'user',
        body: `Reopen as a question:\n${row.questions.map((question) => question.question).join('\n')}\n\nPlease revisit the recorded assumption: ${row.answerText ?? ''}`,
      })
      setReopened((current) => [...current, row.key])
      data.refresh()
    } catch (error) {
      setErrors((current) => ({ ...current, [row.key]: errorMessage(error) }))
    } finally {
      inFlight.current.delete(row.key)
      setSending(null)
    }
  }
  return (
    <div className="control-plane-question-log">
      <Card className="control-plane-section">
      <PanelHeader className="control-plane-section-title">
        <h2>{view === 'assumptions' ? 'Assumptions' : 'Decided'}</h2>
        <span className="control-plane-section-subtitle">
          {view === 'assumptions'
            ? 'Material choices the agents recorded without asking you.'
            : 'Questions that closed, with the answer and its outcome.'}
        </span>
        <span className="control-plane-spacer" />
        {view === 'decided' ? (
          <span className="control-plane-meta">
            Median decision wait ·{' '}
            {data.median_decision_wait_ms === null
              ? 'No decisions in the last 7 days'
              : `${Math.round(data.median_decision_wait_ms / 60_000)} min over the last 7 days`}
          </span>
        ) : null}
      </PanelHeader>
      {data.errors.questions ? (
        <StatusPanel
          className="control-plane-section-pad"
          variant="warn"
          headline="The decision log could not be listed"
          detail={data.errors.questions}
          action={
            <Button variant="pill" size="sm" onClick={data.refresh}>
              Retry
            </Button>
          }
        />
      ) : null}
      {data.loading.questions && !rows.length ? (
        <div role="status" aria-label="Loading questions" className="control-plane-stack control-plane-section-pad">
          <Skeleton className="control-plane-h-24 control-plane-w-full" />
          <Skeleton className="control-plane-h-24 control-plane-w-full" />
        </div>
      ) : !rows.length ? (
        <p className="control-plane-empty-line">
          {view === 'assumptions'
            ? 'No assumptions to review. Recorded choices appear here when an agent makes a material decision alone.'
            : 'No decisions yet. Closed questions appear here with their answer.'}
        </p>
      ) : (
        rows.map((row) => {
          const card = data.tickets.find((ticket) => ticket.id === row.ticketId || ticket.key === row.ticketId)
          return (
            <article key={row.key} className="control-plane-log-row">
              <div className="control-plane-row-head">
                {row.ticketId ? (
                  <button
                    type="button"
                    className="control-plane-link control-plane-meta"
                    onClick={() => host.panels?.open({ pageId: 'kanban-ticket', context: { id: row.ticketId ?? '' } })}
                  >
                    {card?.key ?? row.ticketId}
                  </button>
                ) : (
                  <span className="control-plane-meta">Not on the board</span>
                )}
                <span className="control-plane-muted">
                  {row.recordedBy ?? row.by?.user ?? row.by?.channel ?? 'unknown author'}
                  {row.by?.channel && row.by.user ? ` · ${row.by.channel}` : ''}
                </span>
                <span className="control-plane-status" data-outcome={row.outcome ?? row.state}>
                  {(row.outcome ?? row.state).replaceAll('_', ' ')}
                </span>
                <time className="control-plane-meta">{formatAge(row.settledAt ?? row.askedAt)}</time>
              </div>
              <h3>{row.questions.map((question) => question.question).join(' · ')}</h3>
              <p className="control-plane-sent-summary">
                {(row.outcome ?? row.state) === 'dismissed'
                  ? 'Left to the agent'
                  : row.answerText ?? (row.state === 'withdrawn' ? 'Withdrawn by the asker' : 'No answer recorded.')}
              </p>
              {view === 'assumptions' ? (
                <div className="control-plane-log-actions">
                  <Button variant="pill" size="sm" onClick={() => setKept((current) => [...current, row.key])}>
                    Keep
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    disabled={!iii || !row.ticketId || sending === row.key || reopened.includes(row.key)}
                    onClick={() => {
                      void reopen(row)
                    }}
                  >
                    {sending === row.key
                      ? 'Sending…'
                      : reopened.includes(row.key)
                        ? 'Comment posted'
                        : 'Reopen as a question'}
                  </Button>
                </div>
              ) : row.outcome === 'not_delivered' && row.ticketId ? (
                <Button
                  variant="pill"
                  size="sm"
                  onClick={() => host.panels?.open({ pageId: 'kanban-ticket', context: { id: row.ticketId ?? '' } })}
                >
                  Open card
                </Button>
              ) : null}
              {errors[row.key] ? (
                <StatusPanel
                  variant="alert"
                  headline="The comment was not posted"
                  detail={errors[row.key]}
                  action={
                    <Button
                      variant="pill"
                      size="sm"
                      onClick={() => {
                        void reopen(row)
                      }}
                    >
                      Retry
                    </Button>
                  }
                />
              ) : null}
            </article>
          )
        })
      )}
      </Card>
      {view === 'decided' && data.decidedCursor ? (
        <Button variant="ghost" onClick={data.loadMoreDecided}>
          More decisions
        </Button>
      ) : null}
    </div>
  )
}
