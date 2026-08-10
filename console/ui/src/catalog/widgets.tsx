/**
 * The chrome both catalogue pages share, arranged like the directory page:
 * a fixed navigation sidebar (search, filters, grouped rows) and a main
 * workspace that is ALWAYS rendered — a hero empty state when nothing is
 * selected, the document view (breadcrumb → identity head → tabs) when
 * something is.
 *
 * Page chrome comes from `@iii-dev/console-ui` (PageShell/PageBody/
 * PageSidebar/PageMain — the console's own layout system, zero bytes in
 * this bundle); everything else is a scoped class in ../../styles.css.
 *
 * Every list row leads with a glyph tile so the list reads as WHAT it is
 * at a glance: `ƒ` for functions, a family icon (globe, clock, layers…)
 * for triggers. The same tiles reappear scaled up in the identity head and
 * the hero, so the sidebar and the workspace visibly describe one thing.
 */

import {
  Button,
  Input,
  PageBody,
  PageMain,
  PageShell,
  PageSidebar,
} from '@iii-dev/console-ui'
import { Fragment, type ReactNode, useCallback, useState } from 'react'
import type { Family, Tone } from './trigger-kinds'

/**
 * Open/closed state for collapsible groups, stored as the set of ids whose
 * state is FLIPPED from the default. Storing flips rather than open ids
 * keeps a list that grows (a worker connects, a new group appears) honest:
 * the newcomer follows the default instead of arriving collapsed.
 *
 * The default is a predicate because the pages disagree: function groups
 * always open, trigger types open only when something is bound to them.
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
  const isOpen = (id: string) =>
    flipped.has(id) ? !defaultOpen(id) : defaultOpen(id)
  return { isOpen, toggle }
}

/**
 * The catalogue page frame: header, the live now-strip, then sidebar |
 * main. `hasSelection` rides on PageBody as a data attribute so the
 * narrow-container styles can drill in (list OR document, never a squeeze)
 * — the pane, not the viewport, decides.
 */
export function CatalogShell({
  header,
  strip,
  sideTop,
  list,
  sideFooter,
  main,
  side,
  hasSelection,
}: {
  header: ReactNode
  /** The full-width live strip between header and columns. */
  strip?: ReactNode
  /** Sidebar top block: search, filters, the count line. */
  sideTop: ReactNode
  /** The scrolling group/row list (plumbing footer included). */
  list: ReactNode
  /** A persistent catalogue summary below the scrolling list. */
  sideFooter?: ReactNode
  /** The workspace: a hero when nothing is selected, else the document. */
  main: ReactNode
  /** The hosting pane's side (`PageRenderProps.panelSide`). */
  side?: 'left' | 'right'
  hasSelection: boolean
}) {
  return (
    <PageShell className="console-catalog">
      {header}
      {strip ? <div className="console-catalog-strip">{strip}</div> : null}
      <PageBody
        side={side}
        className="console-catalog-body"
        data-selected={hasSelection}
      >
        <PageSidebar width={320} className="console-catalog-side">
          <div className="console-catalog-side-top">{sideTop}</div>
          <div className="console-catalog-side-scroll">{list}</div>
          {sideFooter ? (
            <div className="console-catalog-side-footer">{sideFooter}</div>
          ) : null}
        </PageSidebar>
        <PageMain className="console-catalog-main">{main}</PageMain>
      </PageBody>
    </PageShell>
  )
}

function SearchGlassIcon() {
  return (
    <svg viewBox="0 0 16 16" fill="none" aria-hidden="true" className="icon">
      <circle cx="7" cy="7" r="4.5" stroke="currentColor" strokeWidth="1.5" />
      <path
        d="M10.5 10.5L14 14"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinecap="round"
      />
    </svg>
  )
}

/** The sidebar search box: leading magnifier, esc/× to clear. */
export function SearchField({
  value,
  onChange,
  placeholder,
}: {
  value: string
  onChange: (next: string) => void
  placeholder: string
}) {
  return (
    <div className="console-catalog-search">
      <SearchGlassIcon />
      <Input
        name="catalog-search"
        value={value}
        onChange={onChange}
        preserveCase
        placeholder={placeholder}
        aria-label={placeholder}
        className="console-catalog-search-input"
        onKeyDown={(e) => {
          if (e.key === 'Escape' && value) {
            e.stopPropagation()
            onChange('')
          }
        }}
      />
      {value ? (
        <button
          type="button"
          className="clear"
          aria-label="clear search"
          onClick={() => onChange('')}
        >
          ×
        </button>
      ) : null}
    </div>
  )
}

/** The quiet count line under the search box. */
export function SideCount({ children }: { children: ReactNode }) {
  return (
    <div className="console-catalog-count" aria-live="polite">
      {children}
    </div>
  )
}

/**
 * One chip per family that actually has entries, plus `all`. Families with
 * nothing registered are not rendered: an empty filter is a dead control.
 */
export function FilterChips<T extends string>({
  counts,
  selected,
  onSelect,
}: {
  counts: ReadonlyMap<T, number>
  selected: T | null
  onSelect: (next: T | null) => void
}) {
  const entries = [...counts.entries()].filter(([, n]) => n > 0).sort()
  if (entries.length < 2) return null
  const total = entries.reduce((n, [, count]) => n + count, 0)
  return (
    <div className="console-catalog-filters">
      <button
        type="button"
        className="console-catalog-filter"
        data-selected={selected === null}
        aria-pressed={selected === null}
        onClick={() => onSelect(null)}
      >
        all <span className="count">{total}</span>
      </button>
      {entries.map(([key, count]) => (
        <button
          key={key}
          type="button"
          className="console-catalog-filter"
          data-selected={selected === key}
          aria-pressed={selected === key}
          onClick={() => onSelect(selected === key ? null : key)}
        >
          {key} <span className="count">{count}</span>
        </button>
      ))}
    </div>
  )
}

/** A labelled fact in the detail pane: next run, method, topic, status. */
export function StatTile({
  label,
  value,
  hint,
  tone,
}: {
  label: string
  value: string
  hint?: string
  tone?: 'ok' | 'warn' | 'alert'
}) {
  return (
    <div className="console-catalog-tile">
      <span className="label">{label}</span>
      <span className="value" data-tone={tone}>
        {value}
      </span>
      {hint ? <span className="hint">{hint}</span> : null}
    </div>
  )
}

/**
 * The "this page is live" marker. Every catalogue page here is driven by an
 * engine signal rather than a timer, and the operator deserves to know that
 * without reading the source.
 */
export function LiveDot() {
  return (
    <span
      className="console-catalog-live"
      title="catalog updates live from the engine"
      role="status"
      aria-label="catalog updates live"
    >
      <span className="dot" />
      live
    </span>
  )
}

/** Copy-to-clipboard with the two-second confirmation the old console had. */
export function CopyButton({
  value,
  label = 'copy',
  title,
}: {
  value: string
  label?: string
  title?: string
}) {
  const [copied, setCopied] = useState(false)
  return (
    <Button
      variant="pill"
      size="sm"
      type="button"
      title={title}
      onClick={async () => {
        try {
          await navigator.clipboard.writeText(value)
        } catch {
          return
        }
        setCopied(true)
        window.setTimeout(() => setCopied(false), 2000)
      }}
    >
      {copied ? 'copied' : label}
    </Button>
  )
}

/**
 * A collapsible group heading. The chevron is its own button so the label
 * can be a second, independent target: the triggers page selects the TYPE
 * by its heading (no phantom "type detail" row), the functions page just
 * toggles.
 */
export function GroupHeader({
  label,
  meta,
  count,
  countLabel,
  open,
  onToggle,
  onSelect,
  selected,
  tone,
  toneLabel,
  collapsible = true,
}: {
  label: string
  meta?: string
  count?: number
  countLabel?: string
  open: boolean
  onToggle: () => void
  /** Clicking the label selects the group's own detail (trigger types). */
  onSelect?: () => void
  selected?: boolean
  /** Family color for the leading tag, when the page groups by family. */
  tone?: string
  toneLabel?: string
  /** Empty trigger types have nothing to disclose. */
  collapsible?: boolean
}) {
  const countText = count === undefined ? null : String(count)
  const countTitle =
    count === undefined
      ? undefined
      : `${count} ${countLabel ?? 'item'}${count === 1 ? '' : 's'}`
  const content = (
    <>
      <span className="group-label-line">
        {toneLabel ? (
          <span className="console-catalog-tag" data-tone={tone}>
            {toneLabel}
          </span>
        ) : null}
        <span className="group-name" title={label}>
          {label}
        </span>
        {countText ? (
          <span className="group-count" title={countTitle}>
            {countText}
          </span>
        ) : null}
      </span>
      {meta ? (
        <span className="detail">
          <span className="meta">{meta}</span>
        </span>
      ) : null}
    </>
  )

  return (
    <div className="console-catalog-group" data-selected={selected === true}>
      <button
        type="button"
        className="pick"
        onClick={onSelect ?? onToggle}
        aria-pressed={onSelect ? selected === true : undefined}
        aria-expanded={onSelect ? undefined : open}
      >
        {content}
        {!onSelect && collapsible ? (
          <span className="chevron" data-open={open} aria-hidden>
            ▸
          </span>
        ) : null}
      </button>
      {onSelect && collapsible ? (
        <button
          type="button"
          className="twist"
          onClick={onToggle}
          aria-expanded={open}
          aria-label={`${open ? 'collapse' : 'expand'} ${label}`}
        >
          <span className="chevron" data-open={open} aria-hidden>
            ▸
          </span>
        </button>
      ) : null}
    </div>
  )
}

export function CatalogRow({
  icon,
  primary,
  secondary,
  meta,
  selected,
  onClick,
  flash,
}: {
  /** The leading glyph tile — what KIND of thing this row is. */
  icon?: ReactNode
  primary: ReactNode
  secondary?: ReactNode
  /** Right-aligned live annotation on the primary line (last call, ago). */
  meta?: ReactNode
  selected: boolean
  onClick: () => void
  /** Highlight once: this row's function just ran (or the row just arrived). */
  flash?: boolean
}) {
  return (
    <button
      type="button"
      className={`console-catalog-row${flash ? ' flash' : ''}`}
      data-selected={selected}
      aria-pressed={selected}
      onClick={onClick}
      title={typeof primary === 'string' ? primary : undefined}
    >
      {icon ? <span className="row-glyph">{icon}</span> : null}
      <span className="row-copy">
        <span className="primary-line">
          <span className="primary">{primary}</span>
          {meta}
        </span>
        {secondary ? <span className="secondary">{secondary}</span> : null}
      </span>
    </button>
  )
}

/* ── glyph tiles ────────────────────────────────────────────────────── */

/** The `ƒ` tile that marks a function everywhere: rows, head, hero. */
export function FnGlyph({ size }: { size?: 'lg' | 'hero' }) {
  return (
    <span className="console-catalog-glyph" data-size={size} aria-hidden>
      ƒ
    </span>
  )
}

const FAMILY_ICONS: Record<Family, ReactNode> = {
  http: (
    <>
      <circle cx="8" cy="8" r="6.25" />
      <path d="M1.75 8h12.5" />
      <ellipse cx="8" cy="8" rx="2.75" ry="6.25" />
    </>
  ),
  cron: (
    <>
      <circle cx="8" cy="8" r="6.25" />
      <path d="M8 4.5V8l2.4 1.5" />
    </>
  ),
  queue: (
    <>
      <path d="M8 2l6 3-6 3-6-3z" />
      <path d="M2 8l6 3 6-3" />
      <path d="M2 11l6 3 6-3" />
    </>
  ),
  state: (
    <>
      <ellipse cx="8" cy="4" rx="6" ry="2.25" />
      <path d="M2 4v8c0 1.25 2.7 2.25 6 2.25s6-1 6-2.25V4" />
      <path d="M2 8c0 1.25 2.7 2.25 6 2.25S14 9.25 14 8" />
    </>
  ),
  stream: (
    <>
      <path d="M1.5 5.5c1.1-1.3 2.2-1.3 3.25 0s2.2 1.3 3.25 0 2.2-1.3 3.25 0 2.2 1.3 3.25 0" />
      <path d="M1.5 10.5c1.1-1.3 2.2-1.3 3.25 0s2.2 1.3 3.25 0 2.2-1.3 3.25 0 2.2 1.3 3.25 0" />
    </>
  ),
  hook: (
    <>
      <path d="M6.5 9.5l3-3" />
      <path d="M5.25 6.75L3.5 8.5a2.5 2.5 0 003.5 3.5l1.75-1.75" />
      <path d="M10.75 9.25l1.75-1.75a2.5 2.5 0 00-3.5-3.5L7.25 5.75" />
    </>
  ),
  asset: (
    <>
      <path d="M4 1.75h5L12.25 5v9.25H4z" />
      <path d="M9 1.75V5h3.25" />
    </>
  ),
  other: <path d="M8.75 1.5L3.5 9H7l-.75 5.5L11.5 7H8z" />,
}

/** The family tile that marks a trigger: globe, clock, layers, waves… */
export function FamilyGlyph({
  family,
  tone,
  size,
}: {
  family: Family
  tone?: Tone
  size?: 'lg' | 'hero'
}) {
  return (
    <span
      className="console-catalog-glyph"
      data-tone={tone}
      data-size={size}
      aria-hidden
    >
      <svg
        viewBox="0 0 16 16"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinecap="round"
        strokeLinejoin="round"
        aria-hidden="true"
      >
        {FAMILY_ICONS[family]}
      </svg>
    </span>
  )
}

/* ── the main workspace pieces ──────────────────────────────────────── */

/**
 * The workspace empty state, on the directory page's pattern: the page's
 * glyph, one title, a short paragraph. It renders where the document will,
 * so selecting something changes content, never layout.
 */
export function Hero({
  glyph,
  eyebrow,
  title,
  body,
  items,
}: {
  glyph: ReactNode
  eyebrow?: string
  title: string
  body: string
  items?: readonly { label: string; value: string }[]
}) {
  return (
    <div className="console-catalog-hero">
      <div className="inner">
        {glyph}
        <div className="copy">
          {eyebrow ? <span className="eyebrow">{eyebrow}</span> : null}
          <h2 className="title">{title}</h2>
          <p className="body">{body}</p>
        </div>
        {items?.length ? (
          <dl className="guide">
            {items.map((item) => (
              <div className="guide-row" key={item.label}>
                <dt className="guide-term">{item.label}</dt>
                <dd className="guide-description">{item.value}</dd>
              </div>
            ))}
          </dl>
        ) : null}
      </div>
    </div>
  )
}

/**
 * A selected catalogue item has two jobs: the primary work surface and a
 * compact reference rail. Keeping them in one component lets the rail stack
 * below the document when the host pane is too narrow for three columns.
 */
export function CatalogWorkspace({
  children,
  context,
}: {
  children: ReactNode
  context?: ReactNode
}) {
  return (
    <div className="console-catalog-workspace">
      <div className="console-catalog-workspace-main">{children}</div>
      {context ? (
        <aside
          className="console-catalog-context"
          aria-label="selection context"
        >
          {context}
        </aside>
      ) : null}
    </div>
  )
}

/** A titled group in the contextual rail. */
export function ContextPanel({
  title,
  description,
  action,
  children,
  wide = false,
}: {
  title: string
  description?: string
  action?: { label: string; onClick: () => void }
  children: ReactNode
  wide?: boolean
}) {
  return (
    <section className="console-catalog-context-panel" data-wide={wide}>
      <div className="context-head">
        <div className="context-heading">
          <h3>{title}</h3>
          {description ? <p>{description}</p> : null}
        </div>
        {action ? (
          <Button
            variant="pill"
            size="sm"
            type="button"
            onClick={action.onClick}
          >
            {action.label}
          </Button>
        ) : null}
      </div>
      <div className="context-body">{children}</div>
    </section>
  )
}

/** A dense object summary used for related triggers and target functions. */
export function ContextItem({
  glyph,
  title,
  description,
  meta,
  onClick,
}: {
  glyph?: ReactNode
  title: string
  description?: ReactNode
  meta?: ReactNode
  onClick?: () => void
}) {
  const content = (
    <>
      {glyph ? <span className="context-item-glyph">{glyph}</span> : null}
      <span className="context-item-copy">
        <span className="context-item-title">{title}</span>
        {description ? (
          <span className="context-item-description">{description}</span>
        ) : null}
        {meta ? <span className="context-item-meta">{meta}</span> : null}
      </span>
      {onClick ? (
        <span className="context-item-arrow" aria-hidden>
          →
        </span>
      ) : null}
    </>
  )

  return onClick ? (
    <button
      type="button"
      className="console-catalog-context-item"
      onClick={onClick}
    >
      {content}
    </button>
  ) : (
    <div className="console-catalog-context-item">{content}</div>
  )
}

/**
 * The document's place line: `functions › harness › harness::state::list`.
 * The back button only paints in narrow containers, where the document
 * replaces the list and needs a way out.
 */
export function Crumb({
  trail,
  onBack,
}: {
  trail: readonly (string | undefined)[]
  onBack: () => void
}) {
  const segments = trail.filter((s): s is string => Boolean(s))
  return (
    <div className="console-catalog-crumb">
      <button
        type="button"
        className="console-catalog-back"
        onClick={onBack}
        aria-label="back to the list"
      >
        ←
      </button>
      {segments.map((segment, i) => (
        <Fragment key={`${i}-${segment}`}>
          {i > 0 ? (
            <span className="sep" aria-hidden>
              ›
            </span>
          ) : null}
          <span className="seg" data-last={i === segments.length - 1}>
            {segment}
          </span>
        </Fragment>
      ))}
    </div>
  )
}

/** The document masthead: big glyph, name, description, chips, actions. */
export function IdentityHead({
  glyph,
  title,
  status,
  description,
  chips,
  actions,
}: {
  glyph: ReactNode
  title: string
  status?: string
  description?: string | null
  chips?: ReactNode
  actions?: ReactNode
}) {
  return (
    <div className="console-catalog-ident">
      {glyph}
      <div className="ident-copy">
        <div className="ident-title-line">
          <h2 className="ident-title">{title}</h2>
          {status ? <span className="ident-status">{status}</span> : null}
        </div>
        {description ? <p className="ident-desc">{description}</p> : null}
        {chips ? <div className="ident-chips">{chips}</div> : null}
      </div>
      {actions ? <div className="ident-actions">{actions}</div> : null}
    </div>
  )
}

/** The label/value fact sheet (the prototype's "function details" card). */
export function Facts({
  items,
}: {
  items: readonly { label: string; value: ReactNode }[]
}) {
  if (items.length === 0) return null
  return (
    <dl className="console-catalog-facts">
      {items.map((item) => (
        <div className="fact" key={item.label}>
          <dt>{item.label}</dt>
          <dd>{item.value}</dd>
        </div>
      ))}
    </dl>
  )
}

export function Note({ children }: { children: ReactNode }) {
  return <div className="console-catalog-note">{children}</div>
}

export function ErrorNote({
  title = 'request failed',
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
    <div className="console-catalog-error" role="alert">
      <div className="error-copy">
        <span className="error-title">{title}</span>
        <span className="error-detail">
          {call} · {message}
        </span>
      </div>
      {onRetry ? (
        <Button variant="pill" size="sm" type="button" onClick={onRetry}>
          retry
        </Button>
      ) : null}
    </div>
  )
}

/** A quiet, structural placeholder for the first catalogue load. */
export function CatalogListSkeleton({ label }: { label: string }) {
  return (
    <div className="console-catalog-skeleton" role="status" aria-label={label}>
      {[0, 1, 2].map((group) => (
        <div className="skeleton-group" key={group}>
          <span className="skeleton-heading" />
          <span className="skeleton-row" />
          <span className="skeleton-row short" />
        </div>
      ))}
    </div>
  )
}

/** Key/value chips for a trigger config, ids, counts. */
export function Chip({
  k,
  v,
  tone,
}: {
  k: string
  v: ReactNode
  tone?: string
}) {
  return (
    <span className="console-catalog-chip" data-tone={tone}>
      <span className="k">{k}</span>
      {v}
    </span>
  )
}
