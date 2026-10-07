// The status of a suggestion: the badge that is also the menu that moves it,
// the three dialogs that ask for a fact first (shipped, rejected, duplicate),
// and the history of who changed it and when. Only people move a suggestion.
import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  Input,
  Select,
  StatusPanel,
  uiClasses,
} from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { ChevronDown, CircleAlert, History, LoaderCircle } from 'lucide-react'
import { useId, useState } from 'react'
import type { EvalApi } from '../../../api'
import type { Lifecycle, LifecycleStatus, ReviewChange, SuggestionReview } from '../../../types'
import { Pill } from './marks'
import {
  historyRows,
  lifecycleBadge,
  moveRefusal,
  OTHER_REASON,
  parseDuplicateOf,
  parsePr,
  parseVersion,
  REJECT_REASONS,
  STATUS_CHOICES,
} from './review-model'
import { DialogFrame, FieldLine, FormActions, FormField, TextArea } from './review-parts'

type Asked = 'shipped' | 'rejected' | 'duplicate'

// A menu item that opens a dialog waits one tick, so the menu has returned
// focus before the dialog takes it.
const afterMenuClose = (run: () => void) => () => {
  window.setTimeout(run, 0)
}

const DIALOG: Record<Asked, { title: (n: number) => string; submit: string }> = {
  shipped: { title: (n) => `Mark S${n} as shipped`, submit: 'Mark as shipped' },
  rejected: { title: (n) => `Reject S${n}`, submit: 'Reject suggestion' },
  duplicate: { title: (n) => `Mark S${n} as a duplicate`, submit: 'Mark as duplicate' },
}

export interface Target {
  api: EvalApi
  evaluationId: string
  index: number
  title: string
}

function TransitionForm({
  target,
  kind,
  lifecycle,
  sheet,
  narrow,
  onSaved,
  onCancel,
}: {
  target: Target
  kind: Asked
  lifecycle: Lifecycle
  sheet: boolean
  narrow: boolean
  onSaved: (row: SuggestionReview) => void
  onCancel: () => void
}) {
  const uid = useId()
  const [pr, setPr] = useState(lifecycle.pr ?? '')
  const [version, setVersion] = useState(lifecycle.version ?? '')
  const [reason, setReason] = useState<string>(REJECT_REASONS[0])
  const [note, setNote] = useState('')
  const [duplicateOf, setDuplicateOf] = useState('')
  const [saving, setSaving] = useState(false)
  const [failure, setFailure] = useState<string | null>(null)

  const prValue = parsePr(pr)
  const versionValue = version.trim() === '' ? undefined : parseVersion(version)
  const duplicateValue = parseDuplicateOf(duplicateOf, { evaluationId: target.evaluationId, index: target.index })
  const noteMissing = kind === 'rejected' && reason === OTHER_REASON && note.trim() === ''

  const change = (): ReviewChange | undefined => {
    switch (kind) {
      case 'shipped':
        if (!prValue || (version.trim() !== '' && !versionValue)) return undefined
        return { action: 'set_lifecycle', status: 'shipped', pr: prValue, version: versionValue }
      case 'rejected':
        if (noteMissing) return undefined
        return { action: 'set_lifecycle', status: 'rejected', reason, note: note.trim() || undefined }
      case 'duplicate':
        return duplicateValue
          ? { action: 'set_lifecycle', status: 'duplicate', duplicate_of: duplicateValue }
          : undefined
    }
  }
  const ready = change()

  const submit = async () => {
    if (!ready || saving) return
    setSaving(true)
    setFailure(null)
    try {
      onSaved(await target.api.review(target.evaluationId, target.index, ready))
    } catch (cause) {
      setFailure(errorMessage(cause))
      setSaving(false)
    }
  }

  const size = narrow ? 'lg' : 'sm'
  const wrong = (text: string) => (
    <FieldLine tone="alert" role="alert" icon={<CircleAlert className={uiClasses.icon} aria-hidden />}>
      {text}
    </FieldLine>
  )

  return (
    <form
      className="eval-ui-val-form"
      data-narrow={narrow || undefined}
      noValidate
      onSubmit={(event) => {
        event.preventDefault()
        void submit()
      }}
    >
      <div className="eval-ui-val-fields">
        {kind === 'shipped' ? (
          <>
            <FormField
              id={`${uid}-pr`}
              label="Pull request"
              hint="number or URL"
              line={
                pr.trim() !== '' && !prValue
                  ? wrong('A number like 1292, #1292, owner/repo#1292 or a pull request URL.')
                  : null
              }
            >
              <Input
                id={`${uid}-pr`}
                value={pr}
                onChange={setPr}
                placeholder="#1292"
                disabled={saving}
                aria-invalid={pr.trim() !== '' && !prValue ? true : undefined}
              />
            </FormField>
            <FormField
              id={`${uid}-version`}
              label="Harness version"
              hint="optional"
              help="Turns on the After release panel: the signal per analysis before and from this version."
              line={version.trim() !== '' && !versionValue ? wrong('A semantic version like 1.8.44.') : null}
            >
              <Input
                id={`${uid}-version`}
                value={version}
                onChange={setVersion}
                placeholder="1.8.44"
                disabled={saving}
                aria-invalid={version.trim() !== '' && !versionValue ? true : undefined}
              />
            </FormField>
          </>
        ) : null}
        {kind === 'rejected' ? (
          <>
            <FormField id={`${uid}-reason`} label="Reason">
              <Select
                id={`${uid}-reason`}
                value={reason}
                onChange={setReason}
                options={REJECT_REASONS.map((value) => ({ value, label: value }))}
                disabled={saving}
              />
            </FormField>
            <FormField id={`${uid}-note`} label="Note" hint="optional, required for Other">
              <TextArea id={`${uid}-note`} value={note} onChange={setNote} disabled={saving} maxLength={500} />
            </FormField>
          </>
        ) : null}
        {kind === 'duplicate' ? (
          <FormField
            id={`${uid}-of`}
            label="Duplicate of"
            hint="a PR or another suggestion"
            line={
              duplicateOf.trim() !== '' && !duplicateValue
                ? wrong('A pull request (#1292, a URL) or a suggestion of an analysis (S2, eval_… S2), not this one.')
                : null
            }
          >
            <Input
              id={`${uid}-of`}
              value={duplicateOf}
              onChange={setDuplicateOf}
              placeholder="#1292 or S2"
              disabled={saving}
              aria-invalid={duplicateOf.trim() !== '' && !duplicateValue ? true : undefined}
            />
          </FormField>
        ) : null}
        {failure ? (
          <StatusPanel
            variant="alert"
            role="alert"
            icon={<CircleAlert className={uiClasses.icon} aria-hidden />}
            headline="That did not go through"
            detail={failure}
          />
        ) : null}
      </div>
      <FormActions
        sheet={sheet}
        narrow={narrow}
        cancel={
          <Button type="button" variant={sheet ? 'pill' : 'ghost'} size={size} disabled={saving} onClick={onCancel}>
            Cancel
          </Button>
        }
        submit={
          <Button
            type="submit"
            variant="primary"
            size={size}
            disabled={!ready || saving}
            aria-busy={saving || undefined}
          >
            {saving ? (
              <>
                <LoaderCircle className={`${uiClasses.icon} ${uiClasses.spin}`} aria-hidden />
                Saving…
              </>
            ) : (
              DIALOG[kind].submit
            )}
          </Button>
        }
      />
    </form>
  )
}

/** The dialog that asks for the fact a change needs; it closes itself when the change is saved. */
export function TransitionDialog({
  target,
  kind,
  lifecycle,
  narrow,
  onClose,
  onSaved,
}: {
  target: Target
  kind: Asked
  lifecycle: Lifecycle
  narrow: boolean
  onClose: () => void
  onSaved: (row: SuggestionReview) => void
}) {
  return (
    <DialogFrame
      open
      onOpenChange={(open) => !open && onClose()}
      title={DIALOG[kind].title(target.index + 1)}
      description={target.title}
      narrow={narrow}
    >
      {({ sheet }) => (
        <TransitionForm
          target={target}
          kind={kind}
          lifecycle={lifecycle}
          sheet={sheet}
          narrow={narrow}
          onSaved={(row) => {
            onClose()
            onSaved(row)
          }}
          onCancel={onClose}
        />
      )}
    </DialogFrame>
  )
}

function HistoryList({ lifecycle }: { lifecycle: Lifecycle }) {
  const rows = historyRows(lifecycle)
  return (
    <ol className="eval-ui-rv-history">
      {rows.map((row, index) => (
        <li key={index}>
          <span className="eval-ui-rv-history-what">{row.label}</span>
          <span className="eval-ui-val-quiet eval-ui-val-small">
            {row.by} · {row.at}
          </span>
          {row.note ? <span className="eval-ui-val-small">{row.note}</span> : null}
        </li>
      ))}
    </ol>
  )
}

export function LifecycleMenu({
  target,
  lifecycle,
  narrow,
  onSaved,
  onError,
}: {
  target: Target
  lifecycle: Lifecycle
  narrow: boolean
  onSaved: (row: SuggestionReview) => void
  onError: (message: string) => void
}) {
  const [asked, setAsked] = useState<Asked | 'history' | null>(null)
  const [busy, setBusy] = useState(false)
  const badge = lifecycleBadge(lifecycle)
  const n = target.index + 1

  const choose = async (status: Exclude<LifecycleStatus, 'new'>) => {
    if (busy) return
    setBusy(true)
    try {
      onSaved(await target.api.review(target.evaluationId, target.index, { action: 'set_lifecycle', status }))
    } catch (cause) {
      onError(errorMessage(cause))
    } finally {
      setBusy(false)
    }
  }

  const pick = (status: LifecycleStatus, asks: boolean) => () => {
    if (status === 'new') return
    if (asks) afterMenuClose(() => setAsked(status as Asked))()
    else if (status !== lifecycle.status) void choose(status)
  }

  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <button
            type="button"
            className="eval-ui-rv-status"
            aria-label={`Status of S${n}: ${badge.label}. Change status`}
            aria-busy={busy || undefined}
          >
            <Pill tone={badge.tone} strong={badge.strong}>
              {badge.label}
              {busy ? (
                <LoaderCircle size={16} className={uiClasses.spin} aria-hidden />
              ) : (
                <ChevronDown size={16} aria-hidden />
              )}
            </Pill>
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" className="eval-ui-rv-menu">
          <DropdownMenuRadioGroup value={lifecycle.status}>
            {STATUS_CHOICES.map((choice) => {
              const refusal = moveRefusal(lifecycle.status, choice.status)
              return (
                <DropdownMenuRadioItem
                  key={choice.status}
                  value={choice.status}
                  disabled={busy || refusal !== undefined}
                  onSelect={pick(choice.status, choice.asks)}
                >
                  <span className="eval-ui-rv-menu-item">
                    <span>{choice.label}</span>
                    <span className="eval-ui-rv-menu-hint">{refusal ?? choice.hint}</span>
                  </span>
                </DropdownMenuRadioItem>
              )
            })}
          </DropdownMenuRadioGroup>
          {lifecycle.history.length > 0 ? (
            <>
              <DropdownMenuSeparator />
              <DropdownMenuItem onSelect={afterMenuClose(() => setAsked('history'))}>
                <History size={16} aria-hidden="true" />
                <span className="eval-ui-rv-menu-item">
                  <span>
                    History · {lifecycle.history.length} {lifecycle.history.length === 1 ? 'change' : 'changes'}
                  </span>
                  <span className="eval-ui-rv-menu-hint">Who changed it, and when</span>
                </span>
              </DropdownMenuItem>
            </>
          ) : null}
        </DropdownMenuContent>
      </DropdownMenu>
      {asked === 'history' ? (
        <DialogFrame
          open
          onOpenChange={() => setAsked(null)}
          title={`History of S${n}`}
          description={target.title}
          narrow={narrow}
        >
          {() => <HistoryList lifecycle={lifecycle} />}
        </DialogFrame>
      ) : asked ? (
        <TransitionDialog
          target={target}
          kind={asked}
          lifecycle={lifecycle}
          narrow={narrow}
          onClose={() => setAsked(null)}
          onSaved={onSaved}
        />
      ) : null}
    </>
  )
}
