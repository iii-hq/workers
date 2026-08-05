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
  detail,
}: {
  head: ReactNode
  list: ReactNode
  detail: ReactNode | null
}) {
  return (
    <div className="console-catalog">
      {head}
      <div className="console-catalog-body">
        <div className="console-catalog-list">{list}</div>
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
}: {
  title: string
  count: ReactNode
  search: string
  onSearch: (next: string) => void
  searchPlaceholder: string
  onRefresh: () => void
  loading: boolean
  children?: ReactNode
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
    </div>
  )
}

export function GroupHeader({
  label,
  meta,
  open,
  onToggle,
}: {
  label: string
  meta: string
  open: boolean
  onToggle: () => void
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
      <span className="label">{label}</span>
      <span className="meta">{meta}</span>
    </button>
  )
}

export function CatalogRow({
  primary,
  secondary,
  selected,
  onClick,
}: {
  primary: ReactNode
  secondary?: ReactNode
  selected: boolean
  onClick: () => void
}) {
  return (
    <button
      type="button"
      className="console-catalog-row"
      data-selected={selected}
      onClick={onClick}
    >
      <span className="primary">{primary}</span>
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
export function Chip({ k, v }: { k: string; v: ReactNode }) {
  return (
    <span className="console-catalog-chip">
      <span className="k">{k}</span>
      {v}
    </span>
  )
}
