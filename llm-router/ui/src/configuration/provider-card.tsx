import {
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
  type JsonValue,
} from '@iii-dev/console-ui'
import { useState } from 'react'
import { FieldError } from './field-error'
import { errorAt } from './pointers'
import { apiKeyPlaceholder, type ProviderRuntimeStatus, suggestedEnvVar } from './provider-cards'

type JsonObject = { [key: string]: JsonValue }

function asString(v: JsonValue | undefined): string {
  return typeof v === 'string' ? v : ''
}

/** `${VAR}` (exactly one reference, nothing else) — the recommended shape. */
export function isEnvReference(v: string): boolean {
  return /^\$\{[A-Za-z_][A-Za-z0-9_]*\}$/.test(v.trim())
}

/** Mentions an env reference somewhere (partial templating still counts). */
function hasEnvReference(v: string): boolean {
  return /\$\{[A-Za-z_][A-Za-z0-9_]*\}/.test(v)
}

function statusClass(status: ProviderRuntimeStatus): string | null {
  if (status === 'unknown') return null
  if (status === 'loaded') return 'llmr-cfg-status is-loaded'
  if (status === 'not-loaded') return 'llmr-cfg-status is-not-loaded'
  return 'llmr-cfg-status is-not-connected'
}

function statusLabel(status: ProviderRuntimeStatus): string | null {
  if (status === 'unknown') return null
  if (status === 'loaded') return 'loaded'
  if (status === 'not-loaded') return 'not loaded'
  return 'not connected'
}

export function ProviderCard({
  id,
  label,
  status,
  isDefault,
  slice,
  promptDefault,
  errors,
  onChange,
  onSetDefault,
}: {
  id: string
  label: string
  status: ProviderRuntimeStatus
  isDefault: boolean
  slice: JsonObject
  promptDefault: string | null
  errors?: ReadonlyMap<string, string>
  onChange(next: JsonObject): void
  onSetDefault(): void
}) {
  const apiKey = asString(slice.api_key)
  const plainTextKey = apiKey.length > 0 && !isEnvReference(apiKey)
  const systemPrompt = slice.system_prompt
  const overridden = typeof systemPrompt === 'string'
  const advancedSet =
    asString(slice.api_url).length > 0 || typeof slice.max_tokens === 'number' || overridden
  const [advanced, setAdvanced] = useState(true)
  const [reveal, setReveal] = useState(false)
  const [promptOpen, setPromptOpen] = useState(false)
  const [promptDraft, setPromptDraft] = useState('')
  const statusCls = statusClass(status)
  const statusText = statusLabel(status)
  const effectivePrompt = overridden ? asString(systemPrompt) : promptDefault

  const set = (key: string, v: JsonValue | undefined) => {
    const next = { ...slice }
    if (v === undefined) delete next[key]
    else next[key] = v
    onChange(next)
  }

  const openPrompt = () => {
    setPromptDraft(overridden ? asString(systemPrompt) : (promptDefault ?? ''))
    setPromptOpen(true)
  }

  const applyPrompt = () => {
    set('system_prompt', promptDraft)
    setPromptOpen(false)
  }

  const clearPrompt = () => {
    set('system_prompt', undefined)
    setPromptOpen(false)
  }

  const masked = plainTextKey && !reveal

  return (
    <section className="llmr-cfg-card" data-field={`providers-${id}`}>
      <header className="llmr-cfg-card-head">
        <span className="llmr-cfg-card-title">{label}</span>
        <span className="llmr-cfg-id">{id}</span>
        {statusCls && statusText ? <span className={statusCls}>{statusText}</span> : null}
        {isDefault ? (
          <span className="llmr-cfg-default">default</span>
        ) : (
          <button type="button" className="llmr-cfg-toggle" onClick={onSetDefault}>
            set as default
          </button>
        )}
      </header>

      <label className="llmr-cfg-label" htmlFor={`llmr-${id}-api-key`}>
        api key
      </label>
      <div className="llmr-cfg-key-row">
        <input
          id={`llmr-${id}-api-key`}
          className="llmr-cfg-input"
          // Env references are not secrets — keep them readable; anything
          // else gets masked like the generic password field.
          type={masked ? 'password' : 'text'}
          autoComplete="off"
          spellCheck={false}
          value={apiKey}
          placeholder={apiKeyPlaceholder(id)}
          onChange={(e) => set('api_key', e.target.value || undefined)}
        />
        {plainTextKey ? (
          <button type="button" className="llmr-cfg-toggle" onClick={() => setReveal((v) => !v)}>
            {reveal ? 'hide' : 'show'}
          </button>
        ) : null}
      </div>
      <FieldError message={errorAt(errors, 'providers', id, 'api_key')} />
      {plainTextKey ? (
        <div className="llmr-cfg-warning" role="alert">
          <strong>plain-text secret</strong> — this key is stored verbatim in the configuration entry (and every export
          of it). Set it as an environment variable on the engine host and reference it instead:{' '}
          <button
            type="button"
            className="llmr-cfg-envfix"
            title="replace with the env reference"
            onClick={() => set('api_key', `\${${suggestedEnvVar(id)}}`)}
          >
            {'${'}
            {suggestedEnvVar(id)}
            {'}'}
          </button>{' '}
          — the value expands when the entry is read, so the secret never lands in the store.
        </div>
      ) : isEnvReference(apiKey) ? (
        <div className="llmr-cfg-envok">env reference — expands on read, secret stays out of the store</div>
      ) : hasEnvReference(apiKey) ? (
        <div className="llmr-cfg-warning" role="alert">
          partial env reference — the literal part is still stored as plain text. Move the whole key into one variable.
        </div>
      ) : null}

      <button
        type="button"
        className="llmr-cfg-toggle llmr-cfg-advanced"
        aria-expanded={advanced}
        onClick={() => setAdvanced((open) => !open)}
      >
        {advanced ? '▾ hide advanced' : advancedSet ? '▸ advanced · set' : '▸ advanced'}
      </button>

      {advanced ? (
        <>
          <label className="llmr-cfg-label" htmlFor={`llmr-${id}-api-url`}>
            api url
          </label>
          <input
            id={`llmr-${id}-api-url`}
            className="llmr-cfg-input"
            value={asString(slice.api_url)}
            placeholder="provider default"
            spellCheck={false}
            onChange={(e) => set('api_url', e.target.value || undefined)}
          />
          <FieldError message={errorAt(errors, 'providers', id, 'api_url')} />

          <label className="llmr-cfg-label" htmlFor={`llmr-${id}-max-tokens`}>
            max tokens
          </label>
          <input
            id={`llmr-${id}-max-tokens`}
            className="llmr-cfg-input"
            inputMode="numeric"
            value={typeof slice.max_tokens === 'number' ? String(slice.max_tokens) : ''}
            placeholder="provider default"
            onChange={(e) => {
              const n = Number(e.target.value)
              set('max_tokens', e.target.value === '' || Number.isNaN(n) ? undefined : n)
            }}
          />
          <FieldError message={errorAt(errors, 'providers', id, 'max_tokens')} />

          <div className="llmr-cfg-prompt-head">
            <span className="llmr-cfg-label">system prompt</span>
            {overridden ? <span className="llmr-cfg-status is-overridden">overridden</span> : null}
            <button type="button" className="llmr-cfg-toggle" onClick={openPrompt}>
              {overridden ? 'edit' : 'override'}
            </button>
          </div>
          {effectivePrompt ? (
            <button
              type="button"
              className="llmr-cfg-prompt-preview"
              title={effectivePrompt}
              onClick={openPrompt}
            >
              {effectivePrompt}
            </button>
          ) : (
            <div className="llmr-cfg-echo">no provider-declared prompt</div>
          )}
          <FieldError message={errorAt(errors, 'providers', id, 'system_prompt')} />
        </>
      ) : null}

      <Dialog open={promptOpen} onOpenChange={setPromptOpen}>
        {promptOpen ? (
          <DialogContent data-iii-ui="llm-router" className="llmr-cfg-prompt-dialog">
            <DialogTitle>system prompt · {label}</DialogTitle>
            <DialogDescription>
              {overridden
                ? 'operator override — replaces the provider-declared identity prompt'
                : 'starts from the provider-declared prompt; apply to store as an override'}
            </DialogDescription>
            <textarea
              className="llmr-cfg-input llmr-cfg-prompt-editor"
              value={promptDraft}
              rows={16}
              autoFocus
              spellCheck={false}
              placeholder="identity prompt for this provider"
              onChange={(e) => setPromptDraft(e.target.value)}
            />
            <div className="llmr-cfg-dialog-actions">
              {overridden ? (
                <Button type="button" variant="ghost" size="sm" onClick={clearPrompt}>
                  use provider default
                </Button>
              ) : null}
              <span className="llmr-cfg-dialog-actions-spacer" />
              <Button type="button" variant="ghost" size="sm" onClick={() => setPromptOpen(false)}>
                cancel
              </Button>
              <Button type="button" variant="primary" size="sm" onClick={applyPrompt}>
                apply override
              </Button>
            </div>
          </DialogContent>
        ) : null}
      </Dialog>
    </section>
  )
}
