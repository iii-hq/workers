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
 */

import {
  Boxes,
  CornerDownLeft,
  FunctionSquare,
  MessageSquareText,
  Search,
  Settings,
  Zap,
} from 'lucide-react'
import { type KeyboardEvent, useEffect, useMemo, useRef, useState } from 'react'
import {
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
  { id: 'action', label: 'Actions' },
]

/** Icon plus the tint its chip carries. Colour is category, not severity —
 *  worker status gets its own dot below, where severity belongs. */
const KIND_STYLE = {
  worker: { Icon: Boxes, chip: 'bg-accent-muted text-accent' },
  function: { Icon: FunctionSquare, chip: 'bg-ok-muted text-ok' },
  page: { Icon: Zap, chip: 'bg-warn-muted text-warn' },
  chat: { Icon: MessageSquareText, chip: 'bg-surface-active text-ink-faint' },
  action: { Icon: Settings, chip: 'bg-accent-muted text-accent' },
} as const

/** `worker::rest-of-id` reads better with the owner dimmed, the way the
 *  console's own function labels render it. */
function FunctionId({ id }: { id: string }) {
  const marker = id.indexOf('::')
  if (marker <= 0) return <span className="font-mono text-[0.8rem]">{id}</span>
  return (
    <span className="font-mono text-[0.8rem]">
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
  /** Where a worker row goes when chosen (the workers screen). */
  onOpenWorkers: () => void
  /** Where a function row goes when chosen (its worker's row). */
  onOpenFunction: (functionId: string, worker: string) => void
}

export function CommandPalette({
  open,
  onClose,
  localEntries,
  onOpenWorkers,
  onOpenFunction,
}: CommandPaletteProps) {
  const [query, setQuery] = useState('')
  const [filter, setFilter] = useState<PaletteKind | 'all'>('all')
  const [active, setActive] = useState(0)
  const [engineEntries, setEngineEntries] = useState<PaletteEntry[]>([])
  const [engineError, setEngineError] = useState<string | null>(null)
  const inputRef = useRef<HTMLInputElement>(null)

  // Reset per opening: a palette that reopens on the last query is a palette
  // you have to clear before you can use it.
  useEffect(() => {
    if (!open) return
    setQuery('')
    setFilter('all')
    setActive(0)
    inputRef.current?.focus()
  }, [open])

  useEffect(() => {
    if (!open) return
    let cancelled = false
    void readEngine().then((snapshot) => {
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
        run: onOpenWorkers,
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
  }, [open, onOpenWorkers, onOpenFunction])

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

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'Escape') {
      event.preventDefault()
      onClose()
      return
    }
    if (event.key === 'ArrowDown' || (event.key === 'n' && event.ctrlKey)) {
      event.preventDefault()
      setActive((current) => (flat.length ? (current + 1) % flat.length : 0))
      return
    }
    if (event.key === 'ArrowUp' || (event.key === 'p' && event.ctrlKey)) {
      event.preventDefault()
      setActive((current) =>
        flat.length ? (current - 1 + flat.length) % flat.length : 0,
      )
      return
    }
    if (event.key === 'Tab') {
      event.preventDefault()
      const step = event.shiftKey ? -1 : 1
      const index = FILTERS.findIndex((option) => option.id === filter)
      const next = (index + step + FILTERS.length) % FILTERS.length
      setFilter(FILTERS[next].id)
      setActive(0)
      return
    }
    if (event.key === 'Enter') {
      event.preventDefault()
      choose(flat[active])
    }
  }

  let cursor = -1

  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center bg-black/50 pt-[12vh] backdrop-blur-sm"
    >
      {/* The scrim is a real button so dismissing by click is reachable
          without a mouse; the dialog below owns every other key. */}
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
        className="flex max-h-[70vh] w-[min(44rem,92vw)] flex-col overflow-hidden rounded-lg border border-accent-border bg-panel shadow-2xl"
        onKeyDown={onKeyDown}
      >
        <div className="flex items-center gap-3 border-b border-edge px-4">
          <Search aria-hidden className="size-4 shrink-0 text-ink-ghost" />
          <input
            ref={inputRef}
            value={query}
            onChange={(event) => {
              setQuery(event.target.value)
              setActive(0)
            }}
            placeholder="Search workers, functions, pages, chats, actions…"
            aria-label="Search the console"
            className="w-full bg-transparent py-3.5 text-sm text-ink outline-none placeholder:text-ink-ghost"
          />
          <kbd className="shrink-0 rounded border border-edge px-1.5 py-0.5 font-mono text-[0.65rem] text-ink-ghost">
            ⌘K
          </kbd>
        </div>
        <div className="flex gap-1 border-b border-edge px-3 py-2">
          {FILTERS.map((option) => (
            <button
              key={option.id}
              type="button"
              onClick={() => {
                setFilter(option.id)
                setActive(0)
              }}
              className={`rounded-full border px-2.5 py-1 text-xs transition-colors ${
                filter === option.id
                  ? 'border-accent-border bg-accent-muted text-accent'
                  : 'border-transparent text-ink-faint hover:bg-surface-active hover:text-ink'
              }`}
            >
              {option.label}
            </button>
          ))}
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto py-1">
          {engineError ? (
            <p className="px-4 py-2 text-xs text-warn">
              engine inventory unavailable: {engineError}
            </p>
          ) : null}
          {flat.length === 0 ? (
            <p className="px-4 py-6 text-center text-sm text-ink-faint">
              no matches
            </p>
          ) : (
            groups.map(([kind, entries]) => (
              <div key={kind}>
                <p className="flex items-center gap-2 px-4 pt-3 pb-1 text-[0.68rem] uppercase tracking-wider text-ink-ghost">
                  <span>{KIND_LABEL[kind]}</span>
                  <span className="rounded-full bg-surface-active px-1.5 py-px font-mono text-[0.62rem] normal-case tracking-normal">
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
                      onMouseEnter={() => setActive(index)}
                      onClick={() => choose(entry)}
                      className={`flex w-full items-center gap-3 border-l-2 py-2 pr-4 pl-3.5 text-left transition-colors ${
                        selected
                          ? 'border-accent bg-accent-muted'
                          : 'border-transparent hover:bg-surface-active'
                      }`}
                    >
                      <span
                        className={`flex size-7 shrink-0 items-center justify-center rounded ${chip}`}
                      >
                        <Icon aria-hidden className="size-3.5" />
                      </span>
                      <span className="min-w-0 flex-1">
                        <span className="flex items-center gap-2 truncate">
                          {entry.kind === 'function' ? (
                            <FunctionId id={entry.title} />
                          ) : (
                            <span
                              className={`truncate text-sm ${
                                entry.kind === 'worker'
                                  ? 'font-mono text-[0.82rem]'
                                  : ''
                              } ${selected ? 'text-ink' : 'text-ink'}`}
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
                          <span className="block truncate text-xs text-ink-faint">
                            {entry.detail}
                          </span>
                        ) : null}
                      </span>
                      {entry.meta ? (
                        <span className="shrink-0 rounded border border-edge px-1.5 py-0.5 font-mono text-[0.65rem] text-ink-ghost">
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
        <div className="flex items-center gap-3 border-t border-edge px-4 py-2 text-[0.68rem] text-ink-ghost">
          <span className="flex items-center gap-1.5">
            <kbd className="rounded border border-edge px-1 py-px font-mono">
              <CornerDownLeft aria-hidden className="size-2.5" />
            </kbd>
            open
          </span>
          <span className="flex items-center gap-1.5">
            <kbd className="rounded border border-edge px-1 py-px font-mono">
              ↑↓
            </kbd>
            select
          </span>
          <span className="flex items-center gap-1.5">
            <kbd className="rounded border border-edge px-1 py-px font-mono">
              tab
            </kbd>
            filter
          </span>
          <span className="flex items-center gap-1.5">
            <kbd className="rounded border border-edge px-1 py-px font-mono">
              esc
            </kbd>
            close
          </span>
          <span className="ml-auto font-mono">
            {flat.length} result{flat.length === 1 ? '' : 's'}
          </span>
        </div>
      </div>
    </div>
  )
}
