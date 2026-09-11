import {
  Button,
  EmptyState,
  type Host,
  IconButton,
  List,
  ListItem,
  PageBody,
  PageHeader,
  PageMain,
  type PageRenderProps,
  PageShell,
  PageSidebar,
  Skeleton,
  StatusPanel,
} from '@iii-dev/console-ui'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { createApi } from './api'
import { DeckEditor, DraftDialog } from './editor'
import { Plus, Presentation, RefreshCw, Sparkles } from './icons'
import {
  type ChangedEvent,
  type Deck,
  type DeckSummary,
  defaultSlide,
  describeError,
  relativeTime,
  type Slide,
  type Theme,
} from './types'

type Props = PageRenderProps & { host: Host }
type Notice = { kind: 'success' | 'error'; text: string }

const EVENTS_FN = 'iii::slides::changed'
const SAVE_DEBOUNCE_MS = 700

export function SlidesPage({ host, onRequestClose, panelSide, panelContext, commands, setDirty }: Props) {
  const api = useMemo(() => createApi(host), [host])
  const [decks, setDecks] = useState<DeckSummary[] | null>(null)
  const [themes, setThemes] = useState<Theme[]>([])
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [deck, setDeck] = useState<Deck | null>(null)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [notice, setNotice] = useState<Notice | null>(null)
  const [saving, setSaving] = useState<'idle' | 'saving' | 'saved' | 'error'>('idle')
  const [draftOpen, setDraftOpen] = useState(false)
  const [drafting, setDrafting] = useState(false)
  const [draftError, setDraftError] = useState<string | null>(null)

  const dirty = useRef(false)
  const savedRevision = useRef(0)
  const saveTimer = useRef<number | null>(null)
  const pending = useRef<Deck | null>(null)

  const notify = useCallback((kind: Notice['kind'], text: string) => {
    setNotice({ kind, text })
    window.setTimeout(
      () => setNotice((current) => (current?.text === text ? null : current)),
      kind === 'success' ? 4_000 : 8_000,
    )
  }, [])

  const refreshList = useCallback(async () => {
    try {
      const { decks: next } = await api.list()
      setDecks(next)
      setLoadError(null)
    } catch (cause) {
      setLoadError(describeError(cause))
      setDecks((current) => current ?? [])
    }
  }, [api])

  const loadDeck = useCallback(
    async (id: string) => {
      try {
        const { deck: next } = await api.get(id)
        savedRevision.current = next.revision
        dirty.current = false
        pending.current = null
        setDeck(next)
        setSaving('idle')
      } catch (cause) {
        notify('error', describeError(cause))
        setSelectedId(null)
        setDeck(null)
      }
    },
    [api, notify],
  )

  useEffect(() => {
    void refreshList()
    api
      .themes()
      .then(({ themes: list }) => setThemes(list))
      .catch((cause) => notify('error', describeError(cause)))
  }, [api, refreshList, notify])

  useEffect(() => {
    if (selectedId) void loadDeck(selectedId)
    else setDeck(null)
  }, [selectedId, loadDeck])

  useEffect(() => {
    const context = panelContext?.context as { deck_id?: string } | null | undefined
    if (context && typeof context.deck_id === 'string' && context.deck_id) setSelectedId(context.deck_id)
  }, [panelContext?.id, panelContext?.context])

  useEffect(() => {
    const offHandler = host.iii.on<ChangedEvent>(EVENTS_FN, (event) => {
      void refreshList()
      if (event.kind === 'deleted' && event.deck_id === selectedId) {
        setSelectedId(null)
        return
      }
      if (event.deck_id === selectedId && event.revision !== savedRevision.current && !dirty.current)
        void loadDeck(event.deck_id)
    })
    const offTrigger = host.iii.registerTrigger({
      type: 'slides::changed',
      function_id: `${EVENTS_FN}::${host.iii.browserId}`,
      config: {},
    })
    return () => {
      offTrigger()
      offHandler()
    }
  }, [host, refreshList, loadDeck, selectedId])

  const flush = useCallback(async () => {
    const next = pending.current
    if (!next) return
    pending.current = null
    setSaving('saving')
    try {
      const { deck: saved } = await api.saveSlides(next, next.slides)
      savedRevision.current = saved.revision
      if (!pending.current) {
        dirty.current = false
        setDirty?.(false)
        setDeck((current) =>
          current && current.id === saved.id
            ? { ...current, revision: saved.revision, updated_at_ms: saved.updated_at_ms }
            : current,
        )
      }
      setSaving('saved')
      void refreshList()
    } catch (cause) {
      setSaving('error')
      notify('error', describeError(cause))
    }
  }, [api, notify, refreshList, setDirty])

  const scheduleSave = useCallback(
    (next: Deck) => {
      pending.current = next
      dirty.current = true
      setDirty?.(next.title)
      if (saveTimer.current !== null) window.clearTimeout(saveTimer.current)
      saveTimer.current = window.setTimeout(() => void flush(), SAVE_DEBOUNCE_MS)
    },
    [flush, setDirty],
  )

  useEffect(
    () => () => {
      if (saveTimer.current !== null) window.clearTimeout(saveTimer.current)
      if (pending.current) void flush()
    },
    [flush],
  )

  const changeDeck = useCallback(
    (patch: Partial<Deck>) => {
      setDeck((current) => {
        if (!current) return current
        const next = { ...current, ...patch }
        scheduleSave(next)
        return next
      })
    },
    [scheduleSave],
  )
  const changeSlides = useCallback((slides: Slide[]) => changeDeck({ slides }), [changeDeck])

  const createDeck = useCallback(async () => {
    try {
      const { deck: created } = await api.create({
        title: 'Untitled deck',
        slides: [defaultSlide('title'), defaultSlide('content')],
      })
      await refreshList()
      setSelectedId(created.id)
    } catch (cause) {
      notify('error', describeError(cause))
    }
  }, [api, refreshList, notify])

  const deleteDeck = useCallback(async () => {
    if (!deck) return
    try {
      await api.remove(deck.id)
      setSelectedId(null)
      await refreshList()
      notify('success', `Deleted "${deck.title}"`)
    } catch (cause) {
      notify('error', describeError(cause))
    }
  }, [api, deck, refreshList, notify])

  const draft = useCallback(
    async (input: Record<string, unknown>) => {
      setDrafting(true)
      setDraftError(null)
      try {
        const { deck: created, model } = await api.outline(input)
        setDraftOpen(false)
        await refreshList()
        setSelectedId(created.id)
        notify('success', `Drafted ${created.slides.length} slides with ${model}`)
      } catch (cause) {
        setDraftError(describeError(cause))
      } finally {
        setDrafting(false)
      }
    },
    [api, refreshList, notify],
  )

  useEffect(
    () =>
      commands?.register([
        {
          id: 'new-deck',
          title: 'New deck',
          detail: 'Create an empty deck',
          shortcut: 'Mod+Enter',
          run: () => void createDeck(),
        },
        {
          id: 'draft-deck',
          title: 'Draft a deck',
          detail: 'Generate a deck from a brief',
          keywords: ['ai', 'outline', 'generate'],
          run: () => setDraftOpen(true),
        },
        { id: 'refresh', title: 'Refresh decks', run: () => void refreshList() },
      ]),
    [commands, createDeck, refreshList],
  )

  const headerActions = (
    <>
      <IconButton label="Refresh" variant="ghost" onClick={() => void refreshList()}>
        <RefreshCw />
      </IconButton>
      <Button variant="ghost" size="sm" onClick={() => setDraftOpen(true)}>
        <Sparkles /> Draft
      </Button>
      <Button variant="primary" size="sm" onClick={() => void createDeck()}>
        <Plus /> New deck
      </Button>
    </>
  )

  return (
    <PageShell>
      <PageHeader
        icon={<Presentation />}
        title="Slides"
        description={
          deck ? `${deck.slides.length} slides \u00b7 ${deck.theme}` : 'Decks you can edit, present and export'
        }
        actions={headerActions}
        onClose={onRequestClose}
      />
      <PageBody side={panelSide}>
        <PageSidebar
          label="Decks"
          side={panelSide}
          collapsible
          resizable
          defaultWidth={240}
          minWidth={180}
          maxWidth={360}
          storageKey="slides.sidebar"
        >
          {decks === null ? (
            <div className="sl-skeletons">
              <Skeleton />
              <Skeleton />
              <Skeleton />
            </div>
          ) : decks.length === 0 ? (
            <p className="sl-hint">No decks yet.</p>
          ) : (
            <List aria-label="Decks">
              {decks.map((summary, i) => (
                <ListItem
                  key={summary.id}
                  selected={summary.id === selectedId}
                  label={summary.title}
                  description={`${summary.slide_count} slides \u00b7 ${relativeTime(summary.updated_at_ms)}`}
                  data-autofocus={i === 0 && !selectedId ? '' : undefined}
                  onClick={() => setSelectedId(summary.id)}
                />
              ))}
            </List>
          )}
        </PageSidebar>
        <PageMain className="sl-main">
          {notice ? (
            <StatusPanel
              variant={notice.kind === 'error' ? 'alert' : 'success'}
              headline={notice.text}
              className="sl-notice"
            />
          ) : null}
          {loadError ? (
            <StatusPanel variant="alert" headline="Could not load decks" detail={loadError} className="sl-notice" />
          ) : null}
          {deck ? (
            <DeckEditor
              api={api}
              deck={deck}
              themes={themes}
              saving={saving}
              onChangeDeck={changeDeck}
              onChangeSlides={changeSlides}
              onDeleteDeck={() => void deleteDeck()}
              onNotice={notify}
            />
          ) : selectedId ? (
            <div className="sl-loading">
              <Skeleton />
            </div>
          ) : (
            <div className="sl-empty">
              <EmptyState
                icon={Presentation}
                title="Pick a deck or start a new one"
                description="Draft a deck from a brief, or build one slide by slide. Present it here, export it as PowerPoint, PDF or HTML."
                action={{ label: 'Draft a deck', onClick: () => setDraftOpen(true) }}
              />
            </div>
          )}
        </PageMain>
      </PageBody>
      <DraftDialog
        open={draftOpen}
        themes={themes}
        busy={drafting}
        error={draftError}
        onOpenChange={setDraftOpen}
        onDraft={(input) => void draft(input)}
      />
    </PageShell>
  )
}
