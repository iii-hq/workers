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
    <p className="font-sans text-base text-alert sm:text-sm" role="alert">
      {message}
    </p>
  ) : null
}

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
    <div className={cn('space-y-5 py-1', className)}>
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
            className="size-5 shrink-0 text-ink-ghost sm:size-4"
            aria-hidden
          />
          <h3
            id="provider-settings-heading"
            className="font-sans text-[11px] font-medium text-ink-ghost"
          >
            Provider settings
          </h3>
        </div>

        <div className="divide-y divide-edge overflow-hidden rounded-lg bg-surface ring-1 ring-inset ring-edge">
          {properties.api_url ? (
            <div className="space-y-2 p-3">
              <label
                htmlFor={apiUrlId}
                className="block font-sans text-base font-medium text-ink sm:text-sm"
              >
                API endpoint
              </label>
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
                className="h-12 max-sm:text-base sm:h-9"
              />
              <FieldMessage message={fieldError(errors, 'api_url')} />
            </div>
          ) : null}

          {properties.max_tokens ? (
            <div className="space-y-2 p-3">
              <label
                htmlFor={maxTokensId}
                className="block font-sans text-base font-medium text-ink sm:text-sm"
              >
                Maximum output tokens
              </label>
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
                className="h-12 max-sm:text-base sm:h-9"
              />
              <FieldMessage message={fieldError(errors, 'max_tokens')} />
            </div>
          ) : null}

          {systemPromptSchema ? (
            <div className="space-y-3 p-3">
              <label className="flex min-h-12 cursor-pointer items-center justify-between gap-3">
                <span className="min-w-0 flex-1">
                  <span className="block font-sans text-base font-medium text-ink sm:text-sm">
                    Custom system prompt
                  </span>
                  <span className="block font-sans text-base leading-relaxed text-ink-faint sm:text-sm">
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
                  className="size-5 shrink-0 accent-accent sm:size-4"
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
                  className="w-full resize-y rounded-sm border border-transparent bg-panel-raised px-3 py-2 font-sans text-base leading-relaxed text-ink placeholder:text-ink-ghost hover:bg-surface-hover focus:border-rule-focus focus:outline-none focus:ring-[3px] focus:ring-accent/10 sm:text-sm"
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
