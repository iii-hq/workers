import {
  Breadcrumb,
  Button,
  Chip,
  Eyebrow,
  IconButton,
  Input,
  ListItem,
  PageBody,
  PageHeader,
  PageMain,
  PageSidebar,
  Panel,
  PanelBody,
  PanelHeader,
  SegmentedControl,
  Select,
  SettingsField,
  SettingsList,
  SettingsSection,
  StatusBar,
  StatusDot,
  StatusPanel,
  Switch,
  Toolbar,
} from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { ArrowLeft, Minus, RefreshCcw } from 'lucide-react'
import { type ReactNode, useEffect, useRef, useState } from 'react'
import type { EvalApi } from '../api'
import {
  type ConfigDraft,
  configFromDraft,
  draftFromConfig,
  draftFromJson,
  EMPTY_DRAFT,
  emptySection,
  type FieldSpec,
  SECTIONS,
  type SectionId,
} from '../config-draft'
import { isActive, type Setup, validateConfig } from '../model'
import type { ConfigResponse, CycleRecord, ThinkingLevel, Where } from '../types'
import { ago } from './bits'
import type { Nav } from './index'

const WHERE = [
  { value: 'harness' as const, label: 'Harness', icon: false as const },
  { value: 'docker' as const, label: 'Docker', icon: false as const },
  { value: 'github' as const, label: 'GitHub', icon: false as const },
]

const THINKING: { value: ThinkingLevel; label: string }[] = ['off', 'minimal', 'low', 'medium', 'high', 'xhigh'].map(
  (value) => ({ value: value as ThinkingLevel, label: value }),
)

/** The ladder: each step needs the one before it. Judge and controls sit apart. */
const STEPS: { id: Setup; section?: SectionId; eyebrow: string; needs?: SectionId; intro: string }[] = [
  {
    id: 'measure',
    eyebrow: '1 · Required',
    intro:
      "Every cycle first measures today's Harness on the E2E. Without a test that covers the goal, nothing runs and nothing is spent.",
  },
  {
    id: 'improve',
    section: 'candidate',
    eyebrow: '2 · Optional',
    intro:
      'Lets the loop propose one change and commit it anywhere in the repository but eval/ and git metadata, in its own worktree. The actor gets no shell, grants or credentials.',
  },
  {
    id: 'prove',
    section: 'validation',
    needs: 'candidate',
    eyebrow: '3 · Needs Improve',
    intro:
      'Runs the candidate commit on a stack where only the Harness differs. It is accepted or rejected in code, and an accepted change is opened as a pull request for review.',
  },
]
const TITLE: Record<Setup, string> = {
  measure: 'Measure',
  improve: 'Improve',
  prove: 'Prove',
  judge: 'Also',
}

/** What a step that is off leaves out, said once more beside its switch. */
const OFF_DETAIL: Record<Setup, string | undefined> = {
  measure: undefined,
  judge: undefined,
  improve:
    'Turn it on to name the repository, the base commit and the actor model. A cycle then proposes and writes one change.',
  prove: 'Turn it on to name the stack that pins the candidate commit.',
}

/** `eval::config` / `eval::configure`: the loop's `LoopConfigV1`, nothing else. */
export function LoopSettings({
  api,
  state,
  error: readError,
  cycles,
  section,
  narrow,
  nav,
  onClose,
  onSaved,
  onDirtyChange,
}: {
  api: EvalApi
  state: ConfigResponse | null
  error: string | null
  cycles: CycleRecord[]
  section?: Setup
  narrow: boolean
  nav: Nav
  onClose?: () => void
  onSaved: () => void
  onDirtyChange: (dirty: boolean) => void
}) {
  const stored = state?.config ?? null
  const [draft, setDraft] = useState<ConfigDraft>(() => (stored ? draftFromConfig(stored.config) : EMPTY_DRAFT))
  const [dirty, setDirty] = useState(false)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [json, setJson] = useState<string | null>(null)
  const [shown, setShown] = useState<Setup>(section ?? 'measure')
  const anchors = useRef<Partial<Record<Setup, HTMLElement | null>>>({})

  // A clean form always shows what is stored, including a change made elsewhere.
  useEffect(() => {
    if (!dirty) setDraft(stored ? draftFromConfig(stored.config) : EMPTY_DRAFT)
  }, [dirty, stored])
  useEffect(() => onDirtyChange(dirty), [dirty, onDirtyChange])
  useEffect(() => {
    if (!section) return
    setShown(section)
    anchors.current[section]?.scrollIntoView({ block: 'start' })
  }, [section])

  const set = (patch: Partial<ConfigDraft>) => {
    setDraft((previous) => ({ ...previous, ...patch }))
    setDirty(true)
    setError(null)
  }
  const on = (id: SectionId) => draft.sections[id] !== null
  // Turning a step off turns off every step that needs it.
  const toggle = (id: SectionId, next: boolean) => {
    const sections = { ...draft.sections, [id]: next ? emptySection(id) : null }
    if (!next && id === 'candidate') sections.validation = null
    set({ sections })
  }

  async function save() {
    const config = configFromDraft(draft)
    const problem = config ? validateConfig(config) : 'choose where the E2E runs'
    if (!config || problem) {
      setError(problem)
      return
    }
    setSaving(true)
    setError(null)
    try {
      await api.configure(config)
      setDirty(false)
      nav.announce('Loop settings saved')
      onSaved()
    } catch (failure) {
      setError(errorMessage(failure))
    } finally {
      setSaving(false)
    }
  }

  function discard() {
    setDraft(stored ? draftFromConfig(stored.config) : EMPTY_DRAFT)
    setDirty(false)
    setError(null)
  }

  function applyJson() {
    if (json === null) return
    try {
      setDraft(draftFromJson(json))
      setDirty(true)
      setError(null)
      setJson(null)
    } catch (failure) {
      setError(errorMessage(failure))
    }
  }

  const go = (id: Setup) => {
    setShown(id)
    anchors.current[id]?.scrollIntoView({ block: 'start', behavior: 'smooth' })
  }
  const frozen = stored
    ? cycles.filter((cycle) => isActive(cycle.status) && cycle.config?.revision === stored.revision).length
    : 0
  // Numbers take a compact box; names, models and lists the full control width.
  const field = (key: keyof ConfigDraft, label: string, description?: string, wide = false) => (
    <SettingsField
      label={label}
      description={description}
      controlSize={wide ? 'default' : 'compact'}
      renderControl={(props) => (
        <Input
          {...props}
          value={draft[key] as string}
          onChange={(value) => set({ [key]: value })}
          className="eval-ui-mono-input"
          autoComplete="off"
          spellCheck={false}
        />
      )}
    />
  )
  const needsStack = draft.where === 'docker' || draft.where === 'github'

  const nav_ = (
    <nav aria-label="Settings sections" className="eval-ui-settings-nav">
      <p className="eval-ui-group-label">What the loop may do</p>
      {STEPS.map((step, index) => {
        const ready = step.section ? on(step.section) : !!stored
        return (
          <ListItem
            key={step.id}
            onClick={() => go(step.id)}
            selected={shown === step.id}
            leading={<span className="eval-ui-num eval-ui-faint">{index + 1}</span>}
            label={TITLE[step.id]}
            trailing={
              <Chip>
                <StatusDot tone={ready ? 'ok' : 'ink'} />{' '}
                {ready ? (step.section ? 'On' : 'Ready') : step.section ? 'Off' : 'Not set'}
              </Chip>
            }
          />
        )
      })}
      <p className="eval-ui-group-label">Also</p>
      <ListItem
        onClick={() => go('judge')}
        selected={shown === 'judge'}
        label="Judge"
        trailing={<span className="eval-ui-faint">{on('judge') ? 'On' : 'Off'}</span>}
      />
      <ListItem
        onClick={() => go('judge')}
        label="Controls"
        trailing={
          <span className="eval-ui-num eval-ui-faint">
            {draft.controls.split(/[,\n]/).filter((control) => control.trim()).length}
          </span>
        }
      />
      <p className="eval-ui-faint eval-ui-nav-note">
        Each step needs the one before it. Running cycles keep the values they froze.
      </p>
    </nav>
  )

  return (
    <>
      <PageHeader
        icon={<RefreshCcw size={16} aria-hidden />}
        title="Eval loop"
        description={narrow ? 'Loop settings' : 'Every new cycle freezes these values when it starts'}
        onClose={onClose}
      />
      <PageBody>
        {narrow ? null : (
          <PageSidebar label="Settings sections" defaultWidth={260} className="eval-ui-sidebar">
            {nav_}
          </PageSidebar>
        )}
        <PageMain className="eval-ui-main">
          <Toolbar
            aria-label="Loop settings"
            end={
              <>
                {dirty ? (
                  <Button variant="ghost" onClick={discard}>
                    Discard
                  </Button>
                ) : null}
                <Button variant="primary" disabled={saving || !dirty} onClick={() => void save()}>
                  {saving ? 'Saving…' : 'Save'}
                </Button>
              </>
            }
          >
            <IconButton label="Back to cycles" onClick={nav.home}>
              <ArrowLeft size={16} aria-hidden />
            </IconButton>
            <Breadcrumb
              items={
                narrow
                  ? [{ label: 'Loop settings' }]
                  : [{ label: 'Cycles', onClick: nav.home }, { label: 'Loop settings' }]
              }
            />
          </Toolbar>
          <div className="eval-ui-scroll">
            <div className="eval-ui-form">
              {readError ? (
                <StatusPanel variant="alert" headline="Couldn't read the settings" detail={readError} />
              ) : null}
              {error ? <StatusPanel variant="alert" role="alert" headline="Not saved" detail={error} /> : null}
              {STEPS.map((step) => {
                const id = step.section
                const blocked = step.needs ? !on(step.needs) : false
                return (
                  <section
                    key={step.id}
                    ref={(element) => {
                      anchors.current[step.id] = element
                    }}
                    className="eval-ui-step-section"
                  >
                    <SettingsSection
                      title={
                        <span className="eval-ui-section-title">
                          <Eyebrow as="span">{step.eyebrow}</Eyebrow>
                          {TITLE[step.id]}
                        </span>
                      }
                      description={step.intro}
                      action={
                        id ? (
                          <Switch
                            aria-label={TITLE[step.id]}
                            checked={on(id)}
                            disabled={blocked && !on(id)}
                            onChange={(event) => toggle(id, event.target.checked)}
                          />
                        ) : undefined
                      }
                    >
                      {step.id === 'measure' ? (
                        <>
                          <SettingsList>
                            {field('model', 'Model under test', 'The subject of every E2E run in a cycle.', true)}
                            {field('provider', 'Provider', undefined, true)}
                            {field('agent', 'Agent profile', 'Optional; empty uses the E2E default.', true)}
                            <SettingsField
                              label="Where the E2E runs"
                              description="Harness uses the E2E's own stack. Docker and GitHub run a pinned stack."
                              renderControl={() => (
                                <SegmentedControl<Where | ''>
                                  aria-label="Where the E2E runs"
                                  variant="radio"
                                  value={draft.where}
                                  onChange={(where) =>
                                    set(where === 'harness' ? { where, stackName: '', stackYaml: '' } : { where })
                                  }
                                  options={WHERE}
                                />
                              )}
                            />
                            {needsStack ? field('stackName', 'Stack name', undefined, true) : null}
                            {needsStack ? (
                              <TextRow
                                label="Stack YAML"
                                value={draft.stackYaml}
                                onChange={(stackYaml) => set({ stackYaml })}
                              />
                            ) : null}
                            {field(
                              'runs_per_side',
                              'Runs per side',
                              'Paired runs on each side, 3 by default. With fewer than 5 a change is accepted only when every pair improves.',
                            )}
                            {field(
                              'technical_retries',
                              'Technical retries',
                              'Re-runs of a slot the infrastructure broke. A slot that needed one counts as a fail.',
                            )}
                            {field(
                              'max_scenarios',
                              'Scenarios per cycle',
                              'The covering test plus the control scenarios.',
                            )}
                            {field(
                              'estimated_usd_per_run',
                              'Estimated cost per run (USD)',
                              'Reserved before each run. A run that reports less is still charged this.',
                            )}
                            {field(
                              'min_effect_pass_rate_pp',
                              'Smallest change worth keeping (pp)',
                              'On criterion scores and pass rate, in percentage points.',
                            )}
                            {field(
                              'min_effect_tokens',
                              'Smallest token change',
                              'On token objectives, in tokens per run.',
                            )}
                          </SettingsList>
                          <Panel>
                            <PanelHeader>Budget per cycle</PanelHeader>
                            <PanelBody className="eval-ui-budget">
                              {(
                                [
                                  ['max_spend_usd', 'Spend (USD)'],
                                  ['max_duration_min', 'Time (minutes)'],
                                  ['max_calls', 'Calls'],
                                  ['max_candidates', 'Candidates'],
                                ] as const
                              ).map(([key, label]) => (
                                <div key={key} className="eval-ui-stack-sm">
                                  <Eyebrow as="span" aria-hidden>
                                    {label}
                                  </Eyebrow>
                                  <Input
                                    aria-label={label}
                                    value={draft[key]}
                                    onChange={(value) => set({ [key]: value })}
                                    className="eval-ui-mono-input"
                                    inputMode="decimal"
                                    autoComplete="off"
                                  />
                                </div>
                              ))}
                              <p className="eval-ui-faint eval-ui-span-all">
                                A cycle that passes a limit stops what it started.
                              </p>
                            </PanelBody>
                          </Panel>
                        </>
                      ) : id && on(id) ? (
                        <>
                          <SettingsList>
                            <SectionFields id={id} draft={draft} set={set} />
                          </SettingsList>
                          {id === 'validation' ? (
                            <p className="eval-ui-faint">
                              The pull request uses the GitHub token in the worker's environment (EVAL_GITHUB_TOKEN),
                              never one typed in this form.
                            </p>
                          ) : null}
                        </>
                      ) : id ? (
                        <StatusPanel
                          variant="info"
                          icon={<Minus size={16} aria-hidden />}
                          headline={
                            blocked
                              ? `Needs ${step.needs === 'candidate' ? 'Improve' : 'Prove'} first`
                              : SECTIONS.find((spec) => spec.id === id)?.off
                          }
                          detail={OFF_DETAIL[step.id]}
                          action={
                            blocked ? undefined : (
                              <Button variant="pill" onClick={() => toggle(id, true)}>
                                Set up
                              </Button>
                            )
                          }
                        />
                      ) : null}
                    </SettingsSection>
                  </section>
                )
              })}

              <section
                ref={(element) => {
                  anchors.current.judge = element
                }}
                className="eval-ui-step-section"
              >
                <SettingsSection title="Also">
                  <SettingsList>
                    <SettingsField
                      label="Judge"
                      description="Asked once per cycle, only when an opportunity is ambiguous. A deterministic defect needs no judgment."
                      controlSize="fit"
                      renderControl={(props) => (
                        <Switch
                          {...props}
                          checked={on('judge')}
                          onChange={(event) => toggle('judge', event.target.checked)}
                        />
                      )}
                    />
                    {on('judge') ? <SectionFields id="judge" draft={draft} set={set} /> : null}
                    {field(
                      'controls',
                      'Control scenarios',
                      'Must keep passing whatever the change; comma separated. They count toward the scenarios per cycle.',
                      true,
                    )}
                    <SettingsField
                      label="Start a cycle for every finished execution"
                      description="Off, executions (imported CI runs among them) are data the loop reads, and a cycle starts only when you ask for one."
                      controlSize="fit"
                      renderControl={(props) => (
                        <Switch
                          {...props}
                          checked={draft.admit_executions}
                          onChange={(event) => set({ admit_executions: event.target.checked })}
                        />
                      )}
                    />
                  </SettingsList>
                  {json === null ? (
                    <Button
                      variant="ghost"
                      size="sm"
                      className="eval-ui-start"
                      onClick={() => setJson(JSON.stringify(configFromDraft(draft) ?? stored?.config ?? {}, null, 2))}
                    >
                      Edit as JSON
                    </Button>
                  ) : (
                    <>
                      <TextRow label="LoopConfig JSON" value={json} onChange={setJson} rows={16} />
                      <div className="eval-ui-actions">
                        <Button variant="ghost" onClick={applyJson}>
                          Apply to form
                        </Button>
                        <Button variant="ghost" onClick={() => setJson(null)}>
                          Discard JSON
                        </Button>
                      </div>
                    </>
                  )}
                </SettingsSection>
              </section>
            </div>
          </div>
          <StatusBar
            aria-label="Settings status"
            end={frozen ? <span>{frozen} running cycle(s) froze this revision</span> : null}
          >
            {stored ? (
              <>
                <span className="eval-ui-mono">revision {stored.revision.slice(0, 8)}</span>
                <span>saved {ago(stored.updated_at)}</span>
                <span className="eval-ui-mono">rules {state?.rules_version}</span>
              </>
            ) : (
              <span>Not configured: cycles map coverage but dispatch nothing until this is saved.</span>
            )}
          </StatusBar>
        </PageMain>
      </PageBody>
    </>
  )
}

/** One optional `LoopConfigV1` section's fields, as its spec lists them. */
function SectionFields({
  id,
  draft,
  set,
}: {
  id: SectionId
  draft: ConfigDraft
  set: (patch: Partial<ConfigDraft>) => void
}) {
  const value = draft.sections[id] ?? {}
  const spec = SECTIONS.find((section) => section.id === id)
  const change = (key: string, next: string) =>
    set({ sections: { ...draft.sections, [id]: { ...value, [key]: next } } })
  return (
    <>
      {spec?.fields.map((entry) => (
        <SectionField
          key={entry.key}
          field={entry}
          value={value[entry.key] ?? ''}
          onChange={(next) => change(entry.key, next)}
        />
      ))}
    </>
  )
}

function SectionField({
  field,
  value,
  onChange,
}: {
  field: FieldSpec
  value: string
  onChange: (next: string) => void
}) {
  const label = field.optional ? `${field.label} (optional)` : field.label
  if (field.kind === 'multiline')
    return <TextRow label={label} description={field.hint} value={value} onChange={onChange} />
  return (
    <SettingsField
      label={label}
      description={field.hint}
      controlSize={
        field.kind === 'where' ? 'fit' : field.kind === 'int' || field.kind === 'number' ? 'compact' : 'default'
      }
      renderControl={(props) =>
        field.kind === 'where' ? (
          <SegmentedControl<Where | ''>
            aria-label={field.label}
            variant="radio"
            value={value as Where | ''}
            onChange={onChange}
            options={WHERE.filter((where) => where.value !== 'harness')}
          />
        ) : field.kind === 'thinking' ? (
          <Select<ThinkingLevel>
            id={props.id}
            value={(value || undefined) as ThinkingLevel | undefined}
            options={THINKING}
            onChange={onChange}
            placeholder="Provider default"
            allowEmpty
            emptyLabel="Provider default"
            onClear={() => onChange('')}
          />
        ) : (
          <Input
            {...props}
            value={value}
            onChange={onChange}
            className="eval-ui-mono-input"
            autoComplete="off"
            spellCheck={false}
          />
        )
      }
    />
  )
}

function TextRow({
  label,
  description,
  value,
  onChange,
  rows = 6,
}: {
  label: ReactNode
  description?: ReactNode
  value: string
  onChange: (next: string) => void
  rows?: number
}) {
  return (
    <SettingsField
      label={label}
      description={description}
      layout="stacked"
      controlSize="full"
      renderControl={(props) => (
        <textarea
          {...props}
          className="eval-ui-textarea"
          value={value}
          rows={rows}
          spellCheck={false}
          onChange={(event) => onChange(event.target.value)}
        />
      )}
    />
  )
}
