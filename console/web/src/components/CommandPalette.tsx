/**
 * ⌘K — one keyboard surface over the whole console.
 *
 * The console's subject is the engine and whatever is attached to it, so that
 * is what this searches: connected workers, the functions they register, every
 * page the console can show, the open chats, and the console actions that have
 * no other home. Selecting a row navigates; nothing here mutates the engine.
 *
 * The inventory is read when the palette opens, not cached at boot: a worker
 * that just disconnected should stop being searchable, and one that just
 * arrived should appear without a reload.
 *
 * Two layouts, one component. On a pointer screen it is a centred card the
 * keyboard drives. On a phone it fills the visible viewport, opens from the
 * header's search affordance, and sizes itself around the software keyboard —
 * see `useKeyboardInset`.
 */

import {
  Boxes,
  Command,
  FunctionSquare,
  LayoutGrid,
  MessageSquareText,
  Search,
  Settings,
  X,
  Zap,
} from 'lucide-react'
import {
  type CSSProperties,
  type KeyboardEvent,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react'
import { KeyCombo } from '@/components/ui/KeyCombo'
import { useMediaQuery } from '@/hooks/use-media-query'
import { shortcutPlatform } from '@/lib/keybindings/bindings'
import {
  bindingsFor,
  type KeybindingActionId,
  matchesKeybinding,
} from '@/lib/keybindings/registry'
import {
  type EngineSnapshot,
  filterEntries,
  groupEntries,
  KIND_LABEL,
  type PaletteEntry,
  type PaletteKind,
  readEngine,
} from '@/lib/palette/sources'

const FILTERS: Array<{ id: PaletteKind | 'all'; label: string }> = [
  { id: 'all', label: 'All' },
  { id: 'worker', label: 'Workers' },
  { id: 'function', label: 'Functions' },
  { id: 'page', label: 'Pages' },
  { id: 'chat', label: 'Chats' },
  { id: 'workspace', label: 'Workspaces' },
  { id: 'command', label: 'Commands' },
  { id: 'action', label: 'Actions' },
]

/** The footer's key hints, in the order they read. The chords come from the
 *  registry; only the wording is local. */
const HINTS: Array<{ ids: KeybindingActionId[]; label: string }> = [
  { ids: ['palette.choose'], label: 'Open' },
  // Both arrows: one direction is not a hint about moving through a list.
  { ids: ['palette.previous', 'palette.next'], label: 'Select' },
  { ids: ['palette.cycleFilter'], label: 'Filter' },
  { ids: ['palette.close'], label: 'Close' },
]

/** Icon plus the tint its chip carries. Colour is category, not severity —
 *  worker status gets its own dot below, where severity belongs. */
const KIND_STYLE = {
  worker: { Icon: Boxes, chip: 'bg-accent-muted text-accent' },
  function: { Icon: FunctionSquare, chip: 'bg-ok-muted text-ok' },
  page: { Icon: Zap, chip: 'bg-warn-muted text-warn' },
  chat: { Icon: MessageSquareText, chip: 'bg-surface-active text-ink-faint' },
  workspace: { Icon: LayoutGrid, chip: 'bg-surface-active text-ink-faint' },
  command: { Icon: Command, chip: 'bg-ok-muted text-ok' },
  action: { Icon: Settings, chip: 'bg-accent-muted text-accent' },
} as const

/**
 * The slice of the screen a phone keyboard leaves behind.
 *
 * iOS and Android pan the visual viewport rather than resizing the layout one,
 * so a `fixed` overlay measured in `vh` (or even `dvh`) keeps its full height
 * and puts its result rows under the keyboard. Reading `visualViewport` is the
 * only way to know what is actually on screen. Pointer layouts have no keyboard
 * to dodge, so above the `sm` breakpoint this stays null and the card keeps its
 * own sizing.
 */
function useKeyboardInset(active: boolean): CSSProperties | undefined {
  const [inset, setInset] = useState<{ top: number; height: number } | null>(
    null,
  )
  const pointer = useMediaQuery('(min-width: 640px)')

  useEffect(() => {
    if (!active || pointer || typeof window === 'undefined') {
      setInset(null)
      return
    }
    const viewport = window.visualViewport
    if (!viewport) return
    // `scroll` fires per frame while iOS pans a focused field; only a real
    // change should cost a render.
    const read = () =>
      setInset((previous) =>
        previous?.top === viewport.offsetTop &&
        previous?.height === viewport.height
          ? previous
          : { top: viewport.offsetTop, height: viewport.height },
      )
    read()
    // `resize` covers the keyboard opening and rotation; `scroll` covers the
    // pan that follows a focused field.
    viewport.addEventListener('resize', read)
    viewport.addEventListener('scroll', read)
    return () => {
      viewport.removeEventListener('resize', read)
      viewport.removeEventListener('scroll', read)
    }
  }, [active, pointer])

  if (!inset) return undefined
  return { top: inset.top, height: inset.height, bottom: 'auto' }
}

/** `worker::rest-of-id` reads better with the owner dimmed, the way the
 *  console's own function labels render it. */
function FunctionId({ id }: { id: string }) {
  const marker = id.indexOf('::')
  if (marker <= 0)
    return <span className="font-mono text-sm sm:text-[0.8rem]">{id}</span>
  return (
    <span className="font-mono text-sm sm:text-[0.8rem]">
      <span className="text-ink-ghost">{id.slice(0, marker + 2)}</span>
      <span className="text-ink">{id.slice(marker + 2)}</span>
    </span>
  )
}

/** connected is the healthy state; `available` is a built-in that has no
 *  process to connect. Anything else is worth noticing. */
function statusTone(status: string): string {
  if (status.startsWith('connected')) return 'bg-ok'
  if (status.startsWith('available')) return 'bg-accent'
  return 'bg-warn'
}

export interface CommandPaletteProps {
  open: boolean
  onClose: () => void
  /** Rows the console can build without the engine: pages, chats, actions. */
  localEntries: PaletteEntry[]
  /** Where a worker row goes when chosen, by worker name. */
  onOpenWorker: (name: string) => void
  /** Where a function row goes when chosen (its worker's row). */
  onOpenFunction: (functionId: string, worker: string) => void
  /** Seam for the gallery, which has no engine to read. Must be a stable
   *  reference: the palette re-reads whenever it changes. */
  readInventory?: () => Promise<EngineSnapshot>
}

export function CommandPalette({
  open,
  onClose,
  localEntries,
  onOpenWorker,
  onOpenFunction,
  readInventory = readEngine,
}: CommandPaletteProps) {
  const [query, setQuery] = useState('')
  const [filter, setFilter] = useState<PaletteKind | 'all'>('all')
  const [active, setActive] = useState(0)
  const [engineEntries, setEngineEntries] = useState<PaletteEntry[]>([])
  const [engineError, setEngineError] = useState<string | null>(null)
  const inputRef = useRef<HTMLInputElement>(null)
  const listRef = useRef<HTMLDivElement>(null)
  const viewportStyle = useKeyboardInset(open)
  const platform = shortcutPlatform()

  // Reset per opening: a palette that reopens on the last query is a palette
  // you have to clear before you can use it. Closing returns focus to whatever
  // had it — the palette can be opened from anywhere, so it must not strand
  // the caret when it goes away.
  useEffect(() => {
    if (!open) return
    const opener = document.activeElement as HTMLElement | null
    setQuery('')
    setFilter('all')
    setActive(0)
    inputRef.current?.focus()
    return () => {
      if (opener?.isConnected) opener.focus()
    }
  }, [open])

  useEffect(() => {
    if (!open) return
    let cancelled = false
    // Drop the previous opening's inventory first: a worker that disconnected
    // since must not stay searchable while the new read is in flight, and must
    // not survive a read that fails.
    setEngineEntries([])
    setEngineError(null)
    void readInventory().then((snapshot) => {
      if (cancelled) return
      setEngineError(snapshot.error)
      const workers: PaletteEntry[] = snapshot.workers.map((worker) => ({
        id: `worker:${worker.name}`,
        kind: 'worker',
        title: worker.name,
        detail: `${worker.functionCount} function${worker.functionCount === 1 ? '' : 's'}`,
        meta: worker.version
          ? `${worker.status} · ${worker.version}`
          : worker.status,
        keywords: [worker.status],
        run: () => onOpenWorker(worker.name),
      }))
      const functions: PaletteEntry[] = snapshot.functions.map((fn) => ({
        id: `function:${fn.id}`,
        kind: 'function',
        title: fn.id,
        detail: fn.description,
        meta: fn.worker,
        keywords: fn.worker ? [fn.worker] : undefined,
        run: () => onOpenFunction(fn.id, fn.worker),
      }))
      setEngineEntries([...workers, ...functions])
    })
    return () => {
      cancelled = true
    }
  }, [open, onOpenWorker, onOpenFunction, readInventory])

  const results = useMemo(
    () => filterEntries([...localEntries, ...engineEntries], query, filter),
    [localEntries, engineEntries, query, filter],
  )
  const groups = useMemo(() => groupEntries(results), [results])
  const flat = useMemo(() => groups.flatMap(([, entries]) => entries), [groups])

  useEffect(() => {
    setActive((current) => (current < flat.length ? current : 0))
  }, [flat.length])

  if (!open) return null

  const choose = (entry: PaletteEntry | undefined) => {
    if (!entry) return
    onClose()
    entry.run()
  }

  // Arrow keys walk past the visible window, so the row has to be dragged back
  // into view. Only a keyboard move scrolls: pointing at a row already means
  // you can see it, and scrolling under the cursor moves the target.
  const move = (step: number) => {
    if (!flat.length) return
    const next = (active + step + flat.length) % flat.length
    setActive(next)
    listRef.current
      ?.querySelector(`[data-palette-index="${next}"]`)
      ?.scrollIntoView({ block: 'nearest' })
  }

  // The palette's own keys live in the registry too, under the `palette`
  // scope: documented in the shortcut overlay, dispatched here, because only
  // one of us is listening at a time.
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (matchesKeybinding('palette.close', event, platform)) {
      event.preventDefault()
      onClose()
      return
    }
    if (matchesKeybinding('palette.next', event, platform)) {
      event.preventDefault()
      move(1)
      return
    }
    if (matchesKeybinding('palette.previous', event, platform)) {
      event.preventDefault()
      move(-1)
      return
    }
    if (matchesKeybinding('palette.cycleFilter', event, platform)) {
      event.preventDefault()
      const step = event.shiftKey ? -1 : 1
      const index = FILTERS.findIndex((option) => option.id === filter)
      const next = (index + step + FILTERS.length) % FILTERS.length
      setFilter(FILTERS[next].id)
      setActive(0)
      return
    }
    if (matchesKeybinding('palette.choose', event, platform)) {
      event.preventDefault()
      choose(flat[active])
    }
  }

  let cursor = -1

  return (
    <div
      style={viewportStyle}
      className="fixed inset-0 z-50 flex items-start justify-center bg-black/50 sm:pt-[12vh] sm:backdrop-blur-sm"
    >
      {/* The scrim is a real button so dismissing by click is reachable
          without a mouse; the dialog below owns every other key. On a phone
          the dialog covers it, which is why that layout carries its own
          close affordance. */}
      <button
        type="button"
        aria-label="Close command palette"
        tabIndex={-1}
        className="absolute inset-0 cursor-default"
        onClick={onClose}
      />
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Command palette"
        className="relative flex h-full w-full min-w-0 flex-col overflow-hidden border-edge bg-panel sm:h-auto sm:max-h-[70vh] sm:w-[min(44rem,92vw)] sm:rounded-lg sm:border sm:shadow-2xl"
        onKeyDown={onKeyDown}
      >
        <div className="flex shrink-0 items-center gap-2 border-b border-edge pr-1 pl-4 sm:gap-3 sm:pr-4">
          <Search aria-hidden className="size-4 shrink-0 text-ink-ghost" />
          <input
            ref={inputRef}
            value={query}
            onChange={(event) => {
              setQuery(event.target.value)
              setActive(0)
            }}
            placeholder="Search actions, workspaces, pages, chats, workers, functions…"
            aria-label="Search the console"
            // Under 16px iOS Safari zooms the page on focus, which strands a
            // fixed overlay off screen. Worker and function ids are not prose,
            // so autocorrect and the automatic leading capital both mangle them.
            className="w-full bg-transparent py-3.5 text-base text-ink outline-none placeholder:text-ink-ghost sm:text-sm"
            inputMode="search"
            enterKeyHint="go"
            autoCapitalize="none"
            autoCorrect="off"
            autoComplete="off"
            spellCheck={false}
          />
          <KeyCombo
            binding={bindingsFor('palette.toggle', platform)[0]}
            platform={platform}
            className="hidden shrink-0 sm:inline-flex"
          />
          <button
            type="button"
            onClick={onClose}
            aria-label="Close search"
            className="flex size-12 shrink-0 items-center justify-center rounded-sm text-ink-faint hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus sm:hidden"
          >
            <X className="size-5" aria-hidden />
          </button>
        </div>
        {/* Six filters do not fit a phone's width; scrolling them beats
            crushing them into unreadable slivers. */}
        <div className="flex shrink-0 gap-1 overflow-x-auto border-b border-edge px-3 py-2">
          {FILTERS.map((option) => (
            <button
              key={option.id}
              type="button"
              onClick={() => {
                setFilter(option.id)
                setActive(0)
              }}
              className={`min-h-11 shrink-0 rounded-full border px-3 text-sm transition-colors sm:min-h-0 sm:px-2.5 sm:py-1 sm:text-xs ${
                filter === option.id
                  ? 'border-edge bg-surface-selected text-ink'
                  : 'border-transparent text-ink-faint hover:bg-surface-hover hover:text-ink'
              }`}
            >
              {option.label}
            </button>
          ))}
        </div>
        <div
          ref={listRef}
          className="min-h-0 flex-1 overflow-y-auto overscroll-contain py-1"
        >
          {engineError ? (
            <p className="px-4 py-2 text-sm text-warn sm:text-xs">
              Engine inventory unavailable: {engineError}
            </p>
          ) : null}
          {flat.length === 0 ? (
            <p className="px-4 py-6 text-center text-base text-ink-faint sm:text-sm">
              No matches
            </p>
          ) : (
            groups.map(([kind, entries]) => (
              <div key={kind}>
                <p className="flex items-center gap-2 px-4 pt-3 pb-1 text-xs font-semibold text-ink-ghost sm:text-[0.68rem]">
                  <span>{KIND_LABEL[kind]}</span>
                  <span className="rounded-full bg-surface-active px-1.5 py-px text-[0.62rem] font-normal tracking-normal tabular-nums">
                    {entries.length}
                  </span>
                  <span className="h-px flex-1 bg-edge" />
                </p>
                {entries.map((entry) => {
                  cursor += 1
                  const index = cursor
                  const { Icon: KindIcon, chip } = KIND_STYLE[entry.kind]
                  const Icon = entry.icon ?? KindIcon
                  const selected = index === active
                  return (
                    <button
                      key={entry.id}
                      type="button"
                      data-palette-index={index}
                      onMouseEnter={() => setActive(index)}
                      onClick={() => choose(entry)}
                      className={`flex min-h-12 w-full items-center gap-3 border-l-2 py-2.5 pr-4 pl-3.5 text-left transition-colors sm:min-h-0 sm:py-2 ${
                        selected
                          ? 'border-edge bg-surface-selected'
                          : 'border-transparent hover:bg-surface-hover'
                      }`}
                    >
                      <span
                        className={`flex size-7 shrink-0 items-center justify-center rounded ${chip}`}
                      >
                        <Icon aria-hidden className="size-4" />
                      </span>
                      <span className="min-w-0 flex-1">
                        <span className="flex items-center gap-2 truncate">
                          {entry.kind === 'function' ? (
                            <FunctionId id={entry.title} />
                          ) : (
                            <span
                              className={`truncate text-base text-ink sm:text-sm ${
                                entry.kind === 'worker'
                                  ? 'font-mono sm:text-[0.82rem]'
                                  : ''
                              }`}
                            >
                              {entry.title}
                            </span>
                          )}
                          {entry.kind === 'worker' && entry.keywords?.[0] ? (
                            <span
                              aria-hidden
                              className={`size-1.5 shrink-0 rounded-full ${statusTone(entry.keywords[0])}`}
                            />
                          ) : null}
                        </span>
                        {entry.detail ? (
                          <span className="block truncate text-sm text-ink-faint sm:text-xs">
                            {entry.detail}
                          </span>
                        ) : null}
                      </span>
                      {/* The right-hand tag is the first thing to give up when
                          the row has to fit a phone. */}
                      {entry.shortcut ? (
                        <KeyCombo
                          binding={entry.shortcut}
                          platform={platform}
                          className="hidden shrink-0 sm:inline-flex"
                        />
                      ) : entry.meta ? (
                        <span className="hidden shrink-0 rounded border border-edge px-1.5 py-0.5 font-mono text-[0.65rem] text-ink-ghost sm:block">
                          {entry.meta}
                        </span>
                      ) : null}
                    </button>
                  )
                })}
              </div>
            ))
          )}
        </div>
        <div className="flex shrink-0 items-center gap-3 border-t border-edge px-4 py-2 pb-[max(0.5rem,env(safe-area-inset-bottom))] text-[0.68rem] text-ink-ghost sm:pb-2">
          {/* Key hints are noise on a surface with no keys to press. */}
          <div className="hidden items-center gap-3 sm:flex">
            {HINTS.map((hint) => (
              <span key={hint.label} className="flex items-center gap-1.5">
                {hint.ids.map((id) => (
                  <KeyCombo
                    key={id}
                    binding={bindingsFor(id, platform)[0]}
                    platform={platform}
                  />
                ))}
                {hint.label}
              </span>
            ))}
          </div>
          <span className="ml-auto tabular-nums">
            {flat.length} result{flat.length === 1 ? '' : 's'}
          </span>
        </div>
      </div>
    </div>
  )
}
