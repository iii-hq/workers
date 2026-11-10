/**
 * The chrome both catalogue pages share. The bar is Linear's list density
 * and Vercel's document surfaces:
 *
 * - the list is single-line 32px rows; an id reads as `ns::` (faint) +
 *   the rest (ink), with a live dot and a quiet trailing meta — the
 *   description lives in the row's tooltip and the document, not in a
 *   truncated second line repeated 400 times;
 * - nested rows (an observer under its function) hang off an indent guide;
 * - the document is a centred column: ringed tile, title, description, a
 *   stat strip (label over value), line tabs, then outlined cards.
 *
 * Page chrome comes from `@iii-dev/console-ui`; everything else is a scoped
 * class in ../../styles.css.
 */

import {
  Button,
  IconButton,
  PageBody,
  PageMain,
  PageShell,
  PageSidebar,
  ShadcnButton,
  ShadcnDropdownMenu,
  ShadcnDropdownMenuCheckboxItem,
  ShadcnDropdownMenuContent,
  ShadcnDropdownMenuGroup,
  ShadcnDropdownMenuItem,
  ShadcnDropdownMenuLabel,
  ShadcnDropdownMenuSeparator,
  ShadcnDropdownMenuTrigger,
  Skeleton,
  StatusDot,
  StatusPanel,
  Switch,
} from '@iii-dev/console-ui'
import { useCopyFlash } from '@iii-dev/console-ui/hooks'
import {
  ArrowLeft,
  Check,
  ChevronDown,
  ChevronRight,
  Clock,
  Copy,
  Database,
  Ellipsis,
  FileText,
  Globe,
  Layers,
  Rows3,
  SquareTerminal,
  Waves,
  Webhook,
  Zap,
} from 'lucide-react'
import {
  type FocusEvent,
  type KeyboardEvent,
  type ReactNode,
  useCallback,
  useEffect,
  useId,
  useRef,
  useState,
} from 'react'
import type { Family, Tone } from './observer-kinds'

/**
 * Open/closed state for collapsible groups, stored as the set of ids whose
 * state is FLIPPED from the default, so a group that appears later follows
 * the default instead of arriving collapsed.
 */
export function useGroupToggle(defaultOpen: (id: string) => boolean) {
  const [flipped, setFlipped] = useState<ReadonlySet<string>>(new Set())
  const toggle = useCallback((id: string) => {
    setFlipped((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }, [])
  const isOpen = (id: string) => (flipped.has(id) ? !defaultOpen(id) : defaultOpen(id))
  return { isOpen, toggle }
}

const LIST_STOPS = '.console-catalog-row, .console-catalog-group'

/**
 * The list as one tab stop (roving tabindex): Tab enters on the selected
 * row and leaves again, instead of walking 400 rows. ↑ ↓ Home End move
 * between rows; → opens a folded group or function, ← folds it.
 */
function useListKeys() {
  const ref = useRef<HTMLDivElement>(null)
  // Runs after every render: rows come and go with the live catalogue.
  useEffect(() => {
    const root = ref.current
    if (!root) return
    const stops = [...root.querySelectorAll<HTMLElement>(LIST_STOPS)]
    const current =
      stops.find((stop) => stop === document.activeElement) ??
      stops.find((stop) => stop.getAttribute('aria-current') === 'true') ??
      stops.find((stop) => stop.tabIndex === 0) ??
      stops[0]
    for (const stop of stops) {
      const next = stop === current ? 0 : -1
      if (stop.tabIndex !== next) stop.tabIndex = next
    }
  })
  const onFocus = (event: FocusEvent<HTMLDivElement>) => {
    const target = event.target as HTMLElement
    if (!target.matches(LIST_STOPS)) return
    for (const stop of event.currentTarget.querySelectorAll<HTMLElement>(LIST_STOPS)) {
      stop.tabIndex = stop === target ? 0 : -1
    }
  }
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const target = event.target as HTMLElement
    if (!target.matches(LIST_STOPS)) return
    if (event.key === 'ArrowRight' || event.key === 'ArrowLeft') {
      // A group head folds itself; a function row folds through its twist.
      const toggle = target.matches('.console-catalog-group')
        ? target
        : target.parentElement?.querySelector<HTMLElement>('.console-catalog-twist')
      if (!toggle) return
      const open = toggle.getAttribute('aria-expanded') === 'true'
      if (open !== (event.key === 'ArrowRight')) toggle.click()
      event.preventDefault()
      return
    }
    const stops = [...event.currentTarget.querySelectorAll<HTMLElement>(LIST_STOPS)]
    const at = stops.indexOf(target)
    const next =
      event.key === 'ArrowDown'
        ? stops[at + 1]
        : event.key === 'ArrowUp'
          ? stops[at - 1]
          : event.key === 'Home'
            ? stops[0]
            : event.key === 'End'
              ? stops.at(-1)
              : undefined
    if (!next) return
    event.preventDefault()
    next.focus()
  }
  return { ref, onFocus, onKeyDown }
}

/**
 * The catalogue page frame: header, then sidebar | main. `hasSelection`
 * rides on PageBody so narrow panes drill in (list OR document).
 */
export function CatalogShell({
  header,
  search,
  filters,
  list,
  sideFooter,
  main,
  side,
  label,
  hasSelection,
}: {
  header: ReactNode
  /** The sidebar's top row, beside the collapse toggle. */
  search: ReactNode
  /** A quiet row under the search: a switch, a filter menu. */
  filters?: ReactNode
  list: ReactNode
  sideFooter?: ReactNode
  main: ReactNode
  side?: 'left' | 'right'
  /** Accessible name of the navigation column. */
  label: string
  hasSelection: boolean
}) {
  const listKeys = useListKeys()
  return (
    <PageShell className="console-catalog">
      {header}
      <PageBody side={side} className="console-catalog-body" data-selected={hasSelection}>
        <PageSidebar
          label={label}
          header={<div className="console-catalog-side-head">{search}</div>}
          side={side}
          defaultWidth={300}
          collapsible
          storageKey="console:catalog"
          narrowBelow={700}
          className="console-catalog-side"
        >
          {filters ? <div className="console-catalog-side-filters">{filters}</div> : null}
          <div className="console-catalog-side-scroll" {...listKeys}>
            {list}
          </div>
          {sideFooter ? <div className="console-catalog-side-footer">{sideFooter}</div> : null}
        </PageSidebar>
        <PageMain className="console-catalog-main">{main}</PageMain>
      </PageBody>
    </PageShell>
  )
}

/** The quiet count line at the foot of the list. */
export function SideCount({ children }: { children: ReactNode }) {
  return (
    <div className="console-catalog-count" aria-live="polite">
      {children}
    </div>
  )
}

/** A compact labelled switch: `Internal`. */
export function SwitchRow({
  label,
  checked,
  onChange,
}: {
  label: string
  checked: boolean
  onChange: (next: boolean) => void
}) {
  const id = useId()
  return (
    <div className="console-catalog-switch">
      <label htmlFor={id}>{label}</label>
      <Switch id={id} checked={checked} onChange={(event) => onChange(event.target.checked)} />
    </div>
  )
}

/**
 * One trigger, one menu: `Family · All ▾`. Single choice, drawn as
 * checkboxes (the console never draws radio circles); families with no
 * entries are not offered.
 */
export function FamilyFilter<T extends string>({
  counts,
  selected,
  onSelect,
  nameOf,
}: {
  counts: ReadonlyMap<T, number>
  selected: T | null
  onSelect: (next: T | null) => void
  nameOf: (key: T) => string
}) {
  const entries = [...counts.entries()]
    .filter(([, n]) => n > 0)
    .sort((a, b) => b[1] - a[1] || nameOf(a[0]).localeCompare(nameOf(b[0])))
  if (entries.length < 2) return null
  const total = entries.reduce((n, [, count]) => n + count, 0)
  return (
    <ShadcnDropdownMenu>
      <ShadcnDropdownMenuTrigger
        render={
          <button type="button" className="console-catalog-filter" data-active={selected !== null || undefined}>
            <span className="k">Family</span>
            <span className="v">{selected ? nameOf(selected) : 'All'}</span>
            <ChevronDown aria-hidden />
          </button>
        }
      />
      <ShadcnDropdownMenuContent align="start" sideOffset={6} className="console-catalog-menu">
        <ShadcnDropdownMenuGroup>
          <ShadcnDropdownMenuLabel>Choose one</ShadcnDropdownMenuLabel>
          <ShadcnDropdownMenuCheckboxItem checked={selected === null} onCheckedChange={() => onSelect(null)}>
            All
            <span className="menu-count">{total}</span>
          </ShadcnDropdownMenuCheckboxItem>
          {entries.map(([key, count]) => (
            <ShadcnDropdownMenuCheckboxItem
              key={key}
              checked={selected === key}
              onCheckedChange={() => onSelect(selected === key ? null : key)}
            >
              {nameOf(key)}
              <span className="menu-count">{count}</span>
            </ShadcnDropdownMenuCheckboxItem>
          ))}
        </ShadcnDropdownMenuGroup>
      </ShadcnDropdownMenuContent>
    </ShadcnDropdownMenu>
  )
}

/** "This page is live": every catalogue page follows engine signals. */
export function LiveDot() {
  return (
    <span className="console-catalog-live" title="Updates live from the engine" role="status" aria-label="Updates live">
      <span className="pip" aria-hidden />
      Live
    </span>
  )
}

/**
 * Icon-only copy. Both glyphs stay mounted and cross-fade (scale, blur,
 * opacity), so the swap reads as one control changing state.
 */
export function CopyIconButton({ value, label }: { value: string; label: string }) {
  const { state, copy } = useCopyFlash(value, 2000)
  const copied = state === 'copied'
  return (
    <IconButton
      type="button"
      label={copied ? 'Copied' : label}
      onClick={copy}
      className="console-catalog-copy"
      data-copied={copied}
    >
      <span className="console-catalog-swap" aria-hidden>
        <Copy className="from" />
        <Check className="to" />
      </span>
    </IconButton>
  )
}

/** Copy with a label; says `Copied` for two seconds once it lands. */
export function CopyButton({ value, label = 'Copy', title }: { value: string; label?: string; title?: string }) {
  const { state, copy } = useCopyFlash(value, 2000)
  return (
    <Button variant="secondary" size="sm" type="button" title={title} onClick={copy} className="console-catalog-btn">
      {state === 'copied' ? <Check aria-hidden /> : <Copy aria-hidden />}
      {state === 'copied' ? 'Copied' : label}
    </Button>
  )
}

/** `⌘↵` / `Ctrl ↵` as quiet text inside a button: no keycaps. */
export function ShortcutHint({ keys }: { keys: 'enter' }) {
  const mac = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent)
  const text = keys === 'enter' ? (mac ? '⌘↵' : 'Ctrl ↵') : ''
  return (
    <span className="console-catalog-shortcut" aria-hidden>
      {text}
    </span>
  )
}

/**
 * A code window: a slim title bar (language glyph + name, copy trailing),
 * then the code in a well. The code never wraps mid-token; a long line
 * scrolls sideways instead.
 */
export function CodeBlock({
  label,
  icon,
  children,
  copy,
  copyLabel,
}: {
  /** The language, as a terminal or editor tab names it: `bash`, `json`. */
  label: string
  icon?: ReactNode
  children: ReactNode
  copy: string
  copyLabel: string
}) {
  return (
    <figure className="console-catalog-snippet">
      <figcaption>
        <span className="snippet-lang">
          {icon}
          {label}
        </span>
        <CopyIconButton value={copy} label={copyLabel} />
      </figcaption>
      <div className="snippet-body">{children}</div>
    </figure>
  )
}

/** Past this many arguments, or this many characters (what the 360px
    Call it column shows at 13px without scrolling), one goes per line. */
const INLINE_ARGS = 2
const INLINE_CHARS = 38

/**
 * An `iii call` as a bash prompt, coloured the way a shell theme reads it:
 * the prompt, the program, the subcommand, the function, then each
 * `key=value`. Short calls stay on one line; long ones put an argument per
 * line with `\` continuations. The `$` is decoration — never selected,
 * never copied — and the copy is always the one-line command.
 */
export function TerminalSnippet({
  functionId,
  args,
  copy,
}: {
  functionId: string
  args: readonly { key: string; value: string; kind: string }[]
  copy: string
}) {
  const multiline = args.length > INLINE_ARGS || copy.length > INLINE_CHARS
  const arg = (a: { key: string; value: string; kind: string }) => (
    <>
      <span className="sh-key">{a.key}</span>
      <span className="sh-op">=</span>
      <span className="sh-value" data-kind={a.kind}>
        {a.value}
      </span>
    </>
  )
  return (
    <CodeBlock label="bash" icon={<SquareTerminal aria-hidden />} copy={copy} copyLabel="Copy command">
      <pre className="snippet-code" data-lang="bash">
        <code>
          <span className="line">
            <span className="sh-prompt" aria-hidden>
              $
            </span>
            <span className="sh-cmd">iii</span> <span className="sh-sub">call</span>{' '}
            <span className="sh-fn">{functionId}</span>
            {multiline
              ? args.length > 0
                ? <span className="sh-cont"> \</span>
                : null
              : args.map((a) => (
                  <span key={a.key}>
                    {' '}
                    {arg(a)}
                  </span>
                ))}
          </span>
          {multiline
            ? args.map((a, i) => (
                <span className="line sh-indent" key={a.key}>
                  {arg(a)}
                  {i < args.length - 1 ? <span className="sh-cont"> \</span> : null}
                </span>
              ))
            : null}
        </code>
      </pre>
    </CodeBlock>
  )
}

/** `ade::compose::container` → faint `ade::` + ink `compose::container`. */
export function IdText({ id }: { id: string }) {
  const cut = id.indexOf('::')
  if (cut <= 0) return <span className="id-rest">{id}</span>
  return (
    <>
      <span className="id-ns">{id.slice(0, cut + 2)}</span>
      <span className="id-rest">{id.slice(cut + 2)}</span>
    </>
  )
}

/**
 * A quiet group heading: chevron, name, a fine meta word, the count
 * trailing. Sticky, so the worker stays named while you scroll.
 */
export function GroupHeader({
  label,
  meta,
  count,
  open,
  onToggle,
}: {
  label: string
  meta?: string
  count?: number
  open: boolean
  onToggle: () => void
}) {
  return (
    <button type="button" className="console-catalog-group" aria-expanded={open} onClick={onToggle}>
      <ChevronRight className="chevron" aria-hidden />
      <span className="name" title={label}>
        {label}
      </span>
      {meta ? <span className="meta">{meta}</span> : null}
      {count !== undefined ? <span className="count">{count}</span> : null}
    </button>
  )
}

/** The dot on a row: an observer's state, or a function that just ran. */
export type RowDot = 'live' | 'idle' | 'ok' | 'warn' | 'alert' | 'off'

export function CatalogRow({
  icon,
  primary,
  tooltip,
  meta,
  dot,
  dotLabel,
  selected,
  onClick,
  arrived,
  nested,
}: {
  /** A bare 16px glyph (families); functions carry none. */
  icon?: ReactNode
  primary: ReactNode
  /** Native tooltip: the full id and its description. */
  tooltip?: string
  /** Trailing annotation (last call, a count). */
  meta?: ReactNode
  dot?: RowDot
  /** What the dot means, for screen readers (`Suspended`). */
  dotLabel?: string
  selected: boolean
  onClick: () => void
  /** Registered on the last tick: a wash that fades once it settles. */
  arrived?: boolean
  /** Hangs under a TreeRow (an observer under its function). */
  nested?: boolean
}) {
  return (
    <button
      type="button"
      className="console-catalog-row"
      data-selected={selected}
      data-arrived={arrived || undefined}
      data-nested={nested || undefined}
      aria-current={selected || undefined}
      onClick={onClick}
      title={tooltip}
    >
      {icon ? <span className="row-icon">{icon}</span> : null}
      {dot && nested ? <span className="row-dot" data-dot={dot} aria-hidden /> : null}
      <span className="row-label">{primary}</span>
      {dot && !nested ? <span className="row-dot" data-dot={dot} aria-hidden /> : null}
      {dotLabel ? <span className="console-catalog-sr">, {dotLabel}</span> : null}
      {meta ? <span className="row-meta">{meta}</span> : null}
    </button>
  )
}

/**
 * A row that is also a group: the observed function on the Observers page.
 * The row selects the function; its own disclosure folds its observers.
 */
export function TreeRow({
  icon,
  id,
  tooltip,
  count,
  open,
  onToggle,
  selected,
  onSelect,
}: {
  icon: ReactNode
  id: string
  tooltip?: string
  count: number
  open: boolean
  onToggle: () => void
  selected: boolean
  onSelect: () => void
}) {
  return (
    <div className="console-catalog-tree">
      <button
        type="button"
        className="console-catalog-row"
        data-selected={selected}
        data-tree
        aria-current={selected || undefined}
        onClick={onSelect}
        title={tooltip ?? id}
      >
        <span className="row-icon">{icon}</span>
        <span className="row-label">
          <IdText id={id} />
        </span>
        {count > 0 ? <span className="row-meta">{count}</span> : null}
      </button>
      {count > 0 ? (
        <button
          type="button"
          className="console-catalog-twist"
          tabIndex={-1}
          aria-expanded={open}
          aria-label={`${open ? 'Hide' : 'Show'} observers of ${id}`}
          onClick={onToggle}
        >
          <ChevronRight className="chevron" aria-hidden />
        </button>
      ) : null}
    </div>
  )
}

/* ── glyphs ─────────────────────────────────────────────────────────── */

const FAMILY_ICONS: Record<Family, ReactNode> = {
  http: <Globe />,
  cron: <Clock />,
  queue: <Layers />,
  state: <Database />,
  database: <Rows3 />,
  stream: <Waves />,
  hook: <Webhook />,
  asset: <FileText />,
  other: <Zap />,
}

/** A bare family icon in its tone, for list rows. */
export function FamilyIcon({ family, tone }: { family: Family; tone?: Tone }) {
  return (
    <span className="console-catalog-icon" data-tone={tone} aria-hidden>
      {FAMILY_ICONS[family]}
    </span>
  )
}

/** The masthead tile: a ringed square holding `ƒ` or the family icon. */
export function DocTile({
  family,
  tone,
}: {
  /** Omitted for a function: the tile shows `ƒ`. */
  family?: Family
  tone?: Tone
}) {
  return (
    <span className="console-catalog-tile" data-tone={tone} aria-hidden>
      {family ? FAMILY_ICONS[family] : <span className="fn">ƒ</span>}
    </span>
  )
}

/* ── the main workspace pieces ──────────────────────────────────────── */

/**
 * The workspace before a selection: a ringed icon, a title, a short
 * paragraph and, optionally, the three nouns the page is about.
 */
export function EmptyDoc({
  icon,
  title,
  body,
  items,
  children,
}: {
  icon: ReactNode
  title: string
  body: ReactNode
  items?: readonly { label: string; value: string; icon?: ReactNode }[]
  children?: ReactNode
}) {
  return (
    <div className="console-catalog-empty">
      <div className="card">
        <span className="console-catalog-tile" aria-hidden>
          {icon}
        </span>
        <h2 className="title">{title}</h2>
        <p className="body">{body}</p>
        {items?.length ? (
          <ol className="steps">
            {items.map((item) => (
              <li key={item.label}>
                <span className="step-icon" aria-hidden>
                  {item.icon}
                </span>
                <span className="step-copy">
                  <span className="step-label">{item.label}</span>
                  <span className="step-value">{item.value}</span>
                </span>
              </li>
            ))}
          </ol>
        ) : null}
        {children ? <div className="actions">{children}</div> : null}
      </div>
    </div>
  )
}

export interface MenuAction {
  label: string
  icon?: ReactNode
  onSelect: () => void
  destructive?: boolean
  /** Keep the menu open (a copy that confirms in place). */
  keepOpen?: boolean
}

/**
 * The document masthead: tile, title + status, description, the primary
 * action and a ⋯ menu, then the stat strip. The back arrow only paints
 * when the narrow flow hides the list.
 */
export function DocHead({
  tile,
  eyebrow,
  title,
  status,
  description,
  stats,
  primary,
  menu,
  menuLabel,
  onBack,
  backLabel,
}: {
  tile: ReactNode
  /** One quiet word above the title: `Function`, `Observer`. */
  eyebrow?: ReactNode
  title: ReactNode
  status?: ReactNode
  description?: ReactNode
  stats?: readonly Stat[]
  primary?: ReactNode
  menu?: readonly (MenuAction | 'separator')[]
  menuLabel: string
  onBack: () => void
  backLabel: string
}) {
  return (
    <header className="console-catalog-head">
      <IconButton type="button" className="console-catalog-back" label={backLabel} onClick={onBack}>
        <ArrowLeft />
      </IconButton>
      <div className="head-row">
        {tile}
        <div className="head-copy">
          {eyebrow ? <span className="head-eyebrow">{eyebrow}</span> : null}
          <div className="head-title-line">
            <h2 className="head-title">{title}</h2>
            {status}
          </div>
        </div>
        <div className="head-actions">
          {primary}
          {menu?.length ? <DocMenu items={menu} label={menuLabel} /> : null}
        </div>
      </div>
      {description ? <p className="head-desc">{description}</p> : null}
      {stats?.length ? <StatStrip items={stats} /> : null}
    </header>
  )
}

function DocMenu({ items, label }: { items: readonly (MenuAction | 'separator')[]; label: string }) {
  return (
    <ShadcnDropdownMenu>
      <ShadcnDropdownMenuTrigger
        render={
          <ShadcnButton variant="outline" size="icon-sm" aria-label={label} className="console-catalog-btn">
            <Ellipsis aria-hidden />
          </ShadcnButton>
        }
      />
      <ShadcnDropdownMenuContent align="end" sideOffset={6} className="console-catalog-menu">
        <ShadcnDropdownMenuGroup>
          {items.map((item, i) =>
            item === 'separator' ? (
              <ShadcnDropdownMenuSeparator key={`sep-${String(i)}`} />
            ) : (
              <ShadcnDropdownMenuItem
                key={item.label}
                variant={item.destructive ? 'destructive' : 'default'}
                closeOnClick={!item.keepOpen}
                onClick={item.onSelect}
              >
                {item.icon}
                {item.label}
              </ShadcnDropdownMenuItem>
            ),
          )}
        </ShadcnDropdownMenuGroup>
      </ShadcnDropdownMenuContent>
    </ShadcnDropdownMenu>
  )
}

export interface Stat {
  label: string
  value: ReactNode
  mono?: boolean
}

/** Vercel's summary strip: cells of label over value, hairline-divided. */
export function StatStrip({ items }: { items: readonly Stat[] }) {
  return (
    <dl className="console-catalog-stats">
      {items.map((item) => (
        <div key={item.label}>
          <dt>{item.label}</dt>
          <dd data-mono={item.mono || undefined} title={typeof item.value === 'string' ? item.value : undefined}>
            {item.value}
          </dd>
        </div>
      ))}
    </dl>
  )
}

/** A status word with its dot: `Active`, `Suspended`, `Event source`. */
export function StateBadge({ tone, children }: { tone: 'ok' | 'warn' | 'alert' | 'ink'; children: ReactNode }) {
  return (
    <span className="console-catalog-state" data-tone={tone}>
      <StatusDot tone={tone} />
      {children}
    </span>
  )
}

/**
 * An outlined card of facts: label leading (faint), value trailing,
 * hairline between rows. The title sits above the card, as in Vercel's
 * settings sections.
 */
export function Facts({
  title,
  description,
  items,
  wide,
}: {
  title?: string
  description?: string
  items: readonly { label: string; value: ReactNode; mono?: boolean }[]
  /** Spans both columns of the overview grid. */
  wide?: boolean
}) {
  if (items.length === 0) return null
  return (
    <section className="console-catalog-facts" data-wide={wide || undefined}>
      {title ? (
        <header className="facts-head">
          <h3>{title}</h3>
          {description ? <p>{description}</p> : null}
        </header>
      ) : null}
      <dl>
        {items.map((item) => (
          <div className="fact" key={item.label}>
            <dt>{item.label}</dt>
            <dd data-mono={item.mono || undefined}>{item.value}</dd>
          </div>
        ))}
      </dl>
    </section>
  )
}

export function Note({ children }: { children: ReactNode }) {
  return <div className="console-catalog-note">{children}</div>
}

export function ErrorNote({
  title = 'Request failed',
  call,
  message,
  onRetry,
}: {
  title?: string
  call: string
  message: string
  onRetry?: () => void
}) {
  return (
    <StatusPanel
      variant="alert"
      role="alert"
      className="console-catalog-error"
      headline={title}
      detail={`${call} · ${message}`}
      action={
        onRetry ? (
          <Button variant="secondary" size="sm" type="button" onClick={onRetry} className="console-catalog-btn">
            Retry
          </Button>
        ) : undefined
      }
    />
  )
}

/** A quiet, structural placeholder for the first catalogue load. */
export function CatalogListSkeleton({ label }: { label: string }) {
  return (
    <div className="console-catalog-skeleton" role="status" aria-label={label}>
      {[0, 1, 2].map((group) => (
        <div className="skeleton-group" key={group}>
          <Skeleton className="skeleton-heading" />
          <Skeleton className="skeleton-row" />
          <Skeleton className="skeleton-row" />
          <Skeleton className="skeleton-row short" />
        </div>
      ))}
    </div>
  )
}
