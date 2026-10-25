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
  AppWindow,
  Box,
  Circle,
  File,
  MessageSquareText,
  Search,
  SquareChevronRight,
  SquareFunction,
  X,
  Zap,
} from 'lucide-react'
import {
  type CSSProperties,
  type KeyboardEvent,
  type ReactNode,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react'
import { Kbd } from '@/components/ui/Kbd'
import { KeyCombo } from '@/components/ui/KeyCombo'
import { useMediaQuery } from '@/hooks/use-media-query'
import { useTypewriter } from '@/hooks/use-typewriter'
import { shortcutPlatform } from '@/lib/keybindings/bindings'
import {
  bindingsFor,
  type KeybindingActionId,
  matchDigitIndex,
  matchesKeybinding,
} from '@/lib/keybindings/registry'
import {
  type RegisteredPaletteSource,
  searchPaletteSources,
} from '@/lib/palette/providers'
import {
  loadRecents,
  type RecentEntry,
  recentOf,
  recordRecent,
} from '@/lib/palette/recents'
import {
  type EngineSnapshot,
  filterEntries,
  groupEntries,
  KIND_LABEL,
  type MatchRange,
  matchRanges,
  type PaletteEntry,
  type PaletteKind,
  parseQuery,
  readEngine,
} from '@/lib/palette/sources'

// viewport: phone chrome — the sm and md utilities here are the console's
// phone-vs-desktop presentation (touch sizes, 16px text, sheet vs popover),
// not pane layout; see viewport-breakpoint-conformance.test.ts.

const FILTERS: Array<{ id: PaletteKind | 'all'; label: string }> = [
  { id: 'all', label: 'All' },
  { id: 'worker', label: 'Workers' },
  { id: 'function', label: 'Functions' },
  { id: 'page', label: 'Pages' },
  { id: 'chat', label: 'Chats' },
  { id: 'workspace', label: 'Workspaces' },
  { id: 'command', label: 'Commands' },
  { id: 'action', label: 'Actions' },
  { id: 'file', label: 'Files' },
]

/** The footer's key hints, in the order they read. The chords come from the
 *  registry; only the wording is local. */
const HINTS: Array<{
  ids: KeybindingActionId[]
  label: string
  digitRange?: boolean
}> = [
  // Both arrows: one direction is not a hint about moving through a list.
  { ids: ['palette.previous', 'palette.next'], label: 'Navigate' },
  { ids: ['palette.cycleFilter'], label: 'Filter' },
  { ids: ['palette.pick'], label: 'Jump', digitRange: true },
]

/** What Enter does to the lit row, said in the footer the way Raycast names
 *  its primary action. */
const KIND_ACTION: Record<PaletteKind, string> = {
  worker: 'Open worker',
  function: 'Open function',
  page: 'Go to page',
  chat: 'Open chat',
  workspace: 'Switch workspace',
  command: 'Run command',
  action: 'Run action',
  file: 'Open file',
  item: 'Open',
}

/** Flat key caps: a hairline box, no fill and no shadow, so a row's
 *  shortcut reads as a label, not a button. */
const CAP =
  'h-5 min-w-5 rounded-[5px] border border-neutral-200 bg-transparent px-1 font-inter text-[11px] font-medium text-neutral-500 shadow-none dark:border-neutral-700/80 dark:text-neutral-400'

/** The filter row's chips: a quiet label that fills when chosen. Tab
 *  cycles them from the keyboard; the pointer picks one outright. */
const CHIP =
  'inline-flex h-6 shrink-0 items-center rounded-md px-2 text-xs font-medium whitespace-nowrap transition-colors duration-[var(--motion-duration-control)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-neutral-500 dark:focus-visible:ring-neutral-400'
const CHIP_IDLE =
  'text-neutral-500 hover:bg-neutral-100 hover:text-neutral-900 dark:text-neutral-400 dark:hover:bg-white/[0.06] dark:hover:text-neutral-100'
const CHIP_ON =
  'bg-neutral-900 text-white dark:bg-neutral-100 dark:text-neutral-900'

/** One glyph per kind, for rows that bring none of their own. They echo
 *  the rest of the app: a workspace is the tab strip's window, a command
 *  is the `>` that opens command mode. Category is carried by the glyph and
 *  the group header, not by colour: the only tint in the list is a worker's
 *  status dot, where colour means something. */
const KIND_ICON = {
  worker: Box,
  function: SquareFunction,
  page: AppWindow,
  chat: MessageSquareText,
  workspace: AppWindow,
  command: SquareChevronRight,
  action: Zap,
  file: File,
  item: Circle,
} as const

/** A leading `>`, `/`, `#` or `@` becomes a chip in front of the caret, so
 *  the mode reads as a scope rather than as a stray character in the query. */
const MODE_LABEL: Record<string, string> = {
  '>': 'Commands',
  '/': 'Commands',
  '#': 'Files',
  '@': 'Chats',
}

/** The bare palette's placeholder types these in turn: what it searches,
 *  then each prefix, so the modes are learnt by watching. */
const PROMPTS = [
  'Search pages, chats and workers…',
  'Type > to run a command',
  'Type # to find a file',
  'Type @ to jump to a chat',
] as const

const SOURCE_DEBOUNCE_MS = 120
const SLOW_ANSWER_MS = 200
const EMPTY_SOURCES: readonly RegisteredPaletteSource[] = []

function placeholderFor(
  prefix: string | null,
  filter: PaletteKind | 'all',
): string {
  if (prefix === '#') return 'Search files…'
  if (prefix === '@') return 'Search chats…'
  if (prefix !== null) return 'Search commands…'
  const label = FILTERS.find((option) => option.id === filter)?.label
  return `Search ${label?.toLowerCase() ?? 'the ADE'}…`
}

function emptyText(
  searching: boolean,
  prefix: string | null,
  text: string,
  sourceCount: number,
): string {
  if (searching) return 'Searching…'
  if (prefix === '#' && sourceCount === 0)
    return 'No worker is providing files right now.'
  return text ? `No results for “${text}”` : 'Nothing here yet'
}

/** The key the quick-open digits ride on (⌃ on a Mac, Alt elsewhere, as
 *  `palette.pick` is bound). Holding it numbers the first nine rows. */
const PICK_MODIFIER = shortcutPlatform() === 'mac' ? 'Control' : 'Alt'

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
function FunctionId({
  id,
  ranges,
}: {
  id: string
  ranges: readonly MatchRange[]
}) {
  const marker = id.indexOf('::')
  if (marker <= 0)
    return (
      <span className="max-w-[70%] shrink-0 truncate font-mono text-sm">
        <Highlighted text={id} ranges={ranges} />
      </span>
    )
  const owner = id.slice(0, marker + 2)
  return (
    <span className="max-w-[70%] shrink-0 truncate font-mono text-sm">
      <span className="text-neutral-400 dark:text-neutral-500">
        <Highlighted text={owner} ranges={ranges} />
      </span>
      <span className="text-neutral-900 dark:text-neutral-100">
        <Highlighted
          text={id.slice(marker + 2)}
          ranges={ranges}
          offset={owner.length}
        />
      </span>
    </span>
  )
}

/** The query's letters in a label, in blue: you see why a row matched
 *  without reading it twice. `offset` places `text` inside a longer label
 *  the ranges were measured against. */
function Highlighted({
  text,
  ranges,
  offset = 0,
}: {
  text: string
  ranges: readonly MatchRange[]
  offset?: number
}) {
  if (ranges.length === 0) return text
  const parts: ReactNode[] = []
  let cursor = 0
  for (const [from, to] of ranges) {
    const start = Math.max(from - offset, cursor)
    const end = Math.min(to - offset, text.length)
    if (end <= start) continue
    if (start > cursor) parts.push(text.slice(cursor, start))
    parts.push(
      <mark
        key={start}
        className="bg-transparent text-blue-600 dark:text-blue-400"
      >
        {text.slice(start, end)}
      </mark>,
    )
    cursor = end
  }
  if (cursor < text.length) parts.push(text.slice(cursor))
  return parts
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
  /** Live sources (files and the like) answering the query as it is typed. */
  sources?: readonly RegisteredPaletteSource[]
  /** The active chat's working directory, handed to sources. */
  workingDir?: string | null
  /** The active chat session, handed to sources. */
  conversationId?: string | null
  /** Text to open with; `#` lands in file mode. Read once per opening. */
  initialQuery?: string
}

export function CommandPalette({
  open,
  onClose,
  localEntries,
  onOpenWorker,
  onOpenFunction,
  readInventory = readEngine,
  sources = EMPTY_SOURCES,
  workingDir = null,
  conversationId = null,
  initialQuery = '',
}: CommandPaletteProps) {
  const [query, setQuery] = useState('')
  const [filter, setFilter] = useState<PaletteKind | 'all'>('all')
  const [active, setActive] = useState(0)
  const [engineEntries, setEngineEntries] = useState<PaletteEntry[]>([])
  const [engineError, setEngineError] = useState<string | null>(null)
  const [sourceEntries, setSourceEntries] = useState<PaletteEntry[]>([])
  const [searching, setSearching] = useState(false)
  const [recents, setRecents] = useState<RecentEntry[]>([])
  const [numbering, setNumbering] = useState(false)
  const inputRef = useRef<HTMLInputElement>(null)
  const listRef = useRef<HTMLDivElement>(null)
  const viewportStyle = useKeyboardInset(open)
  const platform = shortcutPlatform()
  const mode = MODE_LABEL[query[0] ?? ''] ? query[0] : null
  const typed = mode ? query.slice(1) : query
  const prompt = useTypewriter(
    PROMPTS,
    open && query === '' && filter === 'all',
  )

  // The digits only show while their modifier is down; letting go anywhere
  // (or leaving the window with it held) puts the row tags back.
  useEffect(() => {
    if (!open) return
    const release = (event: globalThis.KeyboardEvent) => {
      if (event.key === PICK_MODIFIER) setNumbering(false)
    }
    const reset = () => setNumbering(false)
    window.addEventListener('keyup', release)
    window.addEventListener('blur', reset)
    return () => {
      window.removeEventListener('keyup', release)
      window.removeEventListener('blur', reset)
      setNumbering(false)
    }
  }, [open])

  // Reset per opening: a palette that reopens on the last query is a palette
  // you have to clear before you can use it. Closing returns focus to whatever
  // had it — the palette can be opened from anywhere, so it must not strand
  // the caret when it goes away.
  useEffect(() => {
    if (!open) return
    const opener = document.activeElement as HTMLElement | null
    setQuery(initialQuery)
    setFilter('all')
    setActive(0)
    setRecents(loadRecents())
    inputRef.current?.focus()
    return () => {
      if (opener?.isConnected) opener.focus()
    }
  }, [open, initialQuery])

  const parsed = useMemo(() => parseQuery(query, filter), [query, filter])
  // Sources answer as you type, one query at a time: a new keystroke aborts
  // the last ask, and a slow answer to an old query is dropped.
  useEffect(() => {
    if (!open) return
    const controller = new AbortController()
    const timer = window.setTimeout(() => {
      if (sources.length === 0) {
        setSourceEntries([])
        setSearching(false)
        return
      }
      // The indicator waits for a slow answer: a label that flashes for a
      // fast one makes the palette feel slower than it is.
      const slow = window.setTimeout(() => setSearching(true), SLOW_ANSWER_MS)
      controller.signal.addEventListener('abort', () =>
        window.clearTimeout(slow),
      )
      void searchPaletteSources(sources, {
        text: parsed.text,
        prefix: parsed.prefix,
        kinds: parsed.kinds,
        workingDir,
        conversationId,
        signal: controller.signal,
      })
        .then((entries) => {
          window.clearTimeout(slow)
          if (controller.signal.aborted) return
          setSourceEntries(entries)
          setSearching(false)
        })
        .catch(() => {
          window.clearTimeout(slow)
          if (!controller.signal.aborted) setSearching(false)
        })
    }, SOURCE_DEBOUNCE_MS)
    return () => {
      window.clearTimeout(timer)
      controller.abort()
      setSearching(false)
    }
  }, [open, sources, parsed, workingDir, conversationId])

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
        // The dot already says connected; words only for anything else.
        meta: [
          worker.status.startsWith('connected') ? '' : worker.status,
          worker.version ? `v${worker.version}` : '',
        ]
          .filter(Boolean)
          .join(' · '),
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
    () => [
      ...filterEntries(
        [...localEntries, ...engineEntries],
        parsed.text,
        parsed.kinds,
      ),
      // Source rows are already the source's answer to the query.
      ...sourceEntries,
    ],
    [localEntries, engineEntries, sourceEntries, parsed],
  )
  // Recents recall whatever was last chosen — a command included — even
  // though the bare palette only searches navigation.
  const recentRows = useMemo(
    () =>
      parsed.text === '' && parsed.prefix === null && filter === 'all'
        ? recentOf([...localEntries, ...engineEntries], recents)
        : [],
    [localEntries, engineEntries, recents, parsed, filter],
  )
  const groups = useMemo(
    () => groupEntries(results, recentRows),
    [results, recentRows],
  )
  const flat = useMemo(() => groups.flatMap(([, entries]) => entries), [groups])

  // Rows arrive on their own schedule (sources answer late, the engine
  // inventory lands), so the highlight follows the row, not its position:
  // Enter always runs what was lit a moment ago.
  const activeIdRef = useRef<string | null>(null)
  const select = (index: number) => {
    activeIdRef.current = flat[index]?.id ?? null
    setActive(index)
  }
  useEffect(() => {
    const at = flat.findIndex((entry) => entry.id === activeIdRef.current)
    setActive((current) =>
      at !== -1 ? at : current < flat.length ? current : 0,
    )
  }, [flat])

  if (!open) return null

  const choose = (entry: PaletteEntry | undefined) => {
    if (!entry) return
    // A source row is one answer to one query; it has no place in recents.
    if (!entry.id.startsWith('source:')) recordRecent(entry.id)
    onClose()
    entry.run()
  }

  // Arrow keys walk past the visible window, so the row has to be dragged back
  // into view. Only a keyboard move scrolls: pointing at a row already means
  // you can see it, and scrolling under the cursor moves the target.
  const move = (step: number) => {
    if (!flat.length) return
    const next = (active + step + flat.length) % flat.length
    select(next)
    listRef.current
      ?.querySelector(`[data-palette-index="${next}"]`)
      ?.scrollIntoView({ block: 'nearest' })
  }

  // The palette's own keys live in the registry too, under the `palette`
  // scope: documented in the shortcut overlay, dispatched here, because only
  // one of us is listening at a time.
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === PICK_MODIFIER && !event.repeat) {
      setNumbering(true)
    }
    if (matchesKeybinding('palette.close', event, platform)) {
      event.preventDefault()
      onClose()
      return
    }
    const picked = matchDigitIndex('palette.pick', event, platform)
    if (picked !== null) {
      event.preventDefault()
      choose(flat[picked])
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
      return
    }
    // Backspace in an empty field peels off one layer of scope: the mode
    // chip first, then the filter, so getting back to "everything" never
    // needs the mouse.
    if (
      event.key === 'Backspace' &&
      typed === '' &&
      event.target === inputRef.current
    ) {
      if (mode) {
        event.preventDefault()
        setQuery('')
        setActive(0)
      } else if (filter !== 'all') {
        event.preventDefault()
        setFilter('all')
        setActive(0)
      }
    }
  }

  let cursor = -1
  const lit = flat[active]
  // The chip in front of the caret is a typed prefix; a Tab filter shows in
  // the filter row instead, so the two never say the same thing twice.
  const scope = mode ? { glyph: mode, label: MODE_LABEL[mode] } : null
  const scoped = mode !== null || filter !== 'all'

  // Tab cycles the chips; the pointer picks one, then the caret goes back
  // to the field so the next keystroke still searches.
  const pickFilter = (id: PaletteKind | 'all') => {
    setFilter(id)
    setActive(0)
    inputRef.current?.focus()
  }

  return (
    <div
      style={viewportStyle}
      className="command-palette-scrim fixed inset-0 z-50 flex items-start justify-center bg-neutral-950/10 sm:pt-[14vh] dark:bg-black/50"
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
        className="command-palette-card relative flex h-full w-full min-w-0 flex-col overflow-hidden border-neutral-200 bg-white font-inter tracking-[-0.006em] antialiased sm:h-auto sm:max-h-[min(34rem,78vh)] sm:w-[min(42rem,92vw)] sm:rounded-xl sm:border dark:border-neutral-800 dark:bg-neutral-900"
        onKeyDown={onKeyDown}
      >
        {/* The field: a magnifier, the mode chip when a prefix is typed, the
            query, and on a phone the close button. */}
        <div className="flex shrink-0 items-center gap-2.5 border-b border-neutral-100 pr-1 pl-4 sm:pr-4 dark:border-neutral-800">
          <Search
            aria-hidden
            className="size-4 shrink-0 text-neutral-400 dark:text-neutral-500"
          />
          {scope ? (
            <span className="inline-flex h-6 shrink-0 items-center gap-1 rounded-md bg-neutral-100 px-1.5 text-xs font-medium text-neutral-700 dark:bg-white/[0.08] dark:text-neutral-200">
              {scope.glyph ? (
                <span className="text-neutral-400 dark:text-neutral-500">
                  {scope.glyph}
                </span>
              ) : null}
              {scope.label}
            </span>
          ) : null}
          <div className="relative min-w-0 flex-1">
            <input
              ref={inputRef}
              value={typed}
              onChange={(event) => {
                setQuery((mode ?? '') + event.target.value)
                setActive(0)
              }}
              aria-label="Search the ADE"
              aria-describedby="command-palette-prompt"
              // Under 16px iOS Safari zooms the page on focus, which strands a
              // fixed overlay off screen. Worker and function ids are not prose,
              // so autocorrect and the automatic leading capital both mangle them.
              className="h-14 w-full bg-transparent text-base text-neutral-900 caret-neutral-900 outline-none dark:text-neutral-100 dark:caret-neutral-100"
              inputMode="search"
              enterKeyHint="go"
              autoCapitalize="none"
              autoCorrect="off"
              autoComplete="off"
              spellCheck={false}
            />
            {/* The placeholder is drawn, not native, so it can type itself;
                screen readers get it once, through the description. */}
            {typed === '' ? (
              <span
                id="command-palette-prompt"
                className="pointer-events-none absolute inset-y-0 left-0 flex max-w-full items-center truncate text-base text-neutral-400 dark:text-neutral-500"
              >
                {scoped ? placeholderFor(mode, filter) : prompt || '\u00a0'}
              </span>
            ) : null}
          </div>
          {/* Escape, spelled where a pointer can see it; a phone has no Escape
              and gets a real close button instead. */}
          <Kbd
            aria-hidden
            className={`${CAP} hidden shrink-0 pointer-fine:inline-flex`}
          >
            esc
          </Kbd>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close search"
            className="flex size-12 shrink-0 items-center justify-center rounded-md text-neutral-500 hover:text-neutral-900 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus sm:hidden dark:hover:text-neutral-100"
          >
            <X className="size-5" aria-hidden />
          </button>
        </div>

        {/* The filter row: every kind the palette can narrow to, so the
            scopes are discoverable before Tab is pressed. A typed prefix is
            a scope of its own and takes the row away. */}
        {!mode ? (
          <div
            role="radiogroup"
            aria-label="Filter results"
            className="flex shrink-0 items-center gap-1 overflow-x-auto border-b border-neutral-100 px-3 py-2 [scrollbar-width:none] dark:border-neutral-800 [&::-webkit-scrollbar]:hidden"
          >
            {FILTERS.map((option) => {
              const on = option.id === filter
              return (
                // biome-ignore lint/a11y/useSemanticElements: a native radio cannot be a styled chip the palette's own Tab handler drives
                <button
                  key={option.id}
                  type="button"
                  role="radio"
                  aria-checked={on}
                  tabIndex={-1}
                  onClick={() => pickFilter(option.id)}
                  className={`${CHIP} ${on ? CHIP_ON : CHIP_IDLE}`}
                >
                  {option.label}
                </button>
              )
            })}
          </div>
        ) : null}

        <div
          ref={listRef}
          className="min-h-0 flex-1 overflow-y-auto overscroll-contain p-1.5"
        >
          {engineError ? (
            <p className="px-2.5 py-1.5 text-xs text-warn-strong">
              Engine inventory unavailable: {engineError}
            </p>
          ) : null}
          {flat.length === 0 ? (
            <div className="px-4 py-12 text-center">
              <p className="text-sm text-neutral-600 dark:text-neutral-300">
                {emptyText(
                  searching,
                  parsed.prefix,
                  parsed.text,
                  sources.length,
                )}
              </p>
              {!searching && !mode ? (
                <p className="mt-2 flex items-center justify-center gap-1.5 text-xs text-neutral-400 dark:text-neutral-500">
                  Try
                  <Kbd className={`${CAP} font-mono`}>&gt;</Kbd>
                  for commands or
                  <Kbd className={`${CAP} font-mono`}>#</Kbd>
                  for files
                </p>
              ) : null}
            </div>
          ) : (
            groups.map(([kind, entries]) => (
              <div key={kind} className="pb-1">
                <p className="px-2.5 pt-2.5 pb-1 text-xs font-medium text-neutral-500 dark:text-neutral-400">
                  {KIND_LABEL[kind]}
                </p>
                {entries.map((entry) => {
                  cursor += 1
                  const index = cursor
                  // A recent row keeps its own glyph; the group header
                  // already says it is recent.
                  const Icon = entry.icon ?? KIND_ICON[entry.kind]
                  const selected = index === active
                  const digit = numbering && index < 9 ? index + 1 : null
                  const titleRanges = matchRanges(
                    entry.title,
                    parsed.text,
                    entry.kind,
                  )
                  return (
                    <button
                      key={entry.id}
                      type="button"
                      tabIndex={-1}
                      data-palette-index={index}
                      data-selected={selected || undefined}
                      onMouseMove={() => {
                        if (!selected) select(index)
                      }}
                      onClick={() => choose(entry)}
                      className={`flex min-h-12 w-full scroll-my-1.5 items-center gap-3 rounded-lg px-2.5 text-left transition-colors duration-[var(--motion-duration-control)] sm:min-h-10 ${
                        selected
                          ? 'bg-neutral-100 dark:bg-white/[0.06]'
                          : 'hover:bg-neutral-50 dark:hover:bg-white/[0.03]'
                      }`}
                    >
                      <Icon
                        aria-hidden
                        className={`size-4 shrink-0 ${
                          selected
                            ? 'text-neutral-900 dark:text-neutral-100'
                            : 'text-neutral-400 dark:text-neutral-500'
                        }`}
                      />
                      <span className="flex min-w-0 flex-1 items-baseline gap-2">
                        {entry.kind === 'function' ? (
                          <FunctionId id={entry.title} ranges={titleRanges} />
                        ) : (
                          <span
                            className={`max-w-[70%] shrink-0 truncate text-base text-neutral-900 sm:text-sm dark:text-neutral-100 ${
                              entry.kind === 'worker'
                                ? 'font-mono sm:text-[13px]'
                                : ''
                            }`}
                          >
                            <Highlighted
                              text={entry.title}
                              ranges={titleRanges}
                            />
                          </span>
                        )}
                        {entry.kind === 'worker' && entry.keywords?.[0] ? (
                          <span
                            aria-hidden
                            className={`size-1.5 shrink-0 self-center rounded-full ${statusTone(entry.keywords[0])}`}
                          />
                        ) : null}
                        {entry.detail ? (
                          <span className="min-w-0 truncate text-[13px] text-neutral-400 dark:text-neutral-500">
                            <Highlighted
                              text={entry.detail}
                              ranges={matchRanges(entry.detail, parsed.text)}
                            />
                          </span>
                        ) : null}
                      </span>
                      {/* The right-hand tag is the first thing to give up when
                          the row has to fit a phone. Holding the pick key swaps it for
                          the row's number. */}
                      {digit !== null ? (
                        <Kbd
                          className={`${CAP} hidden shrink-0 text-neutral-900 sm:inline-flex dark:text-neutral-100`}
                        >
                          {digit}
                        </Kbd>
                      ) : entry.shortcut ? (
                        <KeyCombo
                          binding={entry.shortcut}
                          platform={platform}
                          className="hidden shrink-0 sm:inline-flex"
                          capClassName={CAP}
                        />
                      ) : entry.meta ? (
                        <span className="hidden max-w-[40%] shrink-0 truncate text-xs text-neutral-400 tabular-nums sm:block dark:text-neutral-500">
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
        {/* Key hints are noise on a surface with no keys to press. */}
        <div className="hidden h-10 shrink-0 items-center gap-4 border-t border-neutral-100 bg-neutral-50/80 px-4 text-xs text-neutral-500 sm:flex dark:border-neutral-800 dark:bg-white/[0.02] dark:text-neutral-400">
          {HINTS.map((hint) => (
            <span key={hint.label} className="flex items-center gap-1.5">
              <span className="flex items-center gap-0.5">
                {hint.ids.map((id) => (
                  <KeyCombo
                    key={id}
                    binding={bindingsFor(id, platform)[0]}
                    platform={platform}
                    digitRange={hint.digitRange}
                    capClassName={CAP}
                  />
                ))}
              </span>
              {hint.label}
            </span>
          ))}
          {/* Raycast's corner: what Enter does to the lit row, by name. */}
          <span className="ml-auto flex items-center gap-2 font-medium text-neutral-900 dark:text-neutral-100">
            {searching ? (
              <span className="font-normal text-neutral-400">Searching…</span>
            ) : null}
            {lit ? KIND_ACTION[lit.kind] : null}
            {lit ? (
              <KeyCombo
                binding={bindingsFor('palette.choose', platform)[0]}
                platform={platform}
                capClassName={CAP}
              />
            ) : null}
          </span>
        </div>
      </div>
    </div>
  )
}
