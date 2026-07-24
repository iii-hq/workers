import { useEffect, useMemo, useRef, useState } from 'react'
import { Button, Input, Select, StatusPanel, Tabs, TabsList, TabsTrigger } from '@iii-dev/console-ui'
import { errorMessage } from '../api'
import { Field, TextArea } from '../components'
import { buildRequest, DEFAULT_FORM, type EvalFormState, type SystemPromptSource } from '../form'
import type { CatalogModel, EvalRequest } from '../types'

const MODEL_SEPARATOR = '\u0000'
const SYSTEM_PROMPT_SOURCE_OPTIONS = [
  { value: 'default', label: 'current default' },
  { value: 'custom', label: 'custom' },
  { value: 'none', label: 'none' },
]

export function NewEvaluationForm({
  models,
  modelCatalogError,
  onLoadDefaultSystemPrompt,
  onCreate,
}: {
  models: CatalogModel[]
  modelCatalogError: string | null
  onLoadDefaultSystemPrompt: (provider?: string) => Promise<string | null>
  onCreate: (request: EvalRequest) => Promise<void>
}) {
  const formRef = useRef<HTMLFormElement>(null)
  const [form, setForm] = useState<EvalFormState>({ ...DEFAULT_FORM })
  const [errors, setErrors] = useState<Record<string, string>>({})
  const [submitting, setSubmitting] = useState(false)
  const [submitError, setSubmitError] = useState<string | null>(null)
  const [manualModel, setManualModel] = useState(models.length === 0)
  const [loadingDefaultFor, setLoadingDefaultFor] = useState<
    'sharedSystemPrompt' | 'controlSystemPrompt' | 'treatmentSystemPrompt' | null
  >(null)
  const [defaultPromptError, setDefaultPromptError] = useState<string | null>(null)

  useEffect(() => {
    if (models.length > 0 && !form.model) setManualModel(false)
  }, [form.model, models.length])

  const groups = useMemo(() => {
    const providers = new Map<string, Array<{ value: string; label: string; title?: string }>>()
    for (const model of models) {
      const options = providers.get(model.provider) ?? []
      options.push({
        value: `${model.provider}${MODEL_SEPARATOR}${model.id}`,
        label: model.display_name ?? model.id,
        title: `${model.id} · context ${model.context_window}`,
      })
      providers.set(model.provider, options)
    }
    return [...providers.entries()]
      .sort(([left], [right]) => left.localeCompare(right))
      .map(([label, options]) => ({
        label,
        options: options.sort((left, right) => left.label.localeCompare(right.label)),
      }))
  }, [models])

  const selectedModel = form.model ? `${form.provider}${MODEL_SEPARATOR}${form.model}` : undefined
  const update = <K extends keyof EvalFormState>(key: K, value: EvalFormState[K]) => {
    setForm((current) => ({ ...current, [key]: value }))
    if (key === 'model' || key === 'provider') setDefaultPromptError(null)
    setErrors((current) => {
      const relatedKey =
        key === 'sharedSystemPromptSource'
          ? 'sharedSystemPrompt'
          : key === 'controlSystemPromptSource'
            ? 'controlSystemPrompt'
            : key === 'treatmentSystemPromptSource'
              ? 'treatmentSystemPrompt'
              : undefined
      if (!current[key] && (!relatedKey || !current[relatedKey])) return current
      const next = { ...current }
      delete next[key]
      if (relatedKey) delete next[relatedKey]
      return next
    })
  }

  const submit = async (event: React.FormEvent) => {
    event.preventDefault()
    const built = buildRequest(form)
    if (!built.request) {
      setErrors(built.errors)
      window.requestAnimationFrame(() => {
        const firstError = formRef.current?.querySelector<HTMLElement>('.eval-ui-field-error')
        if (!firstError) return
        const details = firstError.closest('details')
        if (details) details.open = true
        window.requestAnimationFrame(() => {
          firstError.scrollIntoView({ behavior: 'smooth', block: 'center' })
          firstError
            .closest('label')
            ?.querySelector<HTMLElement>('input, textarea, button, [tabindex]:not([tabindex="-1"])')
            ?.focus()
        })
      })
      return
    }
    setSubmitting(true)
    setSubmitError(null)
    try {
      await onCreate(built.request)
    } catch (error) {
      setSubmitError(errorMessage(error))
    } finally {
      setSubmitting(false)
    }
  }

  const loadDefaultSystemPrompt = async (
    target: 'sharedSystemPrompt' | 'controlSystemPrompt' | 'treatmentSystemPrompt',
  ) => {
    if (form.systemPromptStrategy !== 'override') return
    const provider = form.provider.trim()
    if (!provider) {
      setDefaultPromptError('Select a catalog model or enter its provider before loading the current default.')
      return
    }
    setLoadingDefaultFor(target)
    setDefaultPromptError(null)
    try {
      const prompt = await onLoadDefaultSystemPrompt(provider)
      if (!prompt) {
        setDefaultPromptError(
          'No provider prompt is exposed by the router. Choose “current default” to use the prompt resolved by the harness.',
        )
        return
      }
      update(target, prompt)
    } catch (error) {
      setDefaultPromptError(errorMessage(error))
    } finally {
      setLoadingDefaultFor(null)
    }
  }

  const comparison =
    form.dimension === 'prompt'
      ? {
          title: 'prompt comparison',
          changedLabel: 'prompt',
          sharedLabel: 'system prompt',
          sharedHint: 'The same system prompt is used for every A and B run. Leave empty to use the harness default.',
          sharedPlaceholder: 'You are a precise assistant…',
          baselineHint: 'The current prompt you want to measure.',
          candidateHint: 'The proposed prompt you want to compare.',
        }
      : {
          title: 'system prompt comparison',
          changedLabel: 'system prompt',
          sharedLabel: 'user prompt',
          sharedHint: 'The same user prompt is used for every A and B run.',
          sharedPlaceholder: 'Build a concise summary of…',
          baselineHint: 'The current system prompt you want to measure.',
          candidateHint: 'The proposed system prompt you want to compare.',
        }
  const usesCustomSystemPrompt =
    form.dimension === 'prompt'
      ? form.sharedSystemPromptSource === 'custom'
      : form.controlSystemPromptSource === 'custom' || form.treatmentSystemPromptSource === 'custom'
  const sharedSystemPromptSummary =
    form.sharedSystemPromptSource === 'default'
      ? 'using current default'
      : form.sharedSystemPromptSource === 'none'
        ? 'disabled'
        : `custom · ${form.systemPromptStrategy}`

  return (
    <form ref={formRef} className="eval-ui-form" onSubmit={submit}>
      <div className="eval-ui-section-head">
        <div>
          <h1>new {comparison.title}</h1>
          <p>Change one thing. The model, success criteria, and run settings stay identical.</p>
        </div>
      </div>

      {submitError ? <StatusPanel variant="alert" headline="evaluation could not start" detail={submitError} /> : null}
      {Object.keys(errors).length > 0 ? (
        <div className="eval-ui-error-summary" role="alert">
          <strong>
            Fix {Object.keys(errors).length} {Object.keys(errors).length === 1 ? 'field' : 'fields'} to run this
            comparison.
          </strong>
          <span>{[...new Set(Object.values(errors))].join(' · ')}</span>
        </div>
      ) : null}

      <section className="eval-ui-panel">
        <div className="eval-ui-step-head">
          <span className="eval-ui-step-number">1</span>
          <div>
            <div className="eval-ui-panel-title">choose what changes</div>
            <p>Only the selected input differs between A and B.</p>
          </div>
        </div>
        <Tabs
          value={form.dimension}
          onValueChange={(value) => {
            update('dimension', value as EvalFormState['dimension'])
            setErrors({})
          }}
        >
          <TabsList>
            <TabsTrigger value="prompt">prompt text</TabsTrigger>
            <TabsTrigger value="system_prompt">system prompt</TabsTrigger>
          </TabsList>
        </Tabs>
        <div className="eval-ui-comparison-rule">
          <strong>{comparison.changedLabel}</strong> changes between A and B<span aria-hidden="true">·</span>
          <strong>{comparison.sharedLabel}</strong> stays the same
        </div>

        <div className="eval-ui-variants">
          <VariantEditor
            marker="A"
            role="baseline"
            label={form.controlLabel}
            value={form.dimension === 'prompt' ? form.controlPrompt : form.controlSystemPrompt}
            valueLabel={comparison.changedLabel}
            hint={comparison.baselineHint}
            source={form.dimension === 'system_prompt' ? form.controlSystemPromptSource : undefined}
            valueError={form.dimension === 'prompt' ? errors.controlPrompt : errors.controlSystemPrompt}
            onLabel={(value) => update('controlLabel', value)}
            onValue={(value) =>
              form.dimension === 'prompt' ? update('controlPrompt', value) : update('controlSystemPrompt', value)
            }
            onSource={(source) => update('controlSystemPromptSource', source)}
            onLoadDefault={
              form.dimension === 'system_prompt' &&
              form.controlSystemPromptSource === 'custom' &&
              form.systemPromptStrategy === 'override'
                ? () => void loadDefaultSystemPrompt('controlSystemPrompt')
                : undefined
            }
            loadingDefault={loadingDefaultFor === 'controlSystemPrompt'}
            defaultAvailable={Boolean(form.provider.trim())}
          />
          <VariantEditor
            marker="B"
            role="candidate"
            label={form.treatmentLabel}
            value={form.dimension === 'prompt' ? form.treatmentPrompt : form.treatmentSystemPrompt}
            valueLabel={comparison.changedLabel}
            hint={comparison.candidateHint}
            source={form.dimension === 'system_prompt' ? form.treatmentSystemPromptSource : undefined}
            valueError={form.dimension === 'prompt' ? errors.treatmentPrompt : errors.treatmentSystemPrompt}
            onLabel={(value) => update('treatmentLabel', value)}
            onValue={(value) =>
              form.dimension === 'prompt' ? update('treatmentPrompt', value) : update('treatmentSystemPrompt', value)
            }
            onSource={(source) => update('treatmentSystemPromptSource', source)}
            onLoadDefault={
              form.dimension === 'system_prompt' &&
              form.treatmentSystemPromptSource === 'custom' &&
              form.systemPromptStrategy === 'override'
                ? () => void loadDefaultSystemPrompt('treatmentSystemPrompt')
                : undefined
            }
            loadingDefault={loadingDefaultFor === 'treatmentSystemPrompt'}
            defaultAvailable={Boolean(form.provider.trim())}
          />
        </div>

        {form.dimension === 'prompt' ? (
          <details className="eval-ui-secondary-input">
            <summary>
              <span>system prompt</span>
              <span>optional · {sharedSystemPromptSummary}</span>
            </summary>
            <div className="eval-ui-secondary-input-body">
              <Field label="system prompt source">
                <Select
                  value={form.sharedSystemPromptSource}
                  options={SYSTEM_PROMPT_SOURCE_OPTIONS}
                  onChange={(value) => update('sharedSystemPromptSource', value as SystemPromptSource)}
                />
              </Field>
              {form.sharedSystemPromptSource === 'custom' ? (
                <>
                  <Field
                    label="apply custom system prompt as"
                    hint={
                      form.systemPromptStrategy === 'override'
                        ? 'Replaces the harness default.'
                        : 'Appends this text to the harness default without loading it here.'
                    }
                  >
                    <Select
                      value={form.systemPromptStrategy}
                      options={[
                        { value: 'override', label: 'override' },
                        { value: 'enrich', label: 'enrich' },
                      ]}
                      onChange={(value) =>
                        update('systemPromptStrategy', value as EvalFormState['systemPromptStrategy'])
                      }
                    />
                  </Field>
                  {form.systemPromptStrategy === 'override' ? (
                    <div className="eval-ui-load-default">
                      <span className="eval-ui-label">starting point</span>
                      <Button
                        variant="ghost"
                        size="sm"
                        type="button"
                        disabled={loadingDefaultFor !== null || !form.provider.trim()}
                        onClick={() => void loadDefaultSystemPrompt('sharedSystemPrompt')}
                      >
                        {loadingDefaultFor === 'sharedSystemPrompt'
                          ? 'loading…'
                          : form.provider.trim()
                            ? 'load current default'
                            : 'select model to load default'}
                      </Button>
                      <span className="eval-ui-hint">Uses the selected provider, or the router default.</span>
                    </div>
                  ) : null}
                  <Field label="custom system prompt" error={errors.sharedSystemPrompt} hint={comparison.sharedHint}>
                    <TextArea
                      value={form.sharedSystemPrompt}
                      onChange={(event) => update('sharedSystemPrompt', event.target.value)}
                      rows={6}
                      placeholder={comparison.sharedPlaceholder}
                    />
                  </Field>
                </>
              ) : (
                <SystemPromptSourceState source={form.sharedSystemPromptSource} />
              )}
            </div>
          </details>
        ) : (
          <>
            {usesCustomSystemPrompt ? (
              <div className="eval-ui-system-prompt-mode">
                <Field
                  label="apply custom system prompts as"
                  hint={
                    form.systemPromptStrategy === 'override'
                      ? 'Custom values replace the harness default.'
                      : 'Custom values are appended to the harness default without loading it into either field.'
                  }
                >
                  <Select
                    value={form.systemPromptStrategy}
                    options={[
                      { value: 'override', label: 'override' },
                      { value: 'enrich', label: 'enrich' },
                    ]}
                    onChange={(value) => update('systemPromptStrategy', value as EvalFormState['systemPromptStrategy'])}
                  />
                </Field>
              </div>
            ) : null}
            <div className="eval-ui-shared-editor">
              <div className="eval-ui-shared-editor-head">
                <span>shared by A + B</span>
                <strong>{comparison.sharedLabel}</strong>
              </div>
              <Field error={errors.sharedUserPrompt} hint={comparison.sharedHint}>
                <TextArea
                  value={form.sharedUserPrompt}
                  onChange={(event) => update('sharedUserPrompt', event.target.value)}
                  rows={4}
                  placeholder={comparison.sharedPlaceholder}
                />
              </Field>
            </div>
          </>
        )}

        {defaultPromptError ? (
          <StatusPanel variant="warn" headline="default system prompt unavailable" detail={defaultPromptError} />
        ) : null}
      </section>

      <section className="eval-ui-panel">
        <div className="eval-ui-step-head">
          <span className="eval-ui-step-number">2</span>
          <div>
            <div className="eval-ui-panel-title">shared run settings</div>
            <p>The same model and limits are applied to A and B.</p>
          </div>
        </div>
        {models.length > 0 && !manualModel ? (
          <Field label="model" error={errors.model}>
            <Select
              value={selectedModel}
              groups={groups}
              placeholder="select a catalog model"
              onChange={(value) => {
                const [provider, model] = value.split(MODEL_SEPARATOR)
                setForm((current) => ({ ...current, provider, model }))
                setDefaultPromptError(null)
                setErrors((current) => {
                  const next = { ...current }
                  delete next.model
                  return next
                })
              }}
            />
          </Field>
        ) : (
          <div className="eval-ui-grid-2">
            <Field label="model" error={errors.model}>
              <Input
                value={form.model}
                onChange={(value) => update('model', value)}
                preserveCase
                placeholder="codex/gpt-5.6-luna"
              />
            </Field>
            <Field label="provider" hint="optional when model routing is unambiguous">
              <Input
                value={form.provider}
                onChange={(value) => update('provider', value)}
                preserveCase
                placeholder="openai-codex"
              />
            </Field>
          </div>
        )}
        <div className="eval-ui-inline-action">
          {models.length > 0 ? (
            <button type="button" className="eval-ui-link" onClick={() => setManualModel((current) => !current)}>
              {manualModel ? 'use model catalog' : 'enter model manually'}
            </button>
          ) : modelCatalogError ? (
            <span className="eval-ui-hint">model catalog unavailable — {modelCatalogError}</span>
          ) : (
            <span className="eval-ui-hint">model catalog is empty; enter a model manually.</span>
          )}
        </div>
        <div className="eval-ui-grid-3">
          <Field label="runs per variant" error={errors.runs}>
            <Input
              type="number"
              min={1}
              max={20}
              value={form.runs}
              onChange={(value) => update('runs', value)}
              preserveCase
            />
            <button
              type="button"
              className="eval-ui-link"
              onClick={() => update('runs', String(Math.max(2, Number(form.runs) || 1)))}
            >
              balanced A→B + B→A
            </button>
          </Field>
          <Field
            label="max total tokens"
            error={errors.maxTotalTokens}
            hint="optional; blank means no total-token limit"
          >
            <Input
              type="number"
              min={1}
              value={form.maxTotalTokens}
              onChange={(value) => update('maxTotalTokens', value)}
              preserveCase
              placeholder="no limit"
            />
          </Field>
          <Field label="max cost (USD)" error={errors.maxCostUsd} hint="optional; requires catalog pricing">
            <Input
              type="number"
              min={0}
              step="0.000001"
              value={form.maxCostUsd}
              onChange={(value) => update('maxCostUsd', value)}
              preserveCase
              placeholder="no cost limit"
            />
          </Field>
        </div>
      </section>

      <details className="eval-ui-advanced eval-ui-success-criteria">
        <summary>success criteria (optional)</summary>
        <div className="eval-ui-advanced-body">
          <span className="eval-ui-hint">
            Leave the expected value or custom function empty to collect outputs and metrics without scoring either
            variant.
          </span>
          <Field label="judge each output with">
            <Select
              value={form.evaluatorMode}
              options={[
                {
                  value: 'normalized_text',
                  label: 'normalized text match',
                },
                { value: 'exact', label: 'exact value (strict)' },
                { value: 'custom', label: 'custom iii function' },
              ]}
              onChange={(value) => update('evaluatorMode', value as EvalFormState['evaluatorMode'])}
            />
          </Field>
          {form.evaluatorMode !== 'custom' ? (
            <>
              {form.evaluatorMode === 'exact' ? (
                <Field label="expected format">
                  <Select
                    value={form.expectedFormat}
                    options={[
                      { value: 'text', label: 'text' },
                      { value: 'json', label: 'JSON' },
                    ]}
                    onChange={(value) => update('expectedFormat', value as EvalFormState['expectedFormat'])}
                  />
                </Field>
              ) : null}
              <Field label="expected value" error={errors.expectedValue}>
                <TextArea
                  value={form.expectedValue}
                  onChange={(event) => update('expectedValue', event.target.value)}
                  rows={4}
                  placeholder={form.evaluatorMode === 'exact' && form.expectedFormat === 'json' ? '{"ok":true}' : 'OK'}
                />
                {form.evaluatorMode === 'normalized_text' ? (
                  <span className="eval-ui-hint">
                    Ignores letter case, repeated whitespace, and surrounding punctuation.
                  </span>
                ) : null}
              </Field>
            </>
          ) : (
            <div className="eval-ui-grid-2">
              <Field
                label="evaluator function id"
                error={errors.evaluatorFunctionId}
                hint="receives output, metrics, run identity and arguments"
              >
                <Input
                  value={form.evaluatorFunctionId}
                  onChange={(value) => update('evaluatorFunctionId', value)}
                  preserveCase
                  placeholder="my-eval::assert"
                />
              </Field>
              <Field label="arguments JSON" error={errors.evaluatorArguments}>
                <TextArea
                  className="code"
                  value={form.evaluatorArguments}
                  onChange={(event) => update('evaluatorArguments', event.target.value)}
                  rows={5}
                />
              </Field>
            </div>
          )}
        </div>
      </details>

      <details className="eval-ui-advanced">
        <summary>advanced harness options</summary>
        <div className="eval-ui-advanced-body">
          <div className="eval-ui-grid-3">
            <Field label="invocation timeout (s)" error={errors.invocationTimeoutSeconds} hint="optional">
              <Input
                type="number"
                min={1}
                value={form.invocationTimeoutSeconds}
                onChange={(value) => update('invocationTimeoutSeconds', value)}
                preserveCase
                placeholder="default: 120"
              />
            </Field>
            <Field label="scenario timeout (s)" error={errors.scenarioTimeoutSeconds} hint="optional">
              <Input
                type="number"
                min={1}
                value={form.scenarioTimeoutSeconds}
                onChange={(value) => update('scenarioTimeoutSeconds', value)}
                preserveCase
                placeholder="default: 600"
              />
            </Field>
            <Field label="max turns" error={errors.maxTurns} hint="optional">
              <Input
                type="number"
                min={1}
                value={form.maxTurns}
                onChange={(value) => update('maxTurns', value)}
                preserveCase
                placeholder="default: 1000"
              />
            </Field>
            <Field label="max output tokens / call" error={errors.maxOutputTokensPerCall} hint="optional">
              <Input
                type="number"
                min={1}
                value={form.maxOutputTokensPerCall}
                onChange={(value) => update('maxOutputTokensPerCall', value)}
                preserveCase
                placeholder="default: 8192"
              />
            </Field>
            <Field
              label="max function-call errors"
              error={errors.maxFunctionCallErrors}
              hint="optional; blank means no limit"
            >
              <Input
                type="number"
                min={0}
                value={form.maxFunctionCallErrors}
                onChange={(value) => update('maxFunctionCallErrors', value)}
                preserveCase
                placeholder="no limit"
              />
            </Field>
            <Field label="max error spans" error={errors.maxErrorSpans} hint="optional">
              <Input
                type="number"
                min={0}
                value={form.maxErrorSpans}
                onChange={(value) => update('maxErrorSpans', value)}
                preserveCase
                placeholder="no span limit"
              />
            </Field>
          </div>
          <div className="eval-ui-grid-2">
            <Field label="mode">
              <Select
                value={form.mode || undefined}
                options={[
                  { value: 'ask', label: 'ask' },
                  { value: 'agent', label: 'agent' },
                ]}
                allowEmpty
                emptyLabel="default"
                onClear={() => update('mode', '')}
                onChange={(value) => update('mode', value as EvalFormState['mode'])}
              />
            </Field>
            <Field label="thinking level">
              <Select
                value={form.thinkingLevel || undefined}
                options={['minimal', 'low', 'medium', 'high', 'xhigh'].map((value) => ({ value, label: value }))}
                allowEmpty
                emptyLabel="provider default"
                onClear={() => update('thinkingLevel', '')}
                onChange={(value) => update('thinkingLevel', value as EvalFormState['thinkingLevel'])}
              />
            </Field>
          </div>
          <JsonField
            label="functions policy"
            value={form.functionsJson}
            error={errors.functionsJson}
            hint="Optional. The preset matches chat; blank uses the harness default and denies function calls."
            onChange={(value) => update('functionsJson', value)}
          />
          <JsonField
            label="output contract"
            value={form.outputJson}
            error={errors.outputJson}
            hint="Optional. Blank uses the harness text-output default."
            onChange={(value) => update('outputJson', value)}
          />
          <JsonField
            label="provider options"
            value={form.providerOptionsJson}
            error={errors.providerOptionsJson}
            placeholder="{}"
            onChange={(value) => update('providerOptionsJson', value)}
          />
          <JsonField
            label="metadata"
            value={form.metadataJson}
            error={errors.metadataJson}
            placeholder='{"fs_scope":{"root":"/workspace"}}'
            onChange={(value) => update('metadataJson', value)}
          />
        </div>
      </details>

      <div className="eval-ui-submit">
        <span>
          Runs A {form.runs || '0'}× and B {form.runs || '0'}× in alternating order.
        </span>
        <Button variant="primary" size="md" type="submit" disabled={submitting}>
          {submitting ? 'starting…' : 'run comparison'}
        </Button>
      </div>
    </form>
  )
}

function VariantEditor({
  marker,
  role,
  label,
  value,
  valueLabel,
  hint,
  source,
  valueError,
  onLabel,
  onValue,
  onSource,
  onLoadDefault,
  loadingDefault = false,
  defaultAvailable = true,
}: {
  marker: string
  role: string
  label: string
  value: string
  valueLabel: string
  hint: string
  source?: SystemPromptSource
  valueError?: string
  onLabel: (value: string) => void
  onValue: (value: string) => void
  onSource?: (source: SystemPromptSource) => void
  onLoadDefault?: () => void
  loadingDefault?: boolean
  defaultAvailable?: boolean
}) {
  return (
    <div className="eval-ui-variant">
      <div className="eval-ui-variant-head">
        <span className="eval-ui-variant-marker">{marker}</span>
        <div>
          <div className="eval-ui-variant-title">{role}</div>
          <p>{hint}</p>
        </div>
      </div>
      <Field label="name (optional)">
        <Input value={label} onChange={onLabel} preserveCase />
      </Field>
      {source && onSource ? (
        <Field label={`${valueLabel} source`} error={source === 'custom' ? undefined : valueError}>
          <Select
            value={source}
            options={SYSTEM_PROMPT_SOURCE_OPTIONS}
            onChange={(nextSource) => onSource(nextSource as SystemPromptSource)}
          />
        </Field>
      ) : null}
      {source === undefined || source === 'custom' ? (
        <>
          <Field label={source ? `custom ${valueLabel}` : valueLabel} error={valueError}>
            <TextArea value={value} onChange={(event) => onValue(event.target.value)} rows={10} />
          </Field>
          {onLoadDefault ? (
            <button
              type="button"
              className="eval-ui-link eval-ui-load-variant-default"
              disabled={loadingDefault || !defaultAvailable}
              onClick={onLoadDefault}
            >
              {loadingDefault
                ? 'loading current default…'
                : defaultAvailable
                  ? 'load current default'
                  : 'select model to load default'}
            </button>
          ) : null}
        </>
      ) : (
        <SystemPromptSourceState source={source} />
      )}
    </div>
  )
}

function SystemPromptSourceState({ source }: { source: Exclude<SystemPromptSource, 'custom'> }) {
  return (
    <div className="eval-ui-system-prompt-state">
      <strong>{source === 'default' ? 'Current default' : 'No system prompt'}</strong>
      <span>
        {source === 'default'
          ? 'Uses the system prompt resolved by the selected provider and harness.'
          : 'Disables the provider and harness system prompt for this variant.'}
      </span>
    </div>
  )
}

function JsonField({
  label,
  value,
  error,
  hint,
  placeholder,
  onChange,
}: {
  label: string
  value: string
  error?: string
  hint?: string
  placeholder?: string
  onChange: (value: string) => void
}) {
  return (
    <Field label={label} error={error} hint={hint}>
      <TextArea
        className="code"
        value={value}
        onChange={(event) => onChange(event.target.value)}
        placeholder={placeholder}
        rows={4}
      />
    </Field>
  )
}
