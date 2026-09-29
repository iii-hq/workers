/**
 * Compositions over the shared primitives that every function-trigger view
 * repeats: a read-only row on the list recipe, an eyebrow-headed section,
 * the identity line, metadata pairs and the running-state body.
 */

import { Eyebrow, type MetaRowItem, Skeleton } from '@iii-dev/console-ui'
import uiClasses from '@iii-dev/console-ui/ui-classes'
import type { ReactNode } from 'react'

/** `MetaRow` items from label/value pairs, dropping the empty ones. */
export function kv(pairs: [ReactNode, ReactNode | null | undefined | false][]): MetaRowItem[] {
  return pairs.flatMap(([label, value]) => (value == null || value === false || value === '' ? [] : [{ label, value }]))
}

/** A static list row (a read-out, not a `ListItem` button). */
export function Row({
  title,
  mono,
  description,
  meta,
}: {
  title: ReactNode
  /** The title is a machine id. */
  mono?: boolean
  description?: ReactNode
  /** Trailing column: chips and fine print. */
  meta?: ReactNode
}) {
  return (
    <div className={uiClasses.listItem}>
      <span className={uiClasses.listItemContent}>
        <span className={`${uiClasses.listItemTitle}${mono ? ' dir-ui-mono' : ''}`}>{title}</span>
        {description ? (
          <span className={uiClasses.listItemDescription} title={typeof description === 'string' ? description : undefined}>
            {description}
          </span>
        ) : null}
      </span>
      {meta ? <span className={uiClasses.listItemMeta}>{meta}</span> : null}
    </div>
  )
}

export function Rows({ children }: { children: ReactNode }) {
  return <div className={`${uiClasses.list} dir-ui-rows`}>{children}</div>
}

export function Section({ label, children }: { label: ReactNode; children: ReactNode }) {
  return (
    <div>
      <Eyebrow as="div" className="dir-ui-section-head">
        {label}
      </Eyebrow>
      {children}
    </div>
  )
}

/** The `ƒ name` line body: id in mono, description under it. */
export function Identity({ name, description }: { name: string; description?: string | null }) {
  return (
    <span className="dir-ui-identity">
      <span className="dir-ui-id">{name}</span>
      {description ? <span className="dir-ui-desc">{description}</span> : null}
    </span>
  )
}

/** The running-state body: two breathing lines under the metadata strip. */
export function Loading({ label }: { label: string }) {
  return (
    <div className="dir-ui-loading" role="status" aria-label={label}>
      <Skeleton style={{ width: '58%' }} />
      <Skeleton style={{ width: '36%' }} />
    </div>
  )
}
