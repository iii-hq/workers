// The top of the detail: id, status, origin, title, where it came from, and
// what can be done with it. Narrow: a labelled Back and one "More actions" menu.
import {
  Button,
  Chip,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  IconButton,
} from '@iii-dev/console-ui'
import { Ban, ChevronLeft, Ellipsis, ExternalLink, RefreshCw, Trash2 } from 'lucide-react'
import { isActive, statusPresentation } from '../../../model'
import type { AnalysisRecord, Snapshot } from '../../../types'
import { Pill } from './marks'
import { canReanalyze } from './notices'
import { metaRest } from './present'
import type { DetailActions } from './shared'

/** The pill words the monitor's work: `Completed · 1 suggestion`. */
function statusLabel(record: AnalysisRecord): { label: string; tone: ReturnType<typeof statusPresentation>['tone'] } {
  const status = statusPresentation(record)
  return record.status === 'completed'
    ? { label: `Completed · ${status.label.toLowerCase()}`, tone: status.tone }
    : status
}

// A menu item that opens a dialog waits one tick, so the menu has returned
// focus before the dialog takes it.
const afterMenuClose = (run: () => void) => () => {
  window.setTimeout(run, 0)
}

export function Masthead({
  record,
  snapshot,
  narrow,
  actions,
  onBack,
}: {
  record: AnalysisRecord
  snapshot: Snapshot | undefined
  narrow: boolean
  actions: DetailActions
  onBack?: () => void
}) {
  const active = isActive(record.status)
  const status = statusLabel(record)
  const busy = actions.busy !== null
  const size = narrow ? 'lg' : 'sm'
  const again = canReanalyze(record)

  const identity = (
    <div className="eval-ui-ad-ident">
      <span className="eval-ui-ad-mono-quiet eval-ui-ad-id">{record.evaluation_id}</span>
      <Pill tone={status.tone} pulse={active && status.tone === 'accent'}>
        {status.label}
      </Pill>
      <Chip>{record.origin === 'automatic' ? 'Automatic' : 'Manual'}</Chip>
    </div>
  )

  const openSession = actions.openSession
  const title = (
    <>
      <h1 className="eval-ui-ad-title">{record.source_title ?? record.session_id}</h1>
      <p className="eval-ui-ad-meta">
        {openSession ? (
          <button
            type="button"
            className="eval-ui-ad-meta-link"
            aria-label={`Open session ${record.session_id}`}
            onClick={openSession}
          >
            {record.session_id}
          </button>
        ) : (
          record.session_id
        )}
        {' · '}
        {metaRest(record, snapshot)}
      </p>
    </>
  )

  if (narrow) {
    return (
      <header className="eval-ui-ad-masthead" data-narrow="true">
        {onBack ? (
          <Button variant="ghost" size="lg" className="eval-ui-ad-back" onClick={onBack}>
            <ChevronLeft size={16} aria-hidden="true" />
            Back to the list
          </Button>
        ) : null}
        <div className="eval-ui-ad-ident-row">
          <span className="eval-ui-ad-mono-quiet eval-ui-ad-id">{record.evaluation_id}</span>
          <div className="eval-ui-ad-ident-actions">
            {openSession ? (
              <Button variant="ghost" size="lg" onClick={openSession}>
                Open session
                <ExternalLink size={16} aria-hidden="true" />
              </Button>
            ) : null}
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <IconButton label="More actions" className="eval-ui-ad-more">
                  <Ellipsis size={16} aria-hidden="true" />
                </IconButton>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                {active ? (
                  <DropdownMenuItem disabled={busy} onSelect={actions.cancel}>
                    <Ban size={16} aria-hidden="true" />
                    Cancel analysis
                  </DropdownMenuItem>
                ) : again ? (
                  <DropdownMenuItem disabled={busy} onSelect={actions.reanalyze}>
                    <RefreshCw size={16} aria-hidden="true" />
                    Reanalyze
                  </DropdownMenuItem>
                ) : null}
                {active ? null : (
                  <>
                    {again ? <DropdownMenuSeparator /> : null}
                    <DropdownMenuItem disabled={busy} onSelect={afterMenuClose(actions.remove)}>
                      <Trash2 size={16} aria-hidden="true" />
                      Delete analysis
                    </DropdownMenuItem>
                  </>
                )}
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
        </div>
        <div className="eval-ui-ad-ident">
          <Pill tone={status.tone} pulse={active && status.tone === 'accent'}>
            {status.label}
          </Pill>
          <Chip>{record.origin === 'automatic' ? 'Automatic' : 'Manual'}</Chip>
        </div>
        {title}
      </header>
    )
  }

  return (
    <header className="eval-ui-ad-masthead">
      <div className="eval-ui-ad-masthead-copy">
        {identity}
        {title}
      </div>
      <div className="eval-ui-ad-actions">
        {actions.openSession ? (
          <Button variant="ghost" size={size} onClick={actions.openSession}>
            Open session
            <ExternalLink size={16} aria-hidden="true" />
          </Button>
        ) : null}
        {active ? (
          <Button variant="pill" size={size} disabled={busy} onClick={actions.cancel}>
            Cancel analysis
          </Button>
        ) : (
          <>
            {again ? (
              <Button variant="pill" size={size} disabled={busy} onClick={actions.reanalyze}>
                Reanalyze
              </Button>
            ) : null}
            <IconButton label="Delete analysis" disabled={busy} onClick={actions.remove}>
              <Trash2 size={16} aria-hidden="true" />
            </IconButton>
          </>
        )}
      </div>
    </header>
  )
}
