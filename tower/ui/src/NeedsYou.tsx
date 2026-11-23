import {
  BottomSheet,
  BottomSheetContent,
  Button,
  EmptyState,
  Eyebrow,
  Input,
  List,
  ListGroup,
  ListGroupLabel,
  ListItem,
  PageSidebar,
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
import { type ReactNode, useCallback, useEffect, useId, useMemo, useRef, useState } from 'react'
import { formatTokenCount } from './format'
import { useTowerHost } from './host'
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
import type { TowerOverviewData as TowerData } from './useTowerOverview'
import { type AskEntry, type NeedsYouItem, type QuestionNeed, type TowerQuestion } from '../../src/types'
import { type DecisionNeed, decisionChatRequest, selectionFromSideChat } from './side-chat'

export type TowerActions = Pick<Host['iii'], 'call'>
const IDLE: AnswerState = { status: 'idle' }
export const needLabel = (item: NeedsYouItem) =>
  item.kind === 'question'
    ? `Question · ${item.questionClass.replaceAll('_', ' ')}`
    : item.kind === 'held'
      ? 'Held call'
      : 'Stalled run'

export function AskAboutButton({
  item,
  onClick,
  disabled,
  active,
  available = false,
  tokens = null,
}: {
  item: NeedsYouItem
  onClick: () => void
  disabled?: boolean
  active?: boolean
  available?: boolean
  tokens?: number | null
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
      <MessageSquare className="tower-size-4" />
      Ask about this
      {tokens !== null ? (
        <span className="tower-meta">{formatTokenCount(tokens)}</span>
      ) : (
        <span className="tower-meta">
          <span aria-hidden>—</span>
          <span className="tower-sr-only">Context size unavailable</span>
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
  paneId = 'tower',
  initialSelectedId,
  initialAskAboutId,
  onLeave,
  openRuns,
}: {
  data: TowerData
  visible?: boolean
  narrow?: boolean
  onFocusDecision?: () => void
  iii?: TowerActions
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
    if (initialSelectedId) setFocused(data.items.find((item) => item.id === initialSelectedId) ?? null)
  }, [initialSelectedId])
  const [release, setRelease] = useState('all')
  const [query, setQuery] = useState('')
  const [drafts, setDrafts] = useState<Record<string, Selection[]>>({})
  const handles = useRef(new Map<string, DecisionChatHandle>())
  const chatRequests = useRef(new Map<string, symbol>())
  const [chatError, setChatError] = useState<string | null>(null)
  const [heldProposals, setHeldProposals] = useState<Record<string, string>>({})
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
    if (focused?.kind !== 'held' || data.loading.needsYou) return
    if (data.items.some((item) => item.id === focused.id)) return
    if (inFlight.current.has(focused.id)) return
    const status = states[focused.id]?.status ?? 'idle'
    if (status === 'idle' || status === 'failed') setFocused(null)
  }, [data.items, data.loading.needsYou, focused, states])
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
    (item: DecisionNeed, index: number, proposal: { option?: string; text?: string }) => {
      const text = proposal.option ?? proposal.text ?? ''
      if (item.kind === 'question') {
        setDrafts((current) => {
          const next = [...(current[item.id] ?? item.entry.questions.map(recommendedSelection))]
          next[index] = selectionFromSideChat(item.entry.questions[index], text)
          return { ...current, [item.id]: next }
        })
      } else setHeldProposals((current) => ({ ...current, [item.id]: text }))
      setFocused(item)
      onFocusDecision?.()
      setFilledFromChat(item.id)
      if (phone) handles.current.get(item.id)?.close()
    },
    [phone, onFocusDecision],
  )
  const requestFor = useCallback(
    (item: DecisionNeed): DecisionChatRequest =>
      decisionChatRequest(
        item,
        data.questions.find(
          (row) => row.key === `${item.sessionId}/${item.kind === 'question' ? item.questionId : item.functionCallId}`,
        ),
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
  useEffect(() => {
    let mounted = true
    for (const item of data.items) {
      if (item.kind === 'stalled' || !chat?.rebindDecisionChat) continue
      const token = Symbol()
      chatRequests.current.set(item.id, token)
      void chat
        .rebindDecisionChat(requestFor(item))
        .then((handle) => {
          if (!handle) return
          if (!mounted || chatRequests.current.get(item.id) !== token) {
            handle.detach()
            return
          }
          // A click may have opened this copy while the reload lookup was pending.
          handles.current.get(item.id)?.detach()
          handles.current.set(item.id, handle)
          handle.update(
            liveChatStates.current.get(item.id) ?? {
              editable: false,
              closed: true,
            },
          )
        })
        .catch((error) => {
          if (mounted) setChatError(errorMessage(error))
        })
    }
    return () => {
      mounted = false
    }
  }, [chat, data.items, requestFor])
  useEffect(() => {
    for (const [id, handle] of handles.current) {
      handle.update(chatStates.get(id) ?? { editable: false, closed: true })
    }
  }, [chatStates])
  const openChat = useCallback(
    async (item: DecisionNeed) => {
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
        handle.update(
          liveChatStates.current.get(item.id) ?? {
            editable: false,
            closed: true,
          },
        )
      } catch (error) {
        if (active.current) setChatError(errorMessage(error))
      }
    },
    [chat, requestFor, phone],
  )
  const openedInitial = useRef<string | null>(null)
  useEffect(() => {
    if (!initialAskAboutId || openedInitial.current === initialAskAboutId) return
    const item = data.items.find(
      (item): item is DecisionNeed & { urgency: 'now' | 'no-rush' } =>
        item.id === initialAskAboutId && item.kind !== 'stalled',
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
    action: () => Promise<AskEntry | { resolved: boolean } | undefined>,
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
          const { questions } = await iii.call<{ questions: TowerQuestion[] }>('tower::overview', { scope: data.scope })
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
      let result: AskEntry | { resolved: boolean } | undefined
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
      if (item.kind === 'question' && result && 'question_id' in result) {
        updateState(item.id, { type: 'settled', entry: result, own: true })
      } else if (result && 'resolved' in result && !result.resolved) {
        updateState(item.id, { type: 'failed', error: 'Already answered' })
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
        by: { channel: 'tower', user },
      }),
    )
  }
  const heldAction = (item: Extract<NeedsYouItem, { kind: 'held' }>, decision: 'deny' | 'allow' | 'always') => {
    if (!iii) return
    const namespace = item.namespace ? { namespace: item.namespace } : {}
    void send(
      item,
      decision === 'deny' ? 'Deny' : decision === 'always' ? 'Always allow in this session' : 'Allow once',
      async () => {
        if (decision === 'always')
          await iii.call('approval::approve-always', {
            session_id: item.sessionId,
            function_id: item.functionId,
            ...namespace,
          })
        return iii.call<{ resolved: boolean }>('approval::resolve', {
          session_id: item.sessionId,
          function_call_id: item.functionCallId,
          decision: decision === 'deny' ? 'deny' : 'allow',
          ...namespace,
        })
      },
    )
  }
  const detail = selected ? (
    <NeedDetail
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
      heldProposal={heldProposals[selected.id]}
      onAnswer={(dismissed) => selected.kind === 'question' && answer(selected, dismissed)}
      onHeld={(decision) => selected.kind === 'held' && heldAction(selected, decision)}
      onResume={() => {
        if (!iii) return
        void send(selected, 'continue', async () => {
          await iii.call('harness::send', {
            session_id: selected.sessionId,
            message: 'continue',
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
      onBack={() => setFocused(null)}
    />
  ) : null
  if (!visible) return null
  return (
    <div className="tower-inbox" data-phone={phone || undefined} data-detail-open={!!selected || undefined}>
      <PageSidebar label="Needs you releases" width={220} collapsible={false} className="tower-needs-rail">
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
        <div className="tower-sources">
          <Eyebrow>Sources</Eyebrow>
          {(['question', 'held', 'stalled'] as const).map((kind) => (
            <p key={kind}>
              <span>{kind === 'question' ? 'Questions' : kind === 'held' ? 'Held calls' : 'Stalled runs'}</span>
              <span className="tower-meta">{data.items.filter((item) => item.kind === kind).length}</span>
            </p>
          ))}
        </div>
      </PageSidebar>
      <div className="tower-needs-list">
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
          <div role="status" aria-label="Loading Needs you" className="tower-stack">
            <Skeleton className="tower-h-5 tower-w-28" />
            {[0, 1, 2].map((id) => (
              <Skeleton key={id} className="tower-h-16 tower-w-full" />
            ))}
          </div>
        ) : !data.items.length ? (
          <EmptyState
            title={
              data.errors.needsYou || data.errors.board || data.errors.sessions
                ? 'No items from the available sources'
                : 'Nothing needs you'
            }
            description="A question, a held call or a stalled run from any session shows up here."
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
                    <span className="tower-meta">{rows.length}</span>
                  </ListGroupLabel>
                  {rows.map((item) => (
                    <ListItem
                      key={item.id}
                      className="tower-need-row"
                      selected={selected?.id === item.id}
                      onClick={() => setFocused(item)}
                      label={item.title}
                      description={`${needLabel(item)} · ${item.card?.key ?? 'not on the board'}${item.blockedCards ? ` · blocks ${item.blockedCards}` : ''}`}
                      trailing={
                        <>
                          <time className="tower-meta">{formatRelative(item.createdAt)}</time>
                          <ChevronRight className="tower-phone-chevron tower-size-4" />
                        </>
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
            if (!open) setFocused(null)
          }}
        >
          <BottomSheetContent
            data-iii-ui="tower"
            className="tower tower-decision-sheet"
            heading={
              <span className="tower-meta">
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
        <div className="tower-decision-layout">
          <div className="tower-needs-detail">
            {detail ?? (
              <EmptyState
                title="Choose an item"
                description="Open a question, a held call or a stalled run from the list."
              />
            )}
          </div>
        </div>
      )}
    </div>
  )
}

export function NeedDetail({
  item,
  entry,
  record,
  data,
  selections,
  onSelection,
  state,
  canSend,
  onAnswer,
  onHeld,
  onResume,
  onLeave,
  retry,
  onBack,
  askAbout,
  filledFromChat,
  heldProposal,
}: {
  item: NeedsYouItem
  entry?: AskEntry
  record?: TowerQuestion
  data: TowerData
  selections: Selection[]
  onSelection: (index: number, selection: Selection) => void
  state: AnswerState
  canSend: boolean
  onAnswer: (dismissed?: boolean) => void
  onHeld: (decision: 'deny' | 'allow' | 'always') => void
  onResume: () => void
  onLeave: () => void
  retry: () => void
  onBack: () => void
  askAbout: ReactNode
  filledFromChat: boolean
  heldProposal?: string
}) {
  const host = useTowerHost()
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
    already:
      item.kind === 'question' ? `Already answered${state.channel ? ` by ${state.channel}` : ''}` : 'Already resolved',
    withdrawn: 'Withdrawn by the asker',
    'not-delivered': 'The session that asked is gone',
    idle: '',
  }
  return (
    <article className="tower-decision">
      <div className="tower-decision-toolbar">
        <Button variant="ghost" size="sm" className="tower-detail-back" onClick={onBack}>
          <ChevronLeft className="tower-size-4" />
          Back
        </Button>
        {item.card ? (
          <button
            type="button"
            className="tower-link tower-meta"
            onClick={() => host.panels?.open({ pageId: 'kanban-ticket', context: { id: item.card?.key ?? '' } })}
          >
            {item.card.key}
          </button>
        ) : (
          <span className="tower-meta">Not on the board</span>
        )}
        <div className="tower-decision-links">
          {askAbout}
          {item.card ? (
            <Button
              variant="ghost"
              size="sm"
              onClick={() => host.panels?.open({ pageId: 'kanban-ticket', context: { id: item.card?.key ?? '' } })}
            >
              Open card
            </Button>
          ) : null}
          <Button variant="ghost" size="sm" onClick={() => host.chat?.openConversation?.(item.sessionId)}>
            Open session
          </Button>
        </div>
      </div>
      <div className="tower-decision-scroll">
        <Eyebrow className="tower-kind">{needLabel(item)}</Eyebrow>
        <h2 className={item.kind === 'held' ? 'tower-function' : undefined}>{item.title}</h2>
        <p className="tower-meta">
          {item.kind === 'question' ? `asked by ${entry?.asked_by ?? run?.profile ?? 'unknown profile'} · ` : ''}
          {item.kind === 'held'
            ? (item.sessionTitle ?? run?.session.title ?? item.sessionId)
            : item.kind === 'question'
              ? (item.sessionTitle ?? run?.session.title ?? item.sessionId)
              : item.sessionId}{' '}
          · {formatRelative(item.createdAt)} ago
        </p>
        {item.kind === 'question' ? (
          <>
            {filledFromChat ? (
              <p className="tower-muted" role="status">
                Filled from the side chat · send it with Answer.
              </p>
            ) : null}
            {entry?.about.why ? <p>{entry.about.why}</p> : null}
            {item.blocks.length ? (
              <div className="tower-waiting">
                <Eyebrow>Waiting on this</Eyebrow>
                <div className="tower-blocks">
                  {item.blocks.map((key) => (
                    <button
                      key={key}
                      className="tower-link tower-meta"
                      type="button"
                      onClick={() => host.panels?.open({ pageId: 'kanban-ticket', context: { id: key } })}
                    >
                      {key}
                    </button>
                  ))}
                </div>
              </div>
            ) : (
              <p className="tower-muted">Only the session that asked waits on this.</p>
            )}
            {movedOn ? (
              <p className="tower-inline-note">
                <Info className="tower-size-4" />
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
                  <fieldset key={`${item.id}:${question.header}`} className="tower-answer-field">
                    <legend>{questions.length > 1 ? question.question : 'Answer'}</legend>
                    <div className="tower-options">
                      {question.options.map((option) => (
                        <label
                          key={option.label}
                          className="tower-option"
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
                            {option.recommended ? <span className="tower-recommended">Recommended</span> : null}
                            {option.description ? (
                              <span className="tower-option-description">{option.description}</span>
                            ) : null}
                          </span>
                        </label>
                      ))}
                    </div>
                    <label className="tower-free-label" htmlFor={`${inputId}-${index}`}>
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
            <p className="tower-inline-note">
              <Clock className="tower-size-4" />
              {entry?.expires_at ? `Expires ${new Date(entry.expires_at).toLocaleString()}. ` : ''}
              {record && !record.harness
                ? 'Expiry policy is unavailable until the harness returns.'
                : entry && typeof entry.on_expiry === 'object'
                  ? `If nobody answers, apply: ${entry.on_expiry.answer}.`
                  : 'If nobody answers, the run waits.'}
            </p>
          </>
        ) : item.kind === 'held' ? (
          <>
            {item.namespace ? (
              <p className="tower-meta">
                Namespace: {item.namespace}
                {item.implicitNamespace && item.namespace !== item.implicitNamespace
                  ? ` · implicit: ${item.implicitNamespace}`
                  : ''}
              </p>
            ) : null}
            {filledFromChat && heldProposal ? (
              <p className="tower-muted" role="status">
                Filled from the side chat · {heldProposal}. Choose the matching action to send it.
              </p>
            ) : null}
            <Eyebrow>Arguments · redacted</Eyebrow>
            <pre className="tower-arguments">
              {typeof item.arguments === 'string' ? item.arguments : JSON.stringify(item.arguments, null, 2)}
            </pre>
            {item.card ? (
              <p className="tower-muted">
                Card ·{' '}
                <button
                  type="button"
                  className="tower-link tower-meta"
                  onClick={() => host.panels?.open({ pageId: 'kanban-ticket', context: { id: item.card?.key ?? '' } })}
                >
                  {item.card.key}
                </button>
              </p>
            ) : null}
            <p className="tower-muted">Nothing is chosen for you. The call stays held until you pick one.</p>
          </>
        ) : (
          <dl className="tower-stall-facts">
            <div>
              <dt>Last progress</dt>
              <dd>{formatRelative(item.lastProgress)} ago</dd>
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
              <span className="tower-sent-summary">
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
          <div className="tower-trail">
            <Eyebrow>Trail</Eyebrow>
            <ol>
              <li>
                Asked in {item.sessionId}
                <time className="tower-meta">{formatRelative(item.createdAt)} ago</time>
              </li>
              {record?.board ? (
                <li>
                  Put on {item.card?.key ?? record.ticketId} as a question
                  <time className="tower-meta">{formatRelative(record.board.comment.created_at)} ago</time>
                </li>
              ) : null}
              {entry?.settled_at ? (
                <li>
                  {entry.state === 'withdrawn'
                    ? 'Withdrawn by the asker'
                    : `${entry.outcome?.replaceAll('_', ' ')} · ${entry.by?.user ?? entry.by?.channel ?? ''}`}
                  <time className="tower-meta">{formatRelative(entry.settled_at)} ago</time>
                </li>
              ) : null}
            </ol>
          </div>
        ) : null}
        {item.kind !== 'stalled' ? (
          <div className="tower-phone-ask">
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
        <div className="tower-decision-actions">
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
          ) : item.kind === 'held' ? (
            <>
              <Button variant="pill" disabled={!canSend} onClick={() => onHeld('deny')}>
                Deny
              </Button>
              <Button variant="pill" disabled={!canSend} onClick={() => onHeld('allow')}>
                Allow once
              </Button>
              <Button variant="pill" disabled={!canSend} onClick={() => onHeld('always')}>
                Always allow in this session
              </Button>
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
  paneId = 'tower',
}: {
  data: TowerData
  view: 'assumptions' | 'decided'
  iii?: TowerActions
  user: string
  paneId?: string
}) {
  const host = useTowerHost()
  const [kept, setKept] = usePaneState<string[]>(`tower:${paneId}:kept:${user}`, [])
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
  const reopen = async (row: TowerQuestion) => {
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
    <div className="tower-question-log">
      <h2>{view === 'assumptions' ? 'Assumptions' : 'Decided'}</h2>
      {view === 'decided' ? (
        <p className="tower-meta">
          Median decision wait ·{' '}
          {data.median_decision_wait_ms === null
            ? 'No decisions in the last 7 days'
            : `${Math.round(data.median_decision_wait_ms / 60_000)} min over the last 7 days`}
        </p>
      ) : null}
      <p className="tower-muted">
        {view === 'assumptions'
          ? 'Material choices the agents recorded without asking you.'
          : 'Questions that closed, with the answer and its outcome.'}
      </p>
      {data.errors.questions ? (
        <StatusPanel
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
        <div role="status" aria-label="Loading questions" className="tower-stack">
          <Skeleton className="tower-h-24 tower-w-full" />
          <Skeleton className="tower-h-24 tower-w-full" />
        </div>
      ) : !rows.length ? (
        <EmptyState
          title={view === 'assumptions' ? 'No assumptions to review' : 'No decisions yet'}
          description={
            view === 'assumptions'
              ? 'Recorded choices appear here when an agent makes a material decision alone.'
              : 'Closed questions appear here with their answer. Held calls have no history.'
          }
        />
      ) : (
        rows.map((row) => {
          const card = data.tickets.find((ticket) => ticket.id === row.ticketId || ticket.key === row.ticketId)
          return (
            <article key={row.key} className="tower-log-row">
              <div className="tower-row-head">
                {row.ticketId ? (
                  <button
                    type="button"
                    className="tower-link tower-meta"
                    onClick={() => host.panels?.open({ pageId: 'kanban-ticket', context: { id: row.ticketId ?? '' } })}
                  >
                    {card?.key ?? row.ticketId}
                  </button>
                ) : (
                  <span className="tower-meta">Not on the board</span>
                )}
                <span className="tower-muted">
                  {row.recordedBy ?? row.by?.user ?? row.by?.channel ?? 'unknown author'}
                  {row.by?.channel && row.by.user ? ` · ${row.by.channel}` : ''}
                </span>
                <time className="tower-meta">{formatRelative(row.settledAt ?? row.askedAt)} ago</time>
              </div>
              <h3>{row.questions.map((question) => question.question).join(' · ')}</h3>
              <p className="tower-sent-summary">
                {row.answerText ?? (row.state === 'withdrawn' ? 'Withdrawn by the asker' : 'No answer recorded.')}
              </p>
              <p className="tower-muted">{(row.outcome ?? row.state).replaceAll('_', ' ')}</p>
              {view === 'assumptions' ? (
                <div className="tower-log-actions">
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
      {view === 'decided' && data.decidedCursor ? (
        <Button variant="ghost" onClick={data.loadMoreDecided}>
          More decisions
        </Button>
      ) : null}
    </div>
  )
}
