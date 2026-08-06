/**
 * The chrome both catalogue pages share: the two-pane shell, the head row
 * with search and toggles, collapsible group headers, list rows, and the
 * detail pane header. Components come from `@iii-dev/console-ui` (the
 * console's own, zero bytes in this bundle); everything else is a scoped
 * class in ../../styles.css.
 */

import { Badge, Button, Input } from '@iii-dev/console-ui'
import { type ReactNode, useCallback, useState } from 'react'

/**
 * Open/closed state for collapsible groups, stored as the set of ids whose
 * state is FLIPPED from the default. Storing flips rather than open ids
 * keeps a list that grows (a worker connects, a new group appears) honest:
 * the newcomer follows the default instead of arriving collapsed.
 *
 * The default is a predicate because the pages disagree: functions groups
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

export function CatalogShell({
  head,
  list,
  footer,
  detail,
}: {
  head: ReactNode
  list: ReactNode
  /** Rendered after the list inside the same scroll pane (plumbing section). */
  footer?: ReactNode
  detail: ReactNode | null
}) {
  return (
    <div className="console-catalog">
      {head}
      <div className="console-catalog-body">
        <div className="console-catalog-list">
          {list}
          {footer}
        </div>
        {detail ? <div className="console-catalog-detail">{detail}</div> : null}
      </div>
    </div>
  )
}

export function CatalogHead({
  title,
  count,
  search,
  onSearch,
  searchPlaceholder,
  onRefresh,
  loading,
  children,
  below,
}: {
  title: string
  count: ReactNode
  search: string
  onSearch: (next: string) => void
  searchPlaceholder: string
  onRefresh: () => void
  loading: boolean
  children?: ReactNode
  /** Row under the search box: filter chips, when the page has them. */
  below?: ReactNode
}) {
  return (
    <div className="console-catalog-head">
      <div className="console-catalog-head-row">
        <span className="console-catalog-title">{title}</span>
        <Badge>{count}</Badge>
        <span style={{ flex: 1 }} />
        {children}
        <Button variant="pill" size="sm" onClick={onRefresh} disabled={loading}>
          {loading ? 'loading…' : 'refresh'}
        </Button>
      </div>
      <Input
        value={search}
        onChange={onSearch}
        preserveCase
        placeholder={searchPlaceholder}
        aria-label={searchPlaceholder}
        className="console-catalog-search"
      />
      {below}
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
    <span className="console-catalog-live" title="live over engine signals">
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
      title={title}
      onClick={() => {
        void navigator.clipboard.writeText(value)
        setCopied(true)
        window.setTimeout(() => setCopied(false), 2000)
      }}
    >
      {copied ? 'copied' : label}
    </Button>
  )
}

export function GroupHeader({
  label,
  meta,
  open,
  onToggle,
  tone,
  toneLabel,
}: {
  label: string
  meta: string
  open: boolean
  onToggle: () => void
  /** Family color for the leading tag, when the page groups by family. */
  tone?: string
  toneLabel?: string
}) {
  return (
    <button
      type="button"
      className="console-catalog-group"
      onClick={onToggle}
      aria-expanded={open}
    >
      <span className="chevron" data-open={open}>
        ▸
      </span>
      {toneLabel ? (
        <span className="console-catalog-tag" data-tone={tone}>
          {toneLabel}
        </span>
      ) : null}
      <span className="label">{label}</span>
      <span className="meta">{meta}</span>
    </button>
  )
}

export function CatalogRow({
  primary,
  secondary,
  meta,
  selected,
  onClick,
  flash,
}: {
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
      onClick={onClick}
    >
      <span className="primary-line">
        <span className="primary">{primary}</span>
        {meta}
      </span>
      {secondary ? <span className="secondary">{secondary}</span> : null}
    </button>
  )
}

export function DetailHead({
  title,
  subtitle,
  onClose,
  children,
}: {
  title: string
  subtitle?: ReactNode
  onClose: () => void
  /** Actions left of `close` — copy buttons, mostly. */
  children?: ReactNode
}) {
  return (
    <div className="console-catalog-detail-head">
      <div className="console-catalog-head-row">
        <span className="console-catalog-detail-title">{title}</span>
        <span style={{ flex: 1 }} />
        {children}
        <Button variant="pill" size="sm" onClick={onClose}>
          close
        </Button>
      </div>
      {subtitle ? (
        <div className="console-catalog-detail-sub">{subtitle}</div>
      ) : null}
    </div>
  )
}

export function Note({ children }: { children: ReactNode }) {
  return <div className="console-catalog-note">{children}</div>
}

export function ErrorNote({
  call,
  message,
}: {
  call: string
  message: string
}) {
  return (
    <div className="console-catalog-error">
      {call} failed — {message}
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
