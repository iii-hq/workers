// What was captured: how complete it is, which sessions, and what the models
// were allowed to read. Each session opens onto its preview entries, so every
// entry a suggestion cites can be looked at here.
import {
  JsonHighlight,
  StatusPanel,
  Table,
  TableBody,
  TableCell,
  TableFrame,
  TableHead,
  TableHeader,
  TableRow,
  TableViewport,
} from '@iii-dev/console-ui'
import { formatBytes } from '@iii-dev/console-ui/format'
import { Check, ChevronDown, ChevronRight, Info, TriangleAlert } from 'lucide-react'
import { useEffect, useId, useRef, useState } from 'react'
import { entryLabel, shortHash } from '../../../model'
import type { CoverageLevel, JsonValue, MonitorLimits, SessionEvidence, Snapshot } from '../../../types'
import { SectionHead } from './marks'
import { entryKey, locateEntry, plural, reducedEntries, shortId } from './present'
import { scrollToElement } from './scroll'
import type { JumpTarget } from './shared'

function Coverage({ snapshot }: { snapshot: Snapshot }) {
  const { level, limitations } = snapshot.coverage
  const list =
    limitations.length > 0 ? (
      <ul className="eval-ui-ad-list">
        {limitations.map((item, index) => (
          <li key={index}>{item}</li>
        ))}
      </ul>
    ) : null
  const copy: Record<CoverageLevel, { variant: 'success' | 'info' | 'warn'; headline: string; detail: string }> = {
    complete: {
      variant: 'success',
      headline: 'Complete capture',
      detail: 'All pages read. Turn and session tree unchanged when re-checked.',
    },
    partial: {
      variant: 'info',
      headline: 'Partial capture',
      detail: 'Part of the evidence was left out. A signal missing from it is not evidence of healthy behavior.',
    },
    insufficient: {
      variant: 'warn',
      headline: 'Insufficient capture',
      detail: "The evidence can't support a conclusion either way. This doesn't mean the session was healthy.",
    },
  }
  const { variant, headline, detail } = copy[level]
  return (
    <StatusPanel
      variant={variant}
      role={level === 'insufficient' ? 'alert' : 'status'}
      icon={
        level === 'complete' ? (
          <Check size={16} aria-hidden="true" />
        ) : level === 'partial' ? (
          <Info size={16} aria-hidden="true" />
        ) : (
          <TriangleAlert size={16} aria-hidden="true" />
        )
      }
      headline={headline}
      detail={
        <div className="eval-ui-ad-notice-body">
          <p>{detail}</p>
          {list}
        </div>
      }
    />
  )
}

function roleOf(session: SessionEvidence, snapshot: Snapshot): string {
  if (!session.in_scope) return 'Earlier turn (not read)'
  if (session.depth === 0) return `Root · turn ${shortId(snapshot.source_turn_id)}`
  return 'Child'
}

function Entries({
  session,
  snapshot,
  flash,
}: {
  session: SessionEvidence
  snapshot: Snapshot
  flash: { key: string; n: number } | null
}) {
  return (
    <div className="eval-ui-ad-entries">
      {session.preview.map((item: JsonValue, index) => {
        const id =
          typeof item === 'object' && item !== null && !Array.isArray(item) && typeof item.entry_id === 'string'
            ? item.entry_id
            : undefined
        const key = id ? entryKey({ session_id: session.session_id, entry_id: id }) : undefined
        return (
          <div
            key={id ?? index}
            className="eval-ui-ad-entry"
            data-entry-key={key}
            data-flash={(key !== undefined && flash?.key === key) || undefined}
            tabIndex={-1}
          >
            <span className="eval-ui-ad-mono-quiet">
              {id ? entryLabel(snapshot, { session_id: session.session_id, entry_id: id }) : `entry ${index + 1}`}
            </span>
            <JsonHighlight code={JSON.stringify(item, null, 2)} wrap />
          </div>
        )
      })}
    </div>
  )
}

export function Evidence({
  snapshot,
  jump,
  limits,
}: {
  snapshot: Snapshot
  jump: JumpTarget | null
  limits: MonitorLimits | undefined
}) {
  const headingId = useId()
  const section = useRef<HTMLElement>(null)
  const [open, setOpen] = useState<Set<string>>(() => new Set())
  const [flash, setFlash] = useState<{ key: string; n: number } | null>(null)
  const { coverage } = snapshot

  // A chip was clicked: open the session that holds the entry, then light it.
  // Each click is handled once: a reload that replaces the snapshot must not
  // replay it and pull the page back to an old entry.
  const handled = useRef(0)
  useEffect(() => {
    if (!jump || jump.n === handled.current) return
    handled.current = jump.n
    if (locateEntry(snapshot, jump.entry) !== 'preview') return
    setOpen((previous) => new Set(previous).add(jump.entry.session_id))
    setFlash({ key: entryKey(jump.entry), n: jump.n })
  }, [jump, snapshot])

  // The flash and the opened row land in one render, so the entry exists here.
  useEffect(() => {
    if (!flash) return
    const target = Array.from(section.current?.querySelectorAll<HTMLElement>('[data-entry-key]') ?? []).find(
      (element) => element.dataset.entryKey === flash.key,
    )
    scrollToElement(target)
    const timer = window.setTimeout(() => setFlash(null), 1800)
    return () => window.clearTimeout(timer)
  }, [flash])

  const toggle = (sessionId: string) =>
    setOpen((previous) => {
      const next = new Set(previous)
      if (!next.delete(sessionId)) next.add(sessionId)
      return next
    })

  const reduced = reducedEntries(snapshot)
  return (
    <section ref={section} aria-labelledby={headingId} className="eval-ui-ad-section" data-section="evidence">
      <SectionHead
        id={headingId}
        title="Evidence"
        meta={`${plural(coverage.sessions_in_scope, 'session')} · ${plural(coverage.entries_read, 'entry', 'entries')}`}
      />
      <Coverage snapshot={snapshot} />
      <TableViewport>
        <TableFrame>
          <Table density="compact" className="eval-ui-ad-table" aria-labelledby={headingId}>
            <TableHeader>
              <TableRow>
                <TableHead>Session</TableHead>
                <TableHead>Role</TableHead>
                <TableHead className="eval-ui-ad-num">Entries</TableHead>
                <TableHead>Capture hash</TableHead>
                <TableHead>Sent to models</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {snapshot.sessions.map((session) => {
                const expandable = session.preview.length > 0
                const isOpen = open.has(session.session_id)
                return (
                  <SessionRows
                    key={session.session_id}
                    session={session}
                    snapshot={snapshot}
                    role={roleOf(session, snapshot)}
                    expandable={expandable}
                    open={isOpen}
                    onToggle={() => toggle(session.session_id)}
                    flash={flash}
                  />
                )
              })}
            </TableBody>
          </Table>
        </TableFrame>
      </TableViewport>
      <p className="eval-ui-ad-quiet eval-ui-ad-context">
        Context sent:{' '}
        <span className="eval-ui-ad-mono eval-ui-ad-ink">
          {formatBytes(coverage.context_bytes)}
          {limits ? ` of ${formatBytes(limits.model_context_bytes)}` : ''}
        </span>
        . {plural(reduced, 'entry', 'entries')} shortened or masked. Masking can't guarantee free text is clean.
      </p>
    </section>
  )
}

function SessionRows({
  session,
  snapshot,
  role,
  expandable,
  open,
  onToggle,
  flash,
}: {
  session: SessionEvidence
  snapshot: Snapshot
  role: string
  expandable: boolean
  open: boolean
  onToggle: () => void
  flash: { key: string; n: number } | null
}) {
  const detailId = useId()
  return (
    <>
      <TableRow className="eval-ui-ad-session-row" data-in-scope={session.in_scope}>
        <TableCell className="eval-ui-ad-mono">
          {expandable ? (
            <button
              type="button"
              className="eval-ui-ad-expand"
              aria-expanded={open}
              aria-controls={open ? detailId : undefined}
              onClick={onToggle}
            >
              {open ? <ChevronDown size={16} aria-hidden="true" /> : <ChevronRight size={16} aria-hidden="true" />}
              <span>{session.session_id}</span>
            </button>
          ) : (
            session.session_id
          )}
        </TableCell>
        <TableCell>{role}</TableCell>
        <TableCell className="eval-ui-ad-mono eval-ui-ad-num">{session.in_scope ? session.entries : '—'}</TableCell>
        <TableCell className="eval-ui-ad-mono eval-ui-ad-quiet" title={session.json_sha256}>
          {shortHash(session.json_sha256)}
        </TableCell>
        <TableCell className="eval-ui-ad-mono">
          {session.in_scope ? (
            <>
              {session.preview.length}
              {session.omitted_entries > 0 ? (
                <span className="eval-ui-ad-quiet"> · {session.omitted_entries} omitted</span>
              ) : null}
            </>
          ) : (
            '—'
          )}
        </TableCell>
      </TableRow>
      {open ? (
        <TableRow className="eval-ui-ad-entries-row">
          <TableCell colSpan={5} id={detailId}>
            <Entries session={session} snapshot={snapshot} flash={flash} />
          </TableCell>
        </TableRow>
      ) : null}
    </>
  )
}
