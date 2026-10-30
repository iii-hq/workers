import { SlidersHorizontal } from 'lucide-react'
import { useId } from 'react'
import { SecretKeyField } from '@/components/secrets/SecretKeyField'
import { Input } from '@/components/ui/Input'
import { KEY_PROVIDERS } from '@/lib/onboarding/catalog'
import { cn } from '@/lib/utils'
import type {
  JsonSchema,
  JsonValue,
} from '@/pages/Configuration/tabs/WorkersTab/api'
import { ProviderSignIn } from './ProviderSignIn'

// viewport: phone chrome — the sm and md utilities here are the console's
// phone-vs-desktop presentation (touch sizes, 16px text, sheet vs popover),
// not pane layout; see viewport-breakpoint-conformance.test.ts.

type JsonObject = { [key: string]: JsonValue }

interface ProviderSettingsFormProps {
  providerId: string
  schema: JsonSchema
  value: JsonValue
  onChange: (next: JsonValue) => void
  errors?: ReadonlyMap<string, string>
  credentialEnvVar?: string
  configured?: boolean
  /** What the router reports about the credential it resolved. */
  credentialStatus?: {
    source?: string
    error?: string
    checking?: boolean
    detail?: string
  }
  /**
   * Apply a key change at once (`secret://NAME` or `env://NAME`, or
   * `undefined` to clear it)
   * instead of leaving it in the draft for Save — what the model picker does,
   * so storing a key connects the provider in one step.
   */
  onCredentialChange?: (next: string | undefined) => Promise<unknown>
  className?: string
}

/** Provider keys are read by the router, nobody else. */
const ROUTER_CONSUMERS = ['llm-router'] as const

function asObject(value: JsonValue): JsonObject {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? value
    : {}
}

function schemaProperties(schema: JsonSchema): Record<string, JsonSchema> {
  const properties = schema.properties
  return properties &&
    typeof properties === 'object' &&
    !Array.isArray(properties)
    ? (properties as Record<string, JsonSchema>)
    : {}
}

function stringValue(value: JsonValue | undefined): string {
  return typeof value === 'string' ? value : ''
}

function fieldError(
  errors: ReadonlyMap<string, string> | undefined,
  field: string,
): string | undefined {
  return errors?.get(`/${field}`) ?? errors?.get(field)
}

function FieldMessage({ message }: { message?: string }) {
  return message ? (
    <p className="text-[13px] leading-relaxed text-alert-strong" role="alert">
      {message}
    </p>
  ) : null
}

/** The schema's own words for a field, when it has any. */
function fieldDescription(schema: JsonSchema | undefined): string | undefined {
  return typeof schema?.description === 'string' && schema.description.trim()
    ? schema.description.trim()
    : undefined
}

/** A field row: label, the schema's description, the control, its error. */
function Field({
  id,
  label,
  description,
  error,
  children,
}: {
  id: string
  label: string
  description?: string
  error?: string
  children: React.ReactNode
}) {
  return (
    <div className="space-y-2.5 px-5 py-4">
      <div className="space-y-0.5">
        <label htmlFor={id} className="block text-[15px] font-medium text-ink">
          {label}
        </label>
        {description ? (
          <p className="text-[13px] leading-relaxed text-ink-faint">
            {description}
          </p>
        ) : null}
      </div>
      {children}
      <FieldMessage message={error} />
    </div>
  )
}

/** The control itself: a field that reads as one on the card. */
const fieldInputClass =
  'h-10 rounded-lg bg-panel px-3.5 text-[15px] text-ink ring-1 ring-inset ring-edge placeholder:text-ink-ghost hover:bg-panel focus:border-transparent focus:ring-rule-focus sm:h-10 sm:text-[15px]'

/**
 * Friendly fallback for the router's common provider settings. An API-key
 * provider's key goes through the secrets worker like everywhere else in the
 * console (`SecretKeyField`): encrypted, or kept in this project's `.env`
 * when the user prefers an environment variable, so the router slice only
 * ever holds `secret://NAME` or `env://NAME`; a provider that signs in on its
 * own shows that sign-in.
 */
export function ProviderSettingsForm({
  providerId,
  schema,
  value,
  onChange,
  errors,
  credentialEnvVar,
  configured,
  credentialStatus,
  onCredentialChange,
  className,
}: ProviderSettingsFormProps) {
  const apiUrlId = useId()
  const maxTokensId = useId()
  const current = asObject(value)
  const properties = schemaProperties(schema)
  const systemPromptSchema = properties.system_prompt
  const hasSystemPrompt = typeof current.system_prompt === 'string'
  const systemPromptDefault =
    typeof systemPromptSchema?.default === 'string'
      ? systemPromptSchema.default
      : ''

  function patch(field: string, next: JsonValue | undefined) {
    const updated = { ...current }
    if (next === undefined) delete updated[field]
    else updated[field] = next
    onChange(updated)
  }

  return (
    <div
      className={cn(
        'space-y-6 py-1 font-inter text-[15px] text-ink',
        className,
      )}
    >
      {credentialEnvVar ? (
        <SecretKeyField
          name={credentialEnvVar}
          value={stringValue(current.api_key) || undefined}
          onChange={(next) =>
            onCredentialChange
              ? onCredentialChange(next)
              : patch('api_key', next)
          }
          consumers={ROUTER_CONSUMERS}
          environment
          status={{ ...credentialStatus, connected: configured }}
          keysUrl={
            KEY_PROVIDERS.find((entry) => entry.providerId === providerId)
              ?.keysUrl
          }
        />
      ) : (
        <ProviderSignIn providerId={providerId} />
      )}

      <section
        aria-labelledby="provider-settings-heading"
        className="space-y-3"
      >
        <div className="flex items-center gap-2 px-1">
          <SlidersHorizontal
            className="size-4 shrink-0 text-ink-faint"
            aria-hidden
          />
          <h3
            id="provider-settings-heading"
            className="text-[13px] font-medium text-ink-faint"
          >
            Provider settings
          </h3>
        </div>

        <div className="divide-y divide-edge overflow-hidden rounded-xl bg-surface ring-1 ring-inset ring-edge">
          {properties.api_url ? (
            <Field
              id={apiUrlId}
              label="API endpoint"
              description={
                fieldDescription(properties.api_url) ??
                'Where requests for this provider are sent.'
              }
              error={fieldError(errors, 'api_url')}
            >
              <Input
                id={apiUrlId}
                value={stringValue(current.api_url)}
                onChange={(next) => patch('api_url', next || undefined)}
                placeholder={
                  typeof properties.api_url.default === 'string'
                    ? properties.api_url.default
                    : 'Use provider default'
                }
                preserveCase
                inputMode="url"
                className={fieldInputClass}
              />
            </Field>
          ) : null}

          {properties.max_tokens ? (
            <Field
              id={maxTokensId}
              label="Maximum output tokens"
              description={
                fieldDescription(properties.max_tokens) ??
                'The most tokens one reply may contain.'
              }
              error={fieldError(errors, 'max_tokens')}
            >
              <Input
                id={maxTokensId}
                value={
                  typeof current.max_tokens === 'number'
                    ? String(current.max_tokens)
                    : ''
                }
                onChange={(next) => {
                  const parsed = Number(next)
                  patch(
                    'max_tokens',
                    next === '' || Number.isNaN(parsed) ? undefined : parsed,
                  )
                }}
                placeholder={
                  typeof properties.max_tokens.default === 'number'
                    ? String(properties.max_tokens.default)
                    : 'Use provider default'
                }
                preserveCase
                inputMode="numeric"
                className={cn(fieldInputClass, 'max-w-[12rem] tabular-nums')}
              />
            </Field>
          ) : null}

          {systemPromptSchema ? (
            <div className="space-y-3 px-5 py-4">
              <label className="flex min-h-10 cursor-pointer items-center justify-between gap-4">
                <span className="min-w-0 flex-1 space-y-0.5">
                  <span className="block text-[15px] font-medium text-ink">
                    Custom system prompt
                  </span>
                  <span className="block text-[13px] leading-relaxed text-ink-faint">
                    Leave off to use the prompt supplied by the provider.
                  </span>
                </span>
                <input
                  type="checkbox"
                  checked={hasSystemPrompt}
                  onChange={(event) =>
                    patch(
                      'system_prompt',
                      event.currentTarget.checked
                        ? systemPromptDefault
                        : undefined,
                    )
                  }
                  className="size-4 shrink-0 accent-accent"
                />
              </label>
              {hasSystemPrompt ? (
                <textarea
                  value={stringValue(current.system_prompt)}
                  onChange={(event) =>
                    patch('system_prompt', event.currentTarget.value)
                  }
                  rows={6}
                  placeholder="Enter a provider-specific system prompt"
                  className="w-full resize-y rounded-lg bg-panel px-3.5 py-2.5 text-[15px] leading-relaxed text-ink ring-1 ring-inset ring-edge placeholder:text-ink-ghost focus:outline-none focus:ring-rule-focus"
                />
              ) : null}
              <FieldMessage message={fieldError(errors, 'system_prompt')} />
            </div>
          ) : null}
        </div>
      </section>

      <FieldMessage message={errors?.get('')} />
    </div>
  )
}
