import { copyText, errorMessage } from '@iii-dev/console-ui/format'
import { Check, Copy, FileText, Plus, RotateCw, Undo2, X } from 'lucide-react'
import { useEffect, useId, useRef, useState } from 'react'
import { Button } from '@/components/ui/Button'
import { CodeEditor, type CodeEditorHandle } from '@/components/ui/CodeEditor'
import { FileDiff } from '@/components/ui/FileDiff'
import { IconButton } from '@/components/ui/IconButton'
import { Input } from '@/components/ui/Input'
import { KeyCombo } from '@/components/ui/KeyCombo'
import { Selector } from '@/components/ui/Selector'
import {
  SettingsField,
  SettingsList,
  SettingsRow,
  SettingsSection,
} from '@/components/ui/Settings'
import { Skeleton } from '@/components/ui/Skeleton'
import { StatusPanel, type StatusVariant } from '@/components/ui/StatusPanel'
import { Toolbar } from '@/components/ui/Toolbar'
import { shortcutPlatform } from '@/lib/keybindings/bindings'
import type { PageCommandsApi } from '@/types/injectable-ui'
import { WorkerConfigurationReference } from './components/WorkerConfigurationReference'
import type {
  ComposeApi,
  ContainerEntry,
  DeclaredContainer,
} from './compose-api'
import {
  captureDiscardedDraft,
  type DiscardedSettingsDraft,
  restoreDiscardedDraft,
} from './discarded-draft'
import { EnvironmentRow } from './environment-row'
import type { Actions } from './index'
import {
  alsoStarts,
  confirmStoredPatch,
  dependencyBlockReason,
  draftFrom,
  type EnvDraft,
  entryShape,
  entryYaml,
  hasSettingsChanges,
  restoreSettingsDraft,
  type SettingsDraft,
  settingsPatch,
  shortPath,
  submittedSettingsEntry,
  validateSettingsDraft,
} from './model'

const FILE = 'worker-compose.yaml'
const PREBUILT = /^\.\/target\/(debug|release)\//

function sentence(message: string) {
  const text = message.trim()
  return text && !/[.!?]$/.test(text) ? `${text}.` : text
}

function focusedSettingsElement() {
  let active = document.activeElement
  while (active?.shadowRoot?.activeElement)
    active = active.shadowRoot.activeElement
  return active instanceof HTMLElement ? active : null
}

function revealSettings(
  target: HTMLElement | null,
  block: 'start' | 'center',
  owner?: HTMLElement,
) {
  const pane = (owner ?? target)?.closest<HTMLElement>('.wk-main')
  if (!target || !pane) return
  const bounds = target.getBoundingClientRect()
  const viewport = pane.getBoundingClientRect()
  const style = getComputedStyle(target)
  const start =
    Number.parseFloat(style.scrollMarginBlockStart) ||
    (owner &&
      Number.parseFloat(
        getComputedStyle(owner).getPropertyValue('--wk-apply-height'),
      )) ||
    0
  const end = Number.parseFloat(style.scrollMarginBlockEnd) || 0
  const offset =
    start +
    (block === 'center'
      ? Math.max(0, (pane.clientHeight - start - end - bounds.height) / 2)
      : 0)
  pane.scrollTo({
    top: Math.max(0, pane.scrollTop + bounds.top - viewport.top - offset),
    behavior: 'instant',
  })
}

type SettingsProps = {
  api: ComposeApi
  revision: object
  actions: Actions
  name: string
  declared: DeclaredContainer | null
  others: string[]
  containers: readonly DeclaredContainer[]
  states: ReadonlyMap<string, string>
  neededBy: string[]
  compact?: boolean
  commands?: PageCommandsApi
  entry: ContainerEntry | null
  draft: SettingsDraft | null
  saving: boolean
  notice: SaveNotice | null
  previousEntry: ContainerEntry | null
  discardedDraft: DiscardedSettingsDraft | null
  awaitingReload: boolean
  onSubmitted: (entry: ContainerEntry) => void
  onPrevious: (entry: ContainerEntry) => void
  onDiscarded: (draft: DiscardedSettingsDraft | null) => void
  onSaving: (saving: boolean) => void
  onNotice: (notice: SaveNotice | null) => void
  onLoaded: (entry: ContainerEntry, preserveDraft?: boolean) => void
  onDraft: (draft: SettingsDraft) => void
  onOpenLog: () => void
  onRemove: () => void
}

export function SettingsTab(props: SettingsProps) {
  const { api, name, entry, draft, revision } = props
  const [error, setError] = useState<string | null>(null)
  const [attempt, setAttempt] = useState(0)
  const current = useRef(props)
  current.current = props
  // biome-ignore lint/correctness/useExhaustiveDependencies: Retry and a new runtime snapshot explicitly refresh clean declarations; the latest draft is guarded at completion.
  useEffect(() => {
    const canReplace = () => {
      const { entry, draft, saving, awaitingReload } = current.current
      return (
        !saving &&
        !awaitingReload &&
        !(entry && draft && hasSettingsChanges(entry, draft))
      )
    }
    if (!canReplace()) return
    let cancelled = false
    setError(null)
    api
      .container(name)
      .then((next) => {
        if (!cancelled && canReplace()) current.current.onLoaded(next)
      })
      .catch((cause) => {
        if (!cancelled && canReplace()) setError(errorMessage(cause))
      })
    return () => {
      cancelled = true
    }
  }, [api, name, attempt, revision])
  if (error)
    return (
      <div className="wk-settings">
        <StatusPanel
          className="wk-save-notice"
          role="alert"
          variant="alert"
          headline={`Settings for ${name} could not be loaded`}
          detail={`${sentence(error)} Retry to read the declaration.`}
          action={
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setAttempt((n) => n + 1)}
            >
              Retry
            </Button>
          }
        />
      </div>
    )
  if (!entry || !draft)
    return (
      <div
        className="wk-settings"
        role="status"
        aria-busy="true"
        aria-label={`Loading settings for ${name}`}
      >
        <span className="sr-only">Loading worker settings</span>
        <Skeleton className="wk-loading-apply" />
        <div className="wk-form" aria-hidden>
          {['command', 'dependencies', 'environment', 'override'].map(
            (section) => (
              <div key={section} className="wk-loading-section">
                <Skeleton className="wk-loading-title" />
                <Skeleton className="wk-loading-description" />
                <Skeleton className="wk-loading-field" data-section={section} />
              </div>
            ),
          )}
        </div>
      </div>
    )
  return <SettingsForm {...props} entry={entry} draft={draft} />
}

export type SaveNotice = {
  variant: StatusVariant
  headline: string
  detail: string
  retry?: 'restart' | 'reload'
  reloadAfterRestart?: boolean
}

function SettingsForm({
  api,
  actions,
  name,
  declared,
  others,
  containers,
  states,
  neededBy,
  compact = false,
  commands,
  entry,
  draft,
  saving,
  notice,
  previousEntry,
  discardedDraft,
  awaitingReload,
  onSubmitted,
  onPrevious,
  onDiscarded,
  onSaving: setSaving,
  onNotice: setNotice,
  onLoaded,
  onDraft,
  onOpenLog,
  onRemove,
}: SettingsProps & { entry: ContainerEntry; draft: SettingsDraft }) {
  const [previewOpen, setPreviewOpen] = useState(false)
  const [reviewRequest, setReviewRequest] = useState(0)
  const [copied, setCopied] = useState(false)
  const saveLock = useRef(false)
  const copyTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const applyCopy = useRef<HTMLDivElement>(null)
  const result = useRef<HTMLElement>(null)
  const review = useRef<HTMLElement>(null)
  const fields = useRef<HTMLFieldSetElement>(null)
  const configEditor = useRef<CodeEditorHandle>(null)
  const previousNotice = useRef(notice)
  const focusResult = useRef(false)
  const focusReview = useRef(false)
  const focusedReviewRequest = useRef(0)
  const keyboardFocus = useRef(true)
  const previewId = useId()
  const configErrorId = useId()
  const configHintId = useId()
  const validationSummaryId = useId()
  const validation = validateSettingsDraft(entry, draft)
  const configDescription = [
    configHintId,
    validation.config ? configErrorId : null,
  ]
    .filter(Boolean)
    .join(' ')
  const tabFocusKey = shortcutPlatform() === 'mac' ? 'Ctrl+Shift+M' : 'Ctrl+M'
  const dirty = hasSettingsChanges(entry, draft)
  const previewVisible = previewOpen && validation.valid && dirty
  const { patch, changes } = validation.valid
    ? settingsPatch(entry, draft)
    : { patch: {}, changes: 0 }
  const dependencyErrors = new Map<string, string>()
  for (const dependency of draft.startAfter) {
    const reason = containers.length
      ? containers.some((worker) => worker.name === dependency)
        ? dependencyBlockReason(containers, name, dependency)
        : `${dependency} is no longer in the project.`
      : patch.start_after
        ? 'The project declaration is unavailable. Refresh workers before changing dependencies.'
        : null
    if (reason) dependencyErrors.set(dependency, reason)
  }
  const errorCount =
    Object.keys(validation.env).length +
    Number(!!validation.config) +
    dependencyErrors.size
  const firstDependencyError = dependencyErrors.keys().next().value
  const environmentErrors = Object.keys(validation.env).map(Number)
  const firstEnvironmentIndex = environmentErrors[0]
  // Existing names are immutable; a duplicate is repaired on its new row.
  const firstEnvironmentError =
    firstEnvironmentIndex === undefined
      ? undefined
      : (environmentErrors.find(
          (index) =>
            draft.env[index].isNew &&
            draft.env[index].key === draft.env[firstEnvironmentIndex].key,
        ) ?? firstEnvironmentIndex)
  const firstError =
    firstDependencyError !== undefined
      ? `${sentence(dependencyErrors.get(firstDependencyError) ?? '')} Remove this dependency before saving.`
      : firstEnvironmentError !== undefined
        ? validation.env[firstEnvironmentError]
        : validation.config
  const invalidEnvironment =
    firstEnvironmentError === undefined
      ? undefined
      : draft.env[firstEnvironmentError]
  const invalidEnvironmentName =
    invalidEnvironment &&
    (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(invalidEnvironment.key) ||
      draft.env.some(
        (row, index) =>
          index !== firstEnvironmentError &&
          !row.removed &&
          row.key === invalidEnvironment.key,
      ))
  const fixFirstError = () => {
    if (busy) return
    if (
      firstDependencyError === undefined &&
      firstEnvironmentError === undefined &&
      validation.config
    ) {
      configEditor.current?.revealLine(
        validation.configPosition?.line ?? 1,
        validation.configPosition?.column ?? 1,
      )
      return
    }
    const row =
      firstDependencyError !== undefined
        ? fields.current?.querySelector<HTMLElement>('[data-dependency-error]')
        : fields.current?.querySelectorAll<HTMLElement>('.wk-env')[
            Number(firstEnvironmentError)
          ]
    const target =
      row?.querySelector<HTMLElement>(
        firstDependencyError !== undefined
          ? 'button'
          : invalidEnvironmentName
            ? 'input[aria-label="Variable name"][aria-invalid="true"]'
            : invalidEnvironment?.secret &&
                invalidEnvironment.replacing &&
                invalidEnvironment.value === ''
              ? 'input[type="password"][aria-invalid="true"]'
              : '[aria-invalid="true"]:not(:disabled)',
      ) ?? row?.querySelector<HTMLElement>('button[aria-label^="Remove "]')
    revealSettings(row ?? null, 'center')
    target?.focus({ preventScroll: true })
  }
  const busy = actions.busy || saving
  const canSave = dirty && validation.valid && !dependencyErrors.size && !busy
  const previous = previousEntry
    ? restoreSettingsDraft(entry, previousEntry)
    : null
  const canRestore = !!previous && hasSettingsChanges(entry, previous.draft)
  const undoState = useRef({ entry, draft, busy, discardedDraft })
  undoState.current = { entry, draft, busy, discardedDraft }
  const discardedNotice: SaveNotice = {
    variant: 'info',
    headline: 'Discarded draft available',
    detail:
      'Undo restores your last discarded edits for review. They remain available until a successful save.',
  }
  const set = (next: Partial<SettingsDraft>) => {
    if (!awaitingReload && !discardedDraft) setNotice(null)
    onDraft({ ...draft, ...next })
  }
  const setEnv = (index: number, next: Partial<EnvDraft>) =>
    set({
      env: draft.env.map((row, i) => (i === index ? { ...row, ...next } : row)),
    })
  const waitable = others.filter((other) => !draft.startAfter.includes(other))
  const discard = () => {
    if (busy || !dirty) return
    onDiscarded(captureDiscardedDraft(entry, draft))
    onDraft(draftFrom(entry))
    setNotice(
      awaitingReload
        ? {
            variant: 'warn',
            headline: 'Settings saved; awaiting verification',
            detail:
              'Your stored settings remain visible. Reload the saved declaration to verify them.',
            retry: 'reload',
          }
        : { ...discardedNotice, headline: 'Draft discarded' },
    )
    setPreviewOpen(false)
  }
  const undoDiscard = async () => {
    const snapshot = undoState.current.discardedDraft
    if (!snapshot || undoState.current.busy) return
    if (
      hasSettingsChanges(undoState.current.entry, undoState.current.draft) &&
      !(await actions.confirm({
        title: 'Replace the current draft?',
        description:
          'Your pending edits will be replaced by your last discarded draft. Saved settings stay unchanged until you save and restart.',
        confirmLabel: 'Undo discard',
      }))
    )
      return
    const latest = undoState.current
    if (latest.busy || latest.discardedDraft !== snapshot) return
    const restored = restoreDiscardedDraft(snapshot, latest.entry)
    const valid = validateSettingsDraft(latest.entry, restored).valid
    focusReview.current = valid
    focusResult.current = !valid
    onDraft(restored)
    onDiscarded(null)
    setPreviewOpen(valid)
    setNotice({
      variant: 'info',
      headline: 'Discard undone',
      detail: valid
        ? 'Your discarded edits are pending again. Review the changes, then Save and restart to apply them.'
        : 'Your discarded edits are pending again. Fix the marked fields before reviewing or saving them.',
      retry: awaitingReload ? 'reload' : undefined,
    })
  }
  const restore = async () => {
    if (!previous || !canRestore || busy) return
    if (
      dirty &&
      !(await actions.confirm({
        title: 'Replace the current draft?',
        description:
          'Your pending edits will be replaced by the previous settings. The saved declaration stays unchanged until you save and restart.',
        confirmLabel: 'Restore draft',
      }))
    )
      return
    focusReview.current = true
    onDraft(previous.draft)
    setPreviewOpen(true)
    const secretsRemain =
      previous.unavailableSecrets.length > 0 ||
      entry.environment.some((variable) => variable.secret)
    setNotice({
      variant: 'info',
      headline: 'Previous settings restored to draft',
      retry: awaitingReload ? 'reload' : undefined,
      detail: `Review the changes, then Save and restart to apply them.${secretsRemain ? ' Saved secrets stay as they are now; previous values cannot be recovered.' : ''}${previous.unavailableSecrets.length ? ` Re-enter previous secrets if needed: ${previous.unavailableSecrets.join(', ')}.` : ''}`,
    })
  }
  const reviewChanges = () => {
    if (!dirty || !validation.valid || busy) return
    setPreviewOpen(true)
    setReviewRequest((request) => request + 1)
  }
  useEffect(() => {
    const bar = applyCopy.current?.closest<HTMLElement>('.wk-applybar')
    const sheet = bar?.parentElement
    if (!bar || !sheet) return
    const resize = () =>
      sheet.style.setProperty('--wk-apply-height', `${bar.offsetHeight + 24}px`)
    resize()
    const observer = new ResizeObserver(resize)
    observer.observe(bar)
    return () => observer.disconnect()
  }, [])
  useEffect(() => {
    if (saving) return
    if (notice && notice !== previousNotice.current) {
      const destination = focusReview.current
        ? (review.current ?? result.current)
        : result.current
      revealSettings(
        destination,
        destination === review.current ? 'start' : 'center',
      )
      if (focusResult.current || focusReview.current)
        destination?.focus({ preventScroll: true })
      focusResult.current = false
      focusReview.current = false
    }
    previousNotice.current = notice
  }, [notice, saving])
  useEffect(() => {
    if (previewVisible) {
      revealSettings(review.current, 'start')
      if (reviewRequest !== focusedReviewRequest.current) {
        review.current?.focus({ preventScroll: true })
        focusedReviewRequest.current = reviewRequest
      }
    }
  }, [previewVisible, reviewRequest])
  const begin = () => {
    focusResult.current = true
    saveLock.current = true
    setSaving(true)
  }
  useEffect(
    () => () => {
      if (copyTimer.current) clearTimeout(copyTimer.current)
    },
    [],
  )

  const reload = async () => {
    if (saveLock.current || busy) return
    begin()
    try {
      onLoaded(await api.container(name), true)
      setNotice({
        variant: 'success',
        headline: 'Saved settings reloaded',
        detail: dirty
          ? `The saved declaration was read from ${FILE}. Your newer edits remain in the draft.`
          : `The form now matches ${FILE}.`,
      })
    } catch (cause) {
      setNotice({
        variant: 'alert',
        headline: 'Saved settings could not be reloaded',
        detail: `${sentence(errorMessage(cause))} Your draft is retained.`,
        retry: 'reload',
      })
    } finally {
      saveLock.current = false
      setSaving(false)
    }
  }
  const retryRestart = async () => {
    if (saveLock.current || actions.busy) return
    begin()
    try {
      const restarted = await actions.run(`Restarting ${name}`, () =>
        api.lifecycle('restart', name),
      )
      if (restarted && notice?.reloadAfterRestart) {
        try {
          onLoaded(await api.container(name), true)
        } catch (cause) {
          setNotice({
            variant: 'warn',
            headline: 'Restart completed; saved settings could not be reloaded',
            detail: `${sentence(errorMessage(cause))} Your draft is retained until the declaration can be read.`,
            retry: 'reload',
          })
          return
        }
      }
      setNotice(
        restarted
          ? {
              variant: 'success',
              headline: `${name} restarted`,
              detail:
                'The restart completed. Check the current worker state above.',
            }
          : {
              variant: 'alert',
              headline: `${name} could not be restarted`,
              detail: `Open Log to diagnose ${name}, then retry its restart.`,
              retry: 'restart',
              reloadAfterRestart: notice?.reloadAfterRestart,
            },
      )
    } finally {
      saveLock.current = false
      setSaving(false)
    }
  }
  const save = async () => {
    if (!canSave || saveLock.current) return
    begin()
    let written = false
    try {
      const also = alsoStarts(actions.idle, [name])
      if (
        also.length &&
        !(await actions.confirm({
          title: `Save ${name}?`,
          description: `Compose rewrites the declaration and restarts ${name}. These stopped workers also start.`,
          details: also,
          confirmLabel: 'Save and restart',
        }))
      ) {
        focusResult.current = false
        return
      }
      setNotice(null)
      let failureDetail: string | undefined
      const applied = await actions.track(
        `Saving ${name}`,
        (operationId) => api.edit(name, patch, operationId),
        (detail) => {
          failureDetail = detail
        },
        name,
      )
      if (!applied) {
        const failure = failureDetail ? `${sentence(failureDetail)} ` : ''
        try {
          const stored = await api.container(name)
          const confirmed = confirmStoredPatch(stored, patch)
          if (confirmed === true) {
            onPrevious(entry)
            onDiscarded(null)
            onLoaded(stored)
            setNotice({
              variant: 'alert',
              headline: 'Configuration saved; restart failed',
              detail: `${failure}The declaration in ${FILE} matches your edits. Open Log to diagnose ${name}, then retry its restart.`,
              retry: 'restart',
            })
          } else if (confirmed === null) {
            setNotice({
              variant: 'warn',
              headline: 'Operation failed; verify the saved secrets',
              detail: `${failure}The readable settings match your edits. Secret values are write-only, so their replacement cannot be confirmed. Your draft is retained; verify the saved secrets before saving again or retrying the restart.`,
              retry: 'restart',
            })
          } else {
            setNotice({
              variant: 'alert',
              headline: 'The save did not complete',
              detail: `${failure}The declaration does not match your edits. Your draft is retained. Resolve the error, then try Save and restart again.`,
            })
          }
        } catch (cause) {
          setNotice({
            variant: 'alert',
            headline: 'Operation failed; saved settings could not be verified',
            detail: `${failure}${sentence(errorMessage(cause))} Your draft is retained. Reload the declaration to check what was written before trying again.`,
            retry: 'reload',
          })
        }
        return
      }
      written = true
      onPrevious(entry)
      onSubmitted(submittedSettingsEntry(entry, patch))
      const restarted =
        declared?.source !== 'package' ||
        (await actions.run(`Restarting ${name}`, () =>
          api.lifecycle('restart', name),
        ))
      const [declaration, runtime] = await Promise.allSettled([
        api.container(name),
        api.snapshot(),
      ])
      if (declaration.status === 'fulfilled') onLoaded(declaration.value)
      const state =
        runtime.status === 'fulfilled'
          ? runtime.value.status.containers?.find(
              (worker) => worker.container === name,
            )?.state
          : undefined
      setNotice(
        !restarted
          ? {
              variant: 'alert',
              headline: 'Configuration saved; restart failed',
              detail: `Your settings are stored in ${FILE}. Open Log or retry the restart.${declaration.status === 'rejected' ? ' The declaration could not be reloaded; your draft is retained until it can be read.' : ''}`,
              retry: 'restart',
              reloadAfterRestart: declaration.status === 'rejected',
            }
          : declaration.status === 'rejected'
            ? {
                variant: 'warn',
                headline: 'Settings saved; declaration could not be reloaded',
                detail: `${sentence(errorMessage(declaration.reason))} The draft stays visible until the saved declaration can be read.`,
                retry: 'reload',
              }
            : {
                variant:
                  state === 'failed' || state === 'stopped'
                    ? 'warn'
                    : 'success',
                headline: `Settings saved for ${name}`,
                detail: state
                  ? `Configuration written and restart completed. Current worker state: ${state}.`
                  : 'Configuration written and restart completed. Runtime state could not be refreshed; use Refresh workers from the command palette.',
              },
      )
      setPreviewOpen(false)
    } catch (cause) {
      setNotice({
        variant: 'alert',
        headline: written
          ? 'Configuration saved; follow-up failed'
          : 'Settings could not be saved',
        detail: `${sentence(errorMessage(cause))} Your draft is retained.`,
        retry: written ? 'reload' : undefined,
      })
    } finally {
      saveLock.current = false
      setSaving(false)
    }
  }

  const currentCommands = {
    save,
    discard,
    reviewChanges,
    restore,
    undoDiscard,
    canSave,
    canDiscard: dirty && !busy,
    canReview: dirty && validation.valid && !busy,
    canRestore: canRestore && !busy,
    canUndoDiscard: !!discardedDraft && !busy,
  }
  const command = useRef(currentCommands)
  command.current = currentCommands
  useEffect(
    () =>
      commands?.register([
        {
          id: 'save-settings',
          title: 'Save worker settings and restart',
          detail: `Apply the pending declaration for ${name}`,
          shortcut: 'Mod+S',
          firesWhileTyping: true,
          enabled: () => command.current.canSave,
          run: () => {
            void command.current.save()
          },
        },
        {
          id: 'review-settings',
          title: 'Review worker settings changes',
          detail: `Preview the declaration changes for ${name}`,
          keywords: ['preview', 'diff', 'compose'],
          enabled: () => command.current.canReview,
          run: () => command.current.reviewChanges(),
        },
        {
          id: 'restore-settings',
          title: 'Restore previous worker settings',
          detail: `Prepare the previous readable settings for ${name}; saved secrets stay current`,
          keywords: ['undo', 'history', 'previous'],
          enabled: () => command.current.canRestore,
          run: () => {
            void command.current.restore()
          },
        },
        {
          id: 'discard-settings',
          title: 'Discard worker settings changes',
          detail: `Restore the last loaded declaration for ${name}`,
          enabled: () => command.current.canDiscard,
          run: () => command.current.discard(),
        },
        {
          id: 'undo-discard',
          title: 'Undo discarded worker draft',
          detail: `Recover the last discarded edits for ${name} without applying them`,
          keywords: ['undo', 'discard', 'recover'],
          enabled: () => command.current.canUndoDiscard,
          run: () => {
            void command.current.undoDiscard()
          },
        },
      ]),
    [commands, name],
  )

  const previousDescription = canRestore
    ? 'Restore the settings from before your last save as a draft. Review them before applying.'
    : 'Previous readable settings match the last loaded declaration. Saved secrets stay as they are now; previous values cannot be recovered.'
  const restoreAction = (
    <Button
      variant="ghost"
      size="sm"
      disabled={!canRestore || busy}
      onClick={() => void restore()}
    >
      <Undo2 />
      Restore previous settings
    </Button>
  )

  return (
    // Reveal keyboard focus without interfering with native pointer clicks.
    <div
      className="wk-settings"
      onPointerDownCapture={() => {
        keyboardFocus.current = false
      }}
      onKeyDownCapture={(event) => {
        if (event.key === 'Tab') keyboardFocus.current = true
      }}
      onFocusCapture={(event) => {
        if (!keyboardFocus.current) return
        const owner = event.currentTarget
        if ((event.target as HTMLElement).closest('.wk-applybar')) return
        const target = focusedSettingsElement()
        if (!target) return
        const reveal = () => {
          if (!target.isConnected || focusedSettingsElement() !== target) return
          const bar = applyCopy.current?.closest('.wk-applybar')
          if (
            bar &&
            target.getBoundingClientRect().top <
              bar.getBoundingClientRect().bottom + 8
          )
            revealSettings(target, 'center', owner)
        }
        // Monaco positions its input at the clicked cursor after focus.
        if (target.closest('[data-keybindings-standdown]'))
          window.requestAnimationFrame(reveal)
        else reveal()
      }}
    >
      <Toolbar
        as="section"
        role="region"
        aria-label="Apply worker settings"
        className="wk-applybar"
        end={
          <>
            <Button
              variant="ghost"
              size="sm"
              disabled={!dirty || !validation.valid || busy}
              aria-label={previewVisible ? 'Hide preview' : 'Preview changes'}
              aria-expanded={previewVisible}
              aria-controls={previewId}
              onClick={() => setPreviewOpen(!previewVisible)}
            >
              <span className="wk-preview-long" aria-hidden>
                {previewVisible ? 'Hide preview' : 'Preview changes'}
              </span>
              <span className="wk-preview-short" aria-hidden>
                {previewVisible ? 'Hide' : 'Preview'}
              </span>
            </Button>
            <Button
              variant="ghost"
              size="sm"
              disabled={!dirty || busy}
              onClick={discard}
            >
              Discard
            </Button>
            <Button
              variant="primary"
              size="sm"
              disabled={!canSave}
              onClick={() => void save()}
              aria-label={saving ? 'Saving settings' : 'Save and restart'}
              aria-keyshortcuts="Control+S Meta+S"
            >
              {saving ? 'Saving…' : 'Save and restart'}
              {commands ? (
                <span className="wk-save-shortcut" aria-hidden>
                  <KeyCombo
                    binding="Mod+S"
                    capClassName="bg-transparent text-bg"
                    className="[&>span]:text-bg"
                  />
                </span>
              ) : null}
            </Button>
          </>
        }
      >
        <div className="wk-apply-copy" ref={applyCopy}>
          <span className="wk-apply-state" role="status" aria-live="polite">
            {errorCount ? (
              <Button
                variant="ghost"
                size="sm"
                className="wk-validation-jump"
                aria-describedby={validationSummaryId}
                disabled={busy}
                onClick={fixFirstError}
              >
                Fix {errorCount} field{errorCount === 1 ? '' : 's'} before
                saving
              </Button>
            ) : dirty ? (
              `${changes} pending change${changes === 1 ? '' : 's'}`
            ) : awaitingReload ? (
              'Saved; awaiting verification'
            ) : (
              'No pending changes'
            )}
          </span>
          {firstError ? (
            <p id={validationSummaryId} className="wk-field-error">
              {firstError}
            </p>
          ) : null}
          <p className="wk-note">
            {awaitingReload && dirty
              ? 'Earlier settings are saved and awaiting verification. '
              : null}
            Saving restarts <span className="wk-mono">{name}</span>.
            {neededBy.length
              ? ` ${neededBy.join(', ')} ${neededBy.length === 1 ? 'depends' : 'depend'} on it and may be affected while it reconnects.`
              : ' No other worker depends on it.'}
          </p>
        </div>
      </Toolbar>
      {notice ? (
        <section
          ref={result}
          className="wk-result"
          aria-label="Worker settings result"
          tabIndex={-1}
        >
          <StatusPanel
            className="wk-save-notice"
            role={
              notice.variant === 'alert' || notice.variant === 'warn'
                ? 'alert'
                : 'status'
            }
            variant={notice.variant}
            headline={notice.headline}
            detail={notice.detail}
            action={
              notice.retry || discardedDraft ? (
                <>
                  {discardedDraft ? (
                    <Button
                      variant="ghost"
                      size="sm"
                      disabled={busy}
                      onClick={() => void undoDiscard()}
                    >
                      <Undo2 />
                      Undo discard
                    </Button>
                  ) : null}
                  {notice.retry === 'restart' ? (
                    <Button
                      variant="ghost"
                      size="sm"
                      disabled={busy}
                      onClick={onOpenLog}
                    >
                      <FileText />
                      Open Log
                    </Button>
                  ) : null}
                  {notice.retry ? (
                    <Button
                      variant="ghost"
                      size="sm"
                      disabled={busy}
                      onClick={() =>
                        void (notice.retry === 'restart'
                          ? retryRestart()
                          : reload())
                      }
                    >
                      <RotateCw />
                      {notice.retry === 'restart'
                        ? 'Retry restart'
                        : 'Reload saved settings'}
                    </Button>
                  ) : null}
                </>
              ) : undefined
            }
          />
        </section>
      ) : null}
      {previous && compact ? (
        <details key={name} className="wk-previous-settings">
          <summary>Previous saved settings</summary>
          <div className="wk-worker-context-body">
            <p className="wk-note">{previousDescription}</p>
            {restoreAction}
          </div>
        </details>
      ) : previous ? (
        <SettingsList>
          <SettingsRow
            label="Previous saved settings"
            description={previousDescription}
            action={restoreAction}
          />
        </SettingsList>
      ) : null}
      {previewVisible ? (
        <section
          ref={review}
          id={previewId}
          className="wk-review"
          aria-label="Review pending changes"
          tabIndex={-1}
        >
          <p className="wk-note">
            Review the change to <span className="wk-mono">{FILE}</span> before
            applying it.
          </p>
          {previous &&
          (previous.unavailableSecrets.length > 0 ||
            entry.environment.some((variable) => variable.secret)) ? (
            <>
              <p className="wk-note">
                Saved secrets stay as they are now unless explicitly replaced;
                previous values cannot be recovered.
              </p>
              {previous.unavailableSecrets.length > 0 ? (
                <p className="wk-note">
                  Re-enter previous secrets if needed:{' '}
                  {previous.unavailableSecrets.join(', ')}.
                </p>
              ) : null}
            </>
          ) : null}
          <div className="wk-diff">
            <FileDiff
              oldFile={{ name: FILE, contents: entryYaml(entryShape(entry)) }}
              newFile={{
                name: FILE,
                contents: entryYaml(entryShape(entry, draft)),
              }}
              disableFileHeader
              contextExpansionFocusFallback={review}
            />
          </div>
        </section>
      ) : null}
      <div className="wk-settings-content">
        <fieldset ref={fields} className="wk-fields" disabled={busy}>
          <legend className="sr-only">Settings for {name}</legend>
          <div className="wk-form">
            <SettingsSection
              title="Run command"
              description={
                declared?.source === 'path' ? (
                  <>
                    Worker source:{' '}
                    <span className="wk-mono">{shortPath(declared.ref)}</span>.
                  </>
                ) : (
                  'Leave empty to use the command declared by the worker package.'
                )
              }
            >
              <SettingsList>
                <SettingsField
                  className="wk-command-field"
                  label="Command"
                  description={<span className="wk-mono">scripts.run</span>}
                  layout="stacked"
                  controlSize="full"
                  action={
                    <IconButton
                      label={copied ? 'Command copied' : 'Copy run command'}
                      variant="ghost"
                      disabled={!draft.run}
                      onClick={() =>
                        void copyText(draft.run).then((ok) => {
                          setCopied(ok)
                          if (!ok)
                            setNotice({
                              variant: 'warn',
                              headline: 'Command could not be copied',
                              detail:
                                'Select the command text and copy it with your keyboard.',
                            })
                          if (copyTimer.current) clearTimeout(copyTimer.current)
                          copyTimer.current = setTimeout(
                            () => setCopied(false),
                            1800,
                          )
                        })
                      }
                    >
                      {copied ? <Check /> : <Copy />}
                    </IconButton>
                  }
                  renderControl={(control) => (
                    <Input
                      {...control}
                      className="wk-mono"
                      value={draft.run}
                      onChange={(run) => set({ run })}
                      spellCheck={false}
                      autoComplete="off"
                    />
                  )}
                />
              </SettingsList>
              {PREBUILT.test(draft.run) ? (
                <Button
                  variant="pill"
                  size="sm"
                  className="wk-suggestion"
                  onClick={() =>
                    set({ run: `cargo run --locked --bin ${name}` })
                  }
                >
                  <span>Use build command:</span>{' '}
                  <span className="wk-mono">
                    cargo run --locked --bin {name}
                  </span>
                </Button>
              ) : null}
            </SettingsSection>
            <SettingsSection
              title="Starts after"
              description={`Starts ${name} once these workers are ready, and stops it before them.`}
            >
              <SettingsList>
                {draft.startAfter.map((dep) => (
                  <SettingsRow
                    key={dep}
                    data-dependency-error={
                      dependencyErrors.has(dep) ? '' : undefined
                    }
                    layout={dependencyErrors.has(dep) ? 'auto' : 'inline'}
                    label={<span className="wk-mono">{dep}</span>}
                    description={states.get(dep) ?? 'Not in the live project'}
                    meta={
                      dependencyErrors.has(dep) ? (
                        <span className="wk-field-error" role="alert">
                          {dependencyErrors.get(dep)} Remove this dependency
                          before saving.
                        </span>
                      ) : undefined
                    }
                    action={
                      <IconButton
                        label={`Stop waiting for ${dep}`}
                        variant="ghost"
                        onClick={() =>
                          set({
                            startAfter: draft.startAfter.filter(
                              (d) => d !== dep,
                            ),
                          })
                        }
                      >
                        <X />
                      </IconButton>
                    }
                  />
                ))}
                <SettingsRow
                  label={
                    draft.startAfter.length
                      ? 'Add dependency'
                      : 'Starts with the engine'
                  }
                  control={
                    <Selector
                      aria-label="Add dependency"
                      contentClassName="wk-dependency-picker"
                      placeholder="Add dependency…"
                      searchPlaceholder="Search dependencies"
                      value={undefined}
                      disabled={busy || !waitable.length}
                      options={waitable.map((other) => {
                        const reason = dependencyBlockReason(
                          containers,
                          name,
                          other,
                        )
                        return {
                          value: other,
                          label: other,
                          disabled: !!reason,
                          description:
                            reason ?? states.get(other) ?? 'Not running',
                        }
                      })}
                      onChange={(dep) =>
                        set({ startAfter: [...draft.startAfter, dep] })
                      }
                      emptyMessage="No workers match. Try another name."
                    />
                  }
                />
              </SettingsList>
            </SettingsSection>
            <SettingsSection
              title="Environment"
              description={
                <>
                  {entry.env_file.length ? (
                    <>
                      Also loads{' '}
                      <span className="wk-mono">
                        {entry.env_file.join(', ')}
                      </span>
                      .{' '}
                    </>
                  ) : null}
                  Secrets are write-only. Replace a value to change it; saved
                  secrets never appear in the form or preview.
                </>
              }
              action={
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() =>
                    set({
                      env: [
                        ...draft.env,
                        {
                          key: '',
                          value: '',
                          secret: false,
                          replacing: false,
                          removed: false,
                          isNew: true,
                        },
                      ],
                    })
                  }
                >
                  <Plus />
                  Add variable
                </Button>
              }
            >
              {draft.env.length ? (
                <SettingsList>
                  {draft.env.map((row, index) => (
                    <EnvironmentRow
                      key={row.isNew ? `new-${index}` : row.key}
                      row={row}
                      error={validation.env[index]}
                      onChange={(next) => setEnv(index, next)}
                      onRemove={() =>
                        row.isNew
                          ? set({
                              env: draft.env.filter((_, i) => i !== index),
                            })
                          : setEnv(index, { removed: true })
                      }
                    />
                  ))}
                </SettingsList>
              ) : (
                <p className="wk-note">
                  No variables override the environment loaded by the worker.
                </p>
              )}
            </SettingsSection>
            <SettingsSection
              title="Configuration override"
              description="A YAML mapping merged over the worker's own configuration. Values here replace matching keys; leave empty to keep the worker's configuration."
            >
              <WorkerConfigurationReference key={name} name={name} />
              <p id={configHintId} className="wk-note">
                Tab indents. Press {tabFocusKey} to make Tab move between
                controls.
              </p>
              <fieldset
                className="wk-code"
                aria-label="Configuration override"
                aria-describedby={configDescription}
              >
                <CodeEditor
                  ref={configEditor}
                  aria-label="Configuration override YAML"
                  aria-invalid={!!validation.config}
                  aria-describedby={configDescription}
                  aria-errormessage={
                    validation.config ? configErrorId : undefined
                  }
                  language="yaml"
                  value={draft.config}
                  readOnly={busy}
                  onChange={(config) => set({ config })}
                  lineNumbers
                />
              </fieldset>
              {validation.config ? (
                <p id={configErrorId} className="wk-field-error" role="alert">
                  {validation.config}
                  {validation.configDetail ? (
                    <span className="wk-note block">
                      {validation.configDetail}
                    </span>
                  ) : null}
                </p>
              ) : null}
            </SettingsSection>
          </div>
        </fieldset>
        <details className="wk-remove">
          <summary>Remove from project</summary>
          <SettingsList>
            <SettingsRow
              label={`Remove ${name}`}
              description="Removes this worker and references to it from the compose file. Only this worker stops; this does not delete its files."
              action={
                <Button
                  variant="ghost"
                  size="sm"
                  className="wk-danger"
                  disabled={busy}
                  onClick={onRemove}
                >
                  Remove {name}…
                </Button>
              }
            />
          </SettingsList>
        </details>
      </div>
    </div>
  )
}
