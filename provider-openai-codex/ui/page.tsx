import {
  Button,
  type Host,
  type JsonValue,
  type ProviderConfigFormProps,
} from '@iii-dev/console-ui'
import { useState } from 'react'

type JsonObject = { [key: string]: JsonValue }

function asObject(value: JsonValue): JsonObject {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? value
    : {}
}

function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : 'Could not connect to Codex.'
}

function CodexProviderForm({
  host,
  ...props
}: ProviderConfigFormProps & { host: Host }) {
  const [checking, setChecking] = useState(false)
  const [status, setStatus] = useState<string | null>(null)
  const value = asObject(props.value)
  const promptEnabled = typeof value.system_prompt === 'string'

  function patch(field: string, next: JsonValue | undefined) {
    const updated = { ...value }
    if (next === undefined) delete updated[field]
    else updated[field] = next
    props.onChange(updated)
  }

  async function checkConnection() {
    setChecking(true)
    setStatus(null)
    try {
      const response = await host.iii.trigger<{ count?: number }>(
        'provider::openai-codex::refresh_models',
        {},
      )
      const count = response?.count ?? 0
      setStatus(`Connected · ${count} model${count === 1 ? '' : 's'} available`)
    } catch (error) {
      setStatus(messageOf(error))
    } finally {
      setChecking(false)
    }
  }

  return (
    <div className="codex-provider-form">
      <section className="codex-provider-card">
        <div className="codex-provider-status">
          <span
            className="codex-provider-dot"
            data-active={props.modelCount > 0}
          />
          <span>
            {props.modelCount > 0
              ? 'Codex models available'
              : 'Sign-in required'}
          </span>
        </div>
        <h3>Use your Codex app login</h3>
        <p>
          This provider uses your ChatGPT subscription. Do not enter an API key
          here; API keys belong to the OpenAI provider.
        </p>
        <ol>
          <li>Open a terminal on the machine running the provider.</li>
          <li>
            Run <code>codex login</code> and complete the ChatGPT sign-in in the
            Codex app or browser.
          </li>
          <li>Return here and check the connection.</li>
        </ol>
        <div className="codex-provider-action">
          <Button
            type="button"
            variant="primary"
            size="sm"
            disabled={checking}
            onClick={() => void checkConnection()}
          >
            {checking ? 'Checking…' : 'Check connection'}
          </Button>
          {status ? <span role="status">{status}</span> : null}
        </div>
      </section>

      <section className="codex-provider-card codex-provider-prompt">
        <label>
          <span>
            <strong>Custom system prompt</strong>
            <small>Leave off to use the Codex provider default.</small>
          </span>
          <input
            type="checkbox"
            checked={promptEnabled}
            onChange={(event) =>
              patch(
                'system_prompt',
                event.currentTarget.checked ? '' : undefined,
              )
            }
          />
        </label>
        {promptEnabled ? (
          <textarea
            rows={6}
            value={
              typeof value.system_prompt === 'string' ? value.system_prompt : ''
            }
            onChange={(event) =>
              patch('system_prompt', event.currentTarget.value)
            }
            placeholder="Enter a Codex-specific system prompt"
          />
        ) : null}
      </section>
    </div>
  )
}

export default function setup(host: Host) {
  host.providerConfigForms?.register('openai-codex', (props) => (
    <CodexProviderForm host={host} {...props} />
  ))
}
