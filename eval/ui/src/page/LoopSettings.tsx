import { Button, Checkbox, Input, SegmentedControl, Select, StatusPanel } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { ArrowLeft } from 'lucide-react'
import { type FormEvent, type ReactNode, useEffect, useId, useState } from 'react'
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
  type SectionSpec,
} from '../config-draft'
import { validateConfig } from '../model'
import type { ConfigResponse, StoredConfig, ThinkingLevel, Where } from '../types'

const WHERE = [
  { value: 'harness' as const, label: 'Harness', icon: false as const },
  { value: 'docker' as const, label: 'Docker', icon: false as const },
  { value: 'github' as const, label: 'GitHub', icon: false as const },
]

/** `eval::config` / `eval::configure`: the loop's `LoopConfigV1`, nothing else. */
export function LoopSettings({
  api,
  state,
  error: readError,
  onSaved,
  onClose,
  onDirtyChange,
}: {
  api: EvalApi
  state: ConfigResponse | null
  error: string | null
  onSaved: (stored: StoredConfig) => void
  onClose?: () => void
  onDirtyChange: (dirty: boolean) => void
}) {
  const stored = state?.config ?? null
  const [draft, setDraft] = useState<ConfigDraft>(() => (stored ? draftFromConfig(stored.config) : EMPTY_DRAFT))
  const [dirty, setDirty] = useState(false)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [json, setJson] = useState<string | null>(null)

  // A clean form always shows what is stored, including a change made elsewhere.
  useEffect(() => {
    if (!dirty) setDraft(stored ? draftFromConfig(stored.config) : EMPTY_DRAFT)
  }, [dirty, stored])

  useEffect(() => onDirtyChange(dirty), [dirty, onDirtyChange])

  const set = (patch: Partial<ConfigDraft>) => {
    setDraft((previous) => ({ ...previous, ...patch }))
    setDirty(true)
    setError(null)
  }

  async function save(event: FormEvent) {
    event.preventDefault()
    const config = configFromDraft(draft)
    const problem = config ? validateConfig(config) : 'choose where the E2E runs'
    if (!config || problem) {
      setError(problem)
      return
    }
    setSaving(true)
    setError(null)
    try {
      const saved = await api.configure(config)
      setDirty(false)
      onSaved(saved)
    } catch (failure) {
      setError(errorMessage(failure))
    } finally {
      setSaving(false)
    }
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

  const needsStack = draft.where === 'docker' || draft.where === 'github'

  return (
    <form
      className="eval-ui-detail eval-ui-settings"
      aria-labelledby="eval-ui-settings-title"
      onSubmit={(e) => void save(e)}
    >
      {onClose ? (
        <Button type="button" variant="ghost" size="lg" className="eval-ui-back" onClick={onClose}>
          <ArrowLeft size={16} aria-hidden /> Cycles
        </Button>
      ) : null}
      <header className="eval-ui-head">
        <h2 id="eval-ui-settings-title" className="eval-ui-title">
          Loop settings
        </h2>
        <p className="eval-ui-faint">
          {stored ? (
            <>
              Revision <span className="eval-ui-mono">{stored.revision.slice(0, 12)}</span> · saved{' '}
              {new Date(stored.updated_at).toLocaleString()} · rules{' '}
              <span className="eval-ui-mono">{state?.rules_version}</span>
            </>
          ) : (
            'Not configured: cycles map coverage but dispatch nothing until this is saved.'
          )}
        </p>
      </header>
      {readError ? <StatusPanel variant="alert" headline="Couldn't read the settings" detail={readError} /> : null}
      <p className="eval-ui-hint">A save is a new policy revision; running cycles keep the plan they froze.</p>

      <Group title="Subject">
        <Field label="Provider" value={draft.provider} onChange={(provider) => set({ provider })} mono />
        <Field label="Model" value={draft.model} onChange={(model) => set({ model })} mono />
        <Field
          label="Agent profile"
          hint="Optional; empty uses the E2E default."
          value={draft.agent}
          onChange={(agent) => set({ agent })}
          mono
        />
      </Group>

      <Group title="Where the E2E runs">
        <SegmentedControl<Where | ''>
          aria-label="Where the E2E runs"
          variant="radio"
          value={draft.where}
          onChange={(where) => set(where === 'harness' ? { where, stackName: '', stackYaml: '' } : { where })}
          options={WHERE}
        />
        {needsStack ? (
          <>
            <Field label="Stack name" value={draft.stackName} onChange={(stackName) => set({ stackName })} mono />
            <TextArea label="Stack YAML" value={draft.stackYaml} onChange={(stackYaml) => set({ stackYaml })} />
          </>
        ) : null}
      </Group>

      <Group title="Sampling">
        <Field
          label="Runs per side"
          numeric
          value={draft.runs_per_side}
          onChange={(runs_per_side) => set({ runs_per_side })}
        />
        <Field
          label="Technical retries"
          numeric
          value={draft.technical_retries}
          onChange={(technical_retries) => set({ technical_retries })}
        />
        <Field
          label="Max scenarios per plan"
          numeric
          value={draft.max_scenarios}
          onChange={(max_scenarios) => set({ max_scenarios })}
        />
        <Field
          label="Estimated USD per run"
          hint="Reserved before dispatch; a run with no reported cost stays charged at this."
          numeric
          value={draft.estimated_usd_per_run}
          onChange={(estimated_usd_per_run) => set({ estimated_usd_per_run })}
        />
      </Group>

      <Group title="Minimum effect">
        <Field
          label="Tokens per run"
          numeric
          value={draft.min_effect_tokens}
          onChange={(min_effect_tokens) => set({ min_effect_tokens })}
        />
        <Field
          label="Pass rate (pp)"
          numeric
          value={draft.min_effect_pass_rate_pp}
          onChange={(min_effect_pass_rate_pp) => set({ min_effect_pass_rate_pp })}
        />
      </Group>

      <Group title="Budget per cycle">
        <Field
          label="Max spend (USD)"
          numeric
          value={draft.max_spend_usd}
          onChange={(max_spend_usd) => set({ max_spend_usd })}
        />
        <Field
          label="Max duration (ms)"
          numeric
          value={draft.max_duration_ms}
          onChange={(max_duration_ms) => set({ max_duration_ms })}
        />
        <Field label="Max calls" numeric value={draft.max_calls} onChange={(max_calls) => set({ max_calls })} />
        <Field
          label="Max candidates"
          numeric
          value={draft.max_candidates}
          onChange={(max_candidates) => set({ max_candidates })}
        />
      </Group>

      <Group title="Controls">
        <Field
          label="Control scenarios"
          hint="Scenario ids every plan also runs and preserves, comma separated; empty for none."
          value={draft.controls}
          onChange={(controls) => set({ controls })}
          mono
        />
      </Group>

      {SECTIONS.map((section) => (
        <OptionalSection
          key={section.id}
          spec={section}
          value={draft.sections[section.id]}
          onChange={(value) => set({ sections: { ...draft.sections, [section.id]: value } })}
        >
          {section.id === 'promotion' ? (
            <>
              <Field
                label="Release build estimate (USD)"
                hint="What one official rc or finalization build is reserved at; its cost is not reported."
                numeric
                value={draft.release_build_estimate_usd}
                onChange={(release_build_estimate_usd) => set({ release_build_estimate_usd })}
              />
              <p className="eval-ui-hint">
                GitHub and the Registry are the official services; the Release Control URL and both credentials come
                from the deployment (EVAL_RELEASE_CONTROL_URL, EVAL_GITHUB_TOKEN, EVAL_RELEASE_CONTROL_SESSION), never
                from this form.
              </p>
            </>
          ) : null}
        </OptionalSection>
      ))}

      <Group title="Advanced">
        {json === null ? (
          <Button
            type="button"
            variant="ghost"
            onClick={() => setJson(JSON.stringify(configFromDraft(draft) ?? stored?.config ?? {}, null, 2))}
          >
            Edit as JSON
          </Button>
        ) : (
          <>
            <TextArea label="LoopConfig JSON" value={json} onChange={setJson} rows={16} />
            <div className="eval-ui-actions">
              <Button type="button" variant="ghost" onClick={applyJson}>
                Apply to form
              </Button>
              <Button type="button" variant="ghost" onClick={() => setJson(null)}>
                Discard JSON
              </Button>
            </div>
          </>
        )}
      </Group>

      {error ? (
        <p className="eval-ui-form-error" role="alert">
          {error}
        </p>
      ) : null}
      <div className="eval-ui-actions">
        <Button type="submit" variant="primary" disabled={saving || !dirty}>
          {saving ? 'Saving…' : 'Save settings'}
        </Button>
        {dirty && stored ? (
          <Button
            type="button"
            variant="ghost"
            onClick={() => {
              setDraft(draftFromConfig(stored.config))
              setDirty(false)
              setError(null)
            }}
          >
            Revert
          </Button>
        ) : null}
      </div>
    </form>
  )
}

function Group({ title, children }: { title: string; children: ReactNode }) {
  return (
    <fieldset className="eval-ui-section eval-ui-fieldset">
      <legend className="eval-ui-legend">{title}</legend>
      {children}
    </fieldset>
  )
}

function Field({
  label,
  hint,
  value,
  onChange,
  numeric,
  mono,
}: {
  label: string
  hint?: string
  value: string
  onChange: (next: string) => void
  numeric?: boolean
  mono?: boolean
}) {
  const id = useId()
  const hintId = useId()
  return (
    <div className="eval-ui-field">
      <label className="eval-ui-label" htmlFor={id}>
        {label}
      </label>
      <Input
        id={id}
        value={value}
        onChange={onChange}
        inputMode={numeric ? 'decimal' : undefined}
        className={mono || numeric ? 'eval-ui-mono-input' : undefined}
        aria-describedby={hint ? hintId : undefined}
        autoComplete="off"
        spellCheck={false}
      />
      {hint ? (
        <span id={hintId} className="eval-ui-hint">
          {hint}
        </span>
      ) : null}
    </div>
  )
}

function TextArea({
  label,
  value,
  onChange,
  rows = 8,
}: {
  label: string
  value: string
  onChange: (next: string) => void
  rows?: number
}) {
  const id = useId()
  return (
    <div className="eval-ui-field">
      <label className="eval-ui-label" htmlFor={id}>
        {label}
      </label>
      <textarea
        id={id}
        className="eval-ui-textarea"
        value={value}
        rows={rows}
        spellCheck={false}
        onChange={(event) => onChange(event.target.value)}
      />
    </div>
  )
}

const THINKING: { value: ThinkingLevel; label: string }[] = [
  { value: 'off', label: 'off' },
  { value: 'minimal', label: 'minimal' },
  { value: 'low', label: 'low' },
  { value: 'medium', label: 'medium' },
  { value: 'high', label: 'high' },
  { value: 'xhigh', label: 'xhigh' },
]

/** One optional `LoopConfigV1` section: off is absent from the saved config. */
function OptionalSection({
  spec,
  value,
  onChange,
  children,
}: {
  spec: SectionSpec
  value: Record<string, string> | null
  onChange: (next: Record<string, string> | null) => void
  /** Fields of the section kept elsewhere in `LoopConfigV1`, shown while it is on. */
  children?: ReactNode
}) {
  return (
    <Group title={spec.title}>
      <Checkbox
        label={value ? 'On' : 'Off'}
        checked={value !== null}
        onChange={(event) => onChange(event.target.checked ? emptySection(spec.id) : null)}
        aria-label={`${spec.title} on`}
      />
      {value === null ? <p className="eval-ui-hint">{spec.off}</p> : null}
      {value
        ? spec.fields.map((field) => (
            <SectionField
              key={field.key}
              field={field}
              value={value[field.key] ?? ''}
              onChange={(next) => onChange({ ...value, [field.key]: next })}
            />
          ))
        : null}
      {value ? children : null}
    </Group>
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
  const id = useId()
  const label = field.optional ? `${field.label} (optional)` : field.label
  switch (field.kind) {
    case 'multiline':
      return <TextArea label={label} value={value} onChange={onChange} />
    case 'where':
      return (
        <div className="eval-ui-field">
          <span className="eval-ui-label">{label}</span>
          <SegmentedControl<Where | ''>
            aria-label={field.label}
            variant="radio"
            value={value as Where | ''}
            onChange={onChange}
            options={WHERE}
          />
        </div>
      )
    case 'thinking':
      return (
        <div className="eval-ui-field">
          <label className="eval-ui-label" htmlFor={id}>
            {label}
          </label>
          <Select<ThinkingLevel>
            id={id}
            value={(value || undefined) as ThinkingLevel | undefined}
            options={THINKING}
            onChange={onChange}
            placeholder="Provider default"
            allowEmpty
            emptyLabel="Provider default"
            onClear={() => onChange('')}
          />
        </div>
      )
    default:
      return (
        <Field
          label={label}
          hint={field.hint}
          value={value}
          onChange={onChange}
          numeric={field.kind === 'int' || field.kind === 'number'}
          mono
        />
      )
  }
}
