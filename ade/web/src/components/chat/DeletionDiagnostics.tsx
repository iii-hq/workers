import { useCopyFlash } from '@iii-dev/console-ui/hooks'
import { ChevronRight } from 'lucide-react'
import { useId, useMemo, useState } from 'react'
import { Badge } from '@/components/ui/Badge'
import { Button } from '@/components/ui/Button'
import { SearchField } from '@/components/ui/SearchField'
import { StatusPanel } from '@/components/ui/StatusPanel'
import {
  deletionOutcomes,
  hasDeletionOutcomeCounts,
  type SessionTreeDeletionSnapshot,
} from '@/lib/sessions/delete-tree'
import {
  blockerLabels,
  DIAGNOSTIC_PAGE_SIZE,
  groupDeletionBlockers,
  type IndexedBlocker,
  observedTimestamp,
  pageBlockerGroups,
} from './deletion-diagnostics'

function Pagination({
  page,
  total,
  onChange,
  label,
}: {
  page: number
  total: number
  onChange: (page: number) => void
  label: string
}) {
  if (total <= DIAGNOSTIC_PAGE_SIZE) return null
  return (
    <nav aria-label={label} data-deletion-pagination="">
      <Button
        type="button"
        variant="ghost"
        size="lg"
        aria-disabled={page === 0}
        className="aria-disabled:opacity-40"
        onClick={() => {
          if (page > 0) onChange(page - 1)
        }}
      >
        Previous
      </Button>
      <span className="tabular-nums text-ink-faint text-xs">
        Page {page + 1} of {Math.ceil(total / DIAGNOSTIC_PAGE_SIZE)}
      </span>
      <Button
        type="button"
        variant="ghost"
        size="lg"
        aria-disabled={(page + 1) * DIAGNOSTIC_PAGE_SIZE >= total}
        className="aria-disabled:opacity-40"
        onClick={() => {
          if ((page + 1) * DIAGNOSTIC_PAGE_SIZE < total) onChange(page + 1)
        }}
      >
        Next
      </Button>
    </nav>
  )
}

function BlockerDetails({ entry }: { entry: IndexedBlocker }) {
  const [open, setOpen] = useState(false)
  const { blocker, index } = entry
  const fields = [
    ['Function', blocker.function_id || 'Not reported'],
    ['Session', blocker.session_id || 'Not reported'],
    ['Call', blocker.call_id || 'Not reported'],
    ['Observed', observedTimestamp(blocker.started_at)],
  ]
  const copied = useCopyFlash(
    [
      `Blocker: ${blockerLabels[blocker.kind]}`,
      ...fields.map(([label, value]) => `${label}: ${value}`),
    ].join('\n'),
  )
  return (
    <details
      data-deletion-blocker=""
      onToggle={(event) => setOpen(event.currentTarget.open)}
    >
      <summary>
        <ChevronRight className="size-4 shrink-0" aria-hidden />
        <span className="min-w-0 flex-1">
          <span
            className={
              blocker.function_id
                ? 'block truncate font-mono text-ink'
                : 'block text-ink'
            }
            title={blocker.function_id}
          >
            {blocker.function_id || 'Session activity'}
          </span>
          <span className="block text-xs text-ink-faint">
            {blockerLabels[blocker.kind]}
          </span>
        </span>
        <span className="sr-only">
          {' '}
          — blocker {index + 1}; expand for full details
        </span>
      </summary>
      {open ? (
        <div data-deletion-blocker-detail="">
          <dl>
            {fields.map(([label, value]) => (
              <div key={label}>
                <dt>{label}</dt>
                <dd className="font-mono tabular-nums">{value}</dd>
              </div>
            ))}
          </dl>
          <Button type="button" variant="ghost" size="lg" onClick={copied.copy}>
            Copy details
          </Button>
          <span role="status" className="text-xs text-ink-faint">
            {copied.state === 'copied'
              ? 'Copied'
              : copied.state === 'failed'
                ? 'Copy failed; select the details to copy.'
                : ''}
          </span>
        </div>
      ) : null}
    </details>
  )
}

/** Read-only diagnostics; filtering/pagination cannot affect Delete or Force. */
export function DeletionDiagnostics({
  snapshot,
}: {
  snapshot: SessionTreeDeletionSnapshot
}) {
  const [query, setQuery] = useState('')
  const [requestedPage, setPage] = useState(0)
  const [outcomesOpen, setOutcomesOpen] = useState(false)
  const [requestedOutcomePage, setOutcomePage] = useState(0)
  const resultsId = useId()
  const blockers = snapshot.blockers ?? []
  const groups = useMemo(
    () => groupDeletionBlockers(blockers, query),
    [blockers, query],
  )
  const matches = groups.reduce(
    (count, group) => count + group.entries.length,
    0,
  )
  const page = Math.min(
    requestedPage,
    Math.max(0, Math.ceil(matches / DIAGNOSTIC_PAGE_SIZE) - 1),
  )
  const visibleGroups = pageBlockerGroups(groups, page)
  const affectedChats = new Set(
    blockers.map((blocker) => blocker.session_id).filter(Boolean),
  ).size
  const unidentified = blockers.filter((blocker) => !blocker.session_id).length
  const outcomes = useMemo(() => deletionOutcomes(snapshot), [snapshot])
  const countsKnown = hasDeletionOutcomeCounts(snapshot)
  const outcomeRows = useMemo(
    () => [
      ...snapshot.deleted_session_ids.map((id, position) => ({
        id,
        key: `deleted:${position}:${id}`,
        label: 'Deleted sessions',
      })),
      ...outcomes.notDeleted.map((id, position) => ({
        id,
        key: `not-deleted:${position}:${id}`,
        label: 'Not deleted sessions',
      })),
      ...outcomes.unconfirmed.map((id, position) => ({
        id,
        key: `unconfirmed:${position}:${id}`,
        label: 'Deletion outcome unconfirmed',
      })),
    ],
    [snapshot.deleted_session_ids, outcomes],
  )
  const outcomePage = Math.min(
    requestedOutcomePage,
    Math.max(0, Math.ceil(outcomeRows.length / DIAGNOSTIC_PAGE_SIZE) - 1),
  )
  const visibleOutcomes = outcomeRows.slice(
    outcomePage * DIAGNOSTIC_PAGE_SIZE,
    (outcomePage + 1) * DIAGNOSTIC_PAGE_SIZE,
  )

  return (
    <section data-chat-delete-diagnostics="" aria-label="Deletion diagnostics">
      <dl data-deletion-outcome-counts="">
        {[
          ['Confirmed deleted', snapshot.deleted_session_ids.length],
          ['Not deleted', countsKnown ? outcomes.notDeleted.length : 'unknown'],
          [
            'Unconfirmed',
            countsKnown ? outcomes.unconfirmed.length : 'unknown',
          ],
        ].map(([label, count]) => (
          <div key={label}>
            <dt>{label}</dt>
            <dd className="font-mono tabular-nums text-ink">{count}</dd>
          </div>
        ))}
      </dl>
      {outcomeRows.length ? (
        <details
          data-deletion-outcomes=""
          onToggle={(event) => setOutcomesOpen(event.currentTarget.open)}
        >
          <summary>
            <ChevronRight className="size-4" aria-hidden />
            Outcome details · reported chat IDs
          </summary>
          {outcomesOpen ? (
            <div>
              <p
                className="text-xs text-ink-faint tabular-nums"
                aria-live="polite"
              >
                Showing {outcomePage * DIAGNOSTIC_PAGE_SIZE + 1}–
                {Math.min(
                  (outcomePage + 1) * DIAGNOSTIC_PAGE_SIZE,
                  outcomeRows.length,
                )}{' '}
                of {outcomeRows.length} reported IDs.
                {!countsKnown
                  ? ' These IDs do not establish the full deletion scope.'
                  : ''}
              </p>
              <ul
                aria-label="Reported deletion outcomes"
                data-deletion-outcome-list=""
                // biome-ignore lint/a11y/noNoninteractiveTabindex: Keyboard users must be able to scroll the bounded outcome list without changing its list semantics.
                tabIndex={0}
              >
                {visibleOutcomes.map((row) => (
                  <li key={row.key}>
                    <span className="block text-xs text-ink-faint">
                      {row.label}:{' '}
                    </span>
                    <span className="font-mono">
                      {row.id || 'Not reported'}
                    </span>
                  </li>
                ))}
              </ul>
              <Pagination
                page={outcomePage}
                total={outcomeRows.length}
                onChange={setOutcomePage}
                label="Outcome pages"
              />
            </div>
          ) : null}
        </details>
      ) : null}
      <div data-deletion-blockers="">
        <div className="flex flex-wrap items-center gap-2">
          <h3 className="font-medium text-ink">Blockers</h3>
          <Badge>
            <span className="font-mono tabular-nums">{blockers.length}</span>
          </Badge>
          <p className="text-xs text-ink-faint tabular-nums">
            {affectedChats} {unidentified ? 'identified ' : ''}affected chat
            {affectedChats === 1 ? '' : 's'}
          </p>
        </div>
        {unidentified ? (
          <p className="text-xs text-ink-faint">
            {unidentified} blocker{unidentified === 1 ? ' has' : 's have'} no
            session ID; affected chat count excludes these.
          </p>
        ) : null}
        {blockers.length ? (
          <>
            <SearchField
              aria-label="Filter deletion blockers"
              aria-describedby={resultsId}
              placeholder="Function, chat ID, call or blocker type"
              value={query}
              onKeyDown={(event) => {
                // Dialog's capture handler has already prevented dismissal.
                // Clear explicitly even when that native event is prevented.
                if (event.key === 'Escape' && query) {
                  event.stopPropagation()
                  setQuery('')
                  setPage(0)
                }
              }}
              onChange={(next) => {
                setQuery(next)
                setPage(0)
              }}
            />
            <p
              id={resultsId}
              className="text-xs text-ink-faint tabular-nums"
              aria-live="polite"
            >
              {query.trim()
                ? `${matches} of ${blockers.length} blockers match · ${groups.filter((group) => group.sessionId).length} identified chats match. `
                : ''}
              {matches
                ? `Showing ${page * DIAGNOSTIC_PAGE_SIZE + 1}–${Math.min((page + 1) * DIAGNOSTIC_PAGE_SIZE, matches)} of ${matches}${query.trim() ? ' matching' : ''} blockers.`
                : 'No matching blockers.'}
            </p>
            {matches ? (
              <ul aria-label="Deletion blockers" data-deletion-blocker-list="">
                {visibleGroups.map((group) => (
                  <li key={group.sessionId} data-deletion-session-group="">
                    <h4 className="flex min-w-0 items-center gap-2 text-xs text-ink-faint">
                      <span>Chat</span>
                      <span
                        className="min-w-0 flex-1 truncate font-mono"
                        title={group.sessionId}
                      >
                        {group.sessionId || 'Session ID not reported'}
                      </span>
                    </h4>
                    <p className="text-xs text-ink-faint tabular-nums">
                      {query.trim()
                        ? `${group.entries.length} shown here · ${group.total} total in chat`
                        : `${group.entries.length} of ${group.total} blockers shown here`}
                    </p>
                    {group.entries.map((entry) => (
                      <BlockerDetails
                        key={`${entry.index}:${JSON.stringify(entry.blocker)}`}
                        entry={entry}
                      />
                    ))}
                  </li>
                ))}
              </ul>
            ) : (
              <StatusPanel
                headline="No matching blockers"
                detail="Try another function, chat ID, call or blocker type. Filtering does not change deletion eligibility."
              />
            )}
            <Pagination
              page={page}
              total={matches}
              onChange={setPage}
              label="Blocker pages"
            />
          </>
        ) : (
          <StatusPanel
            headline="No blockers reported"
            detail="This does not confirm deletion or make force deletion eligible. Check the outcomes and error above."
          />
        )}
      </div>
    </section>
  )
}
