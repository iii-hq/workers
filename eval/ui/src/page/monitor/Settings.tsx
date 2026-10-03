import {
  Badge,
  Button,
  Input,
  Select,
  Selector,
  SettingsField,
  SettingsList,
  SettingsRow,
  SettingsSection,
  StatusDot,
  StatusPanel,
  Switch,
  uiClasses,
  useConfirm,
} from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { ArrowLeft, CircleAlert, CircleCheck, LoaderCircle, RefreshCw, TriangleAlert } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { EvalApi } from '../../api'
import type { CatalogModel, MonitorConfig, MonitorLimits, MonitorState, ThinkingLevel } from '../../types'
import {
  buildCatalog,
  CODE_ACCESS_RISK,
  codeDirectoryError,
  codeDirectoryHelp,
  codeRepository,
  type Draft,
  draftFromConfig,
  isDirty,
  JUDGE,
  limitRows,
  modelLabel,
  PROVIDER_DEFAULT,
  pickModel,
  runningHint,
  savedNotice,
  THINKING_LABEL,
  thinkingChoices,
  toMonitorModel,
  triageHint,
  triageWarning,
} from './settings-model'

export interface MonitorSettingsProps {
  api: EvalApi
  state: MonitorState | null
  /** What the monitor enforces; the Limits section waits for it. */
  limits: MonitorLimits | undefined
  /** Analyses in flight: they keep the model they started with. */
  runningCount: number
  /** One level at a time: only then does the page need its own way back to the list. */
  narrow: boolean
  onSaved: (config: MonitorConfig) => void
  onClose?: () => void
  /** Reports unsaved edits so the host can guard navigation (`setDirty`). */
  onDirtyChange?: (dirty: boolean) => void
}

type Load<T> = { status: 'loading' } | { status: 'ready'; value: T } | { status: 'error'; message: string }

/** Runs `load` on mount and on `reload`; a stale answer never lands. */
function useRequest<T>(load: () => Promise<T>): [Load<T>, () => void] {
  const [result, setResult] = useState<Load<T>>({ status: 'loading' })
  const token = useRef(0)
  const loadRef = useRef(load)
  loadRef.current = load
  const run = useCallback(() => {
    const mine = ++token.current
    setResult({ status: 'loading' })
    loadRef.current().then(
      (value) => {
        if (token.current === mine) setResult({ status: 'ready', value })
      },
      (error: unknown) => {
        if (token.current === mine) setResult({ status: 'error', message: errorMessage(error) })
      },
    )
  }, [])
  useEffect(() => {
    run()
    return () => {
      token.current++
    }
  }, [run])
  return [result, run]
}

const NO_MODELS: CatalogModel[] = []

export function MonitorSettings({
  api,
  state,
  limits,
  runningCount,
  narrow,
  onSaved,
  onClose,
  onDirtyChange,
}: MonitorSettingsProps) {
  const [baseline, setBaseline] = useState<MonitorConfig | null>(state?.config ?? null)
  const editing = baseline !== null
  const base = useMemo(() => draftFromConfig(baseline), [baseline])
  const [draft, setDraft] = useState<Draft>(base)
  const [saving, setSaving] = useState<'paused' | 'observing' | 'changes' | null>(null)
  const [saveError, setSaveError] = useState<string | null>(null)
  // The backend's refusal of the directory, shown on its field; editing the field clears it.
  const [codeError, setCodeError] = useState<string | null>(null)
  const [notice, setNotice] = useState<string | null>(null)
  const { confirm, dialog } = useConfirm()

  // A config that changes elsewhere replaces the form only while it has no edits.
  const config = state?.config ?? null
  const previousBase = useRef(base)
  useEffect(() => {
    setBaseline(config)
  }, [config])
  useEffect(() => {
    const previous = previousBase.current
    previousBase.current = base
    setDraft((current) => (isDirty(previous, current) ? current : base))
  }, [base])

  const [catalog, reloadCatalog] = useRequest(() => api.models())
  const [triageResult, reloadTriage] = useRequest(async () => (await api.monitor(true)).triage ?? null)
  const triage = triageResult.status === 'ready' ? triageResult.value : (state?.triage ?? null)

  const catalogReady = catalog.status === 'ready'
  const catalogFailed = catalog.status === 'error'
  const view = useMemo(
    () => buildCatalog(catalog.status === 'ready' ? catalog.value : NO_MODELS, baseline?.model ?? null),
    [catalog, baseline],
  )
  const entry = draft.modelKey ? view.byKey.get(draft.modelKey) : undefined
  const choices = thinkingChoices(entry)

  const busy = saving !== null
  const dirty = isDirty(base, draft)
  const canSave = entry !== undefined && !busy

  const onDirtyRef = useRef(onDirtyChange)
  onDirtyRef.current = onDirtyChange
  useEffect(() => {
    onDirtyRef.current?.(dirty)
  }, [dirty])
  useEffect(
    () => () => {
      onDirtyRef.current?.(false)
    },
    [],
  )

  const edit = (change: (current: Draft) => Draft) => {
    setDraft(change)
    setNotice(null)
    setSaveError(null)
    setCodeError(null)
  }

  const save = async (enabled: boolean, kind: 'paused' | 'observing' | 'changes') => {
    if (!entry || busy) return
    setSaving(kind)
    setSaveError(null)
    setCodeError(null)
    try {
      const next = await api.configure(
        enabled,
        toMonitorModel(entry, draft.thinking, baseline?.model ?? null),
        codeRepository(draft),
      )
      setNotice(
        savedNotice({
          before: baseline,
          after: next,
          runningCount,
          now: Date.now(),
          budgetMs: limits?.analysis_budget_ms,
        }),
      )
      setBaseline(next)
      setDraft(draftFromConfig(next))
      onSaved(next)
    } catch (error) {
      const message = errorMessage(error)
      const field = codeDirectoryError(message)
      if (field) setCodeError(field)
      else setSaveError(message)
    } finally {
      setSaving(null)
    }
  }

  const back = async () => {
    if (
      dirty &&
      !(await confirm({
        title: 'Discard unsaved changes?',
        description: "Your edits to the monitor settings haven't been saved.",
        confirmLabel: 'Discard',
        cancelLabel: 'Keep editing',
        tone: 'danger',
      }))
    ) {
      return
    }
    onClose?.()
  }

  const modelPlaceholder =
    catalog.status === 'loading' ? 'Loading models…' : catalogFailed ? 'Models unavailable' : 'Choose a model'
  const thinkingDisabled = busy || !entry || !catalogReady
  const showThinking = !entry || !catalogReady || choices.length > 0
  const thinkingPlaceholder = !entry || catalogFailed ? 'Choose a model first' : 'Provider default'
  const thinkingValue = !catalogFailed && entry && choices.length > 0 ? (draft.thinking ?? PROVIDER_DEFAULT) : undefined

  let modelHint: string | null = null
  if (!baseline) {
    modelHint =
      'An analysis keeps the model it started with. Changing it later only affects new analyses. If the catalog fails, nothing is picked for you.'
  } else {
    modelHint = runningHint(runningCount, baseline, Date.now(), limits?.analysis_budget_ms)
  }

  let footnote: string | null = null
  if (editing) {
    footnote = catalogFailed
      ? 'Choosing a new model is disabled until the list loads.'
      : dirty
        ? 'Unsaved changes'
        : 'No unsaved changes'
  } else if (!entry && !catalogFailed) {
    footnote = 'Choose an analyst model to save.'
  }

  return (
    <div className="eval-ui-settings">
      <form
        className="eval-ui-settings-body"
        aria-label={editing ? 'Monitor settings' : 'Session monitor setup'}
        onSubmit={(event) => event.preventDefault()}
      >
        <header className="eval-ui-settings-head">
          {onClose && narrow ? (
            <Button variant="ghost" size="sm" data-settings-narrow-action onClick={() => void back()}>
              <ArrowLeft aria-hidden size={16} />
              Back to analyses
            </Button>
          ) : null}
          <h1 className="eval-ui-settings-title">{editing ? 'Monitor settings' : 'Session monitor'}</h1>
          <p className="eval-ui-settings-lede">
            {editing
              ? 'Changes apply to new analyses. Analyses already running keep the model and codebase directory they started with.'
              : 'Watches finished Harness sessions, including successful ones. It flags behavior worth improving and drafts suggestions with an E2E plan. It never edits code, opens PRs or touches the observed session.'}
          </p>
        </header>

        {notice ? (
          <StatusPanel
            variant="success"
            role="status"
            icon={<CircleCheck aria-hidden size={16} />}
            headline="Settings saved"
            detail={notice}
          />
        ) : null}

        <SettingsSection
          title="Analyst model"
          description="Investigates signals and writes suggestions, with your provider's credentials."
        >
          {catalogFailed ? (
            <StatusPanel
              variant="alert"
              role="alert"
              icon={<CircleAlert aria-hidden size={16} />}
              headline="Couldn't load the model list"
              detail={
                <>
                  router::models::list didn't answer, so no model is picked for you.
                  {catalog.message ? (
                    <>
                      {' '}
                      <span className="eval-ui-settings-mono">{catalog.message}</span>
                    </>
                  ) : null}
                </>
              }
              action={
                <Button variant="pill" size="sm" data-settings-narrow-action onClick={reloadCatalog}>
                  <RefreshCw aria-hidden size={16} />
                  Retry
                </Button>
              }
            />
          ) : null}
          <SettingsList>
            {catalogFailed && baseline ? (
              <>
                <SettingsRow
                  label="Saved model"
                  control={<span className="eval-ui-settings-mono">{modelLabel(baseline.model)}</span>}
                />
                <SettingsRow
                  label="Thinking"
                  control={
                    baseline.model.thinking_level ? THINKING_LABEL[baseline.model.thinking_level] : 'Provider default'
                  }
                />
              </>
            ) : null}
            <SettingsField
              id="eval-settings-model"
              field="model"
              label="Model"
              renderControl={(controlProps) => (
                <Selector
                  {...controlProps}
                  className={draft.modelKey && !catalogFailed ? 'eval-ui-settings-model' : undefined}
                  contentClassName="eval-ui-settings-models"
                  aria-label="Model"
                  value={catalogFailed ? undefined : (draft.modelKey ?? undefined)}
                  groups={view.groups}
                  disabled={busy || !catalogReady}
                  loading={catalog.status === 'loading'}
                  placeholder={modelPlaceholder}
                  searchPlaceholder="Search models"
                  emptyMessage="No models match"
                  onChange={(key) => {
                    const picked = view.byKey.get(key)
                    if (picked) edit((current) => pickModel(current, picked))
                  }}
                />
              )}
            />
            {showThinking ? (
              <SettingsField
                id="eval-settings-thinking"
                field="thinking"
                label="Thinking level"
                renderControl={(controlProps) => (
                  <Select<string>
                    {...controlProps}
                    aria-label="Thinking level"
                    value={thinkingValue}
                    disabled={thinkingDisabled}
                    placeholder={thinkingPlaceholder}
                    options={
                      choices.length > 0
                        ? [
                            { value: PROVIDER_DEFAULT, label: 'Provider default' },
                            ...choices.map((level) => ({ value: level, label: THINKING_LABEL[level] })),
                          ]
                        : []
                    }
                    onChange={(value) => {
                      // The shared Select reports '' on mount when the draft has no level; that is not a choice.
                      if (!value) return
                      edit((current) => ({
                        ...current,
                        thinking: value === PROVIDER_DEFAULT ? null : (value as ThinkingLevel),
                      }))
                    }}
                  />
                )}
              />
            ) : null}
          </SettingsList>
          {modelHint ? <p className="eval-ui-settings-hint">{modelHint}</p> : null}
        </SettingsSection>

        <SettingsSection
          title="Code access"
          description="Lets the analyst read the code behind a signal, not only the transcripts."
        >
          <SettingsList>
            <SettingsField
              id="eval-settings-code-directory"
              field="code_repository"
              label={
                <>
                  Codebase directory <span className="eval-ui-settings-optional">Optional</span>
                </>
              }
              description={codeDirectoryHelp(limits)}
              error={codeError ?? undefined}
              layout="stacked"
              controlSize="full"
              renderControl={(controlProps) => (
                <Input
                  {...controlProps}
                  className="eval-ui-settings-mono"
                  value={draft.codeDirectory}
                  placeholder="/home/you/workspaces/workers"
                  spellCheck={false}
                  autoComplete="off"
                  disabled={busy}
                  onChange={(codeDirectory) => edit((current) => ({ ...current, codeDirectory }))}
                />
              )}
            />
          </SettingsList>
          <p className="eval-ui-settings-hint eval-ui-settings-risk">
            <TriangleAlert aria-hidden size={16} />
            {CODE_ACCESS_RISK}
          </p>
        </SettingsSection>

        <SettingsSection
          title="Triage"
          description="Jev classifies each session before the analyst model is called."
          action={
            triageResult.status === 'error' || (triageResult.status === 'ready' && triage?.available === false) ? (
              <Button variant="ghost" size="sm" data-settings-narrow-action onClick={reloadTriage}>
                <RefreshCw aria-hidden size={16} />
                Check again
              </Button>
            ) : null
          }
        >
          <SettingsList>
            <SettingsRow
              label="Provider"
              control={
                <>
                  <span className="eval-ui-settings-mono">{JUDGE}</span>
                  {triage ? (
                    triage.available ? (
                      <Badge variant="ok">
                        <StatusDot tone="ok" aria-hidden />
                        Available
                      </Badge>
                    ) : (
                      <Badge variant="alert">
                        <StatusDot tone="alert" aria-hidden />
                        Unavailable
                        {triage.code ? (
                          <>
                            {' · '}
                            <span className="eval-ui-settings-mono">{triage.code}</span>
                          </>
                        ) : null}
                      </Badge>
                    )
                  ) : triageResult.status === 'loading' ? (
                    <Badge>Checking</Badge>
                  ) : (
                    <Badge variant="warn">
                      <StatusDot tone="warn" aria-hidden />
                      Couldn't check
                    </Badge>
                  )}
                </>
              }
            />
            <SettingsRow
              label="Key"
              control={
                triage && !triage.available
                  ? triageHint(triage.code)
                  : 'Kept in judge-typesafe. The monitor never stores it.'
              }
            />
            <SettingsRow
              label="Model"
              control={
                triage?.models[0] ? (
                  <span className="eval-ui-settings-mono">{triage.models[0]}</span>
                ) : (
                  "Set by the provider's configuration"
                )
              }
            />
          </SettingsList>
          {triage && !triage.available ? (
            <StatusPanel
              variant="warn"
              role="status"
              icon={<TriangleAlert aria-hidden size={16} />}
              headline="Triage can't run"
              detail={triageWarning(triage)}
            />
          ) : null}
        </SettingsSection>

        <SettingsSection title="Observation">
          <SettingsList>
            <SettingsField
              id="eval-settings-observe"
              field="enabled"
              label="Analyze finished sessions automatically"
              description={`Completed, failed and cancelled sessions, with their descendants. The monitor's own sessions are skipped. Pausing stops new analyses; analyses already running continue until you cancel them.${editing && catalogFailed ? " Saving this doesn't need the model list." : ''}`}
              layout="inline"
              controlSize="fit"
              renderControl={(controlProps) => (
                <Switch
                  {...controlProps}
                  checked={draft.enabled}
                  // First run: the two save buttons below decide whether observation starts.
                  disabled={busy || !editing}
                  onChange={(event) => {
                    const enabled = event.currentTarget.checked
                    edit((current) => ({ ...current, enabled }))
                  }}
                />
              )}
            />
          </SettingsList>
        </SettingsSection>

        {limits ? (
          <SettingsSection title="Limits" description="Set by the monitor; not editable here.">
            <SettingsList>
              {limitRows(limits, Boolean(baseline?.code_repository)).map(([label, value]) => (
                <SettingsRow
                  key={label}
                  label={label}
                  control={<span className="eval-ui-settings-mono eval-ui-settings-value">{value}</span>}
                />
              ))}
            </SettingsList>
          </SettingsSection>
        ) : null}

        {saveError ? (
          <StatusPanel
            variant="alert"
            role="alert"
            icon={<CircleAlert aria-hidden size={16} />}
            headline="Couldn't save the settings"
            detail={saveError}
          />
        ) : null}

        <div className="eval-ui-settings-footer">
          {footnote ? <span className="eval-ui-settings-footnote">{footnote}</span> : null}
          {editing ? (
            <>
              <Button
                variant="ghost"
                data-settings-narrow-action
                disabled={!dirty || busy}
                onClick={() => {
                  setDraft(base)
                  setSaveError(null)
                }}
              >
                Discard changes
              </Button>
              <Button
                variant="primary"
                data-settings-narrow-action
                aria-busy={saving === 'changes'}
                disabled={!dirty || !canSave}
                onClick={() => void save(draft.enabled, 'changes')}
              >
                {saving === 'changes' ? <LoaderCircle aria-hidden className={uiClasses.spin} size={16} /> : null}
                Save changes
              </Button>
            </>
          ) : (
            <>
              <Button
                variant="pill"
                data-settings-narrow-action
                aria-busy={saving === 'paused'}
                disabled={!canSave}
                onClick={() => void save(false, 'paused')}
              >
                {saving === 'paused' ? <LoaderCircle aria-hidden className={uiClasses.spin} size={16} /> : null}
                Save paused
              </Button>
              <Button
                variant="primary"
                data-settings-narrow-action
                aria-busy={saving === 'observing'}
                disabled={!canSave}
                onClick={() => void save(true, 'observing')}
              >
                {saving === 'observing' ? <LoaderCircle aria-hidden className={uiClasses.spin} size={16} /> : null}
                Save and start observing
              </Button>
            </>
          )}
        </div>
      </form>
      {dialog}
    </div>
  )
}
