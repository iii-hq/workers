import { appendFileSync, writeFileSync } from 'node:fs'
import { registerWorker } from 'iii-browser-sdk'

const root = process.env.NAMESPACED_PROVIDER_FIXTURE_ROOT
if (!root) throw new Error('NAMESPACED_PROVIDER_FIXTURE_ROOT is required')
const engineUrl = process.env.III_ENGINE_URL ?? 'ws://127.0.0.1:49134'
const providerId = process.env.NAMESPACED_PROVIDER_PROVIDER_ID ?? 'openai-codex'
const callsPath = `${root}/provider-calls.jsonl`
const readyPath = `${root}/fixture-ready.json`
const sdk = registerWorker(engineUrl, {
  metadata: { name: 'namespaced-provider-fixture' },
})

const log = (entry) => {
  appendFileSync(callsPath, `${JSON.stringify(entry)}\n`, 'utf8')
}

const model = {
  id: 'codex/provider-fixture',
  provider: providerId,
  display_name: 'Provider fixture model',
  context_window: 128000,
  max_output_tokens: 4096,
  supports_tools: true,
  supports_thinking: true,
  supports_vision: false,
}

sdk.registerFunction(
  `provider::${providerId}::stream`,
  async (input) => {
    log({ type: 'stream', provider: providerId, model: input.model })
    const writer = input.writer_ref
    writer.sendMessage(
      JSON.stringify({
        type: 'done',
        message: {
          role: 'assistant',
          provider: providerId,
          model: input.model,
          timestamp: Date.now(),
          stop_reason: 'end',
          content: [{ type: 'text', text: 'Provider fixture response' }],
          usage: { input: 1, output: 1 },
        },
      }),
    )
    writer.close()
    return { ok: true }
  },
  {
    description:
      'Deterministic local provider fixture; never calls an upstream API.',
    request_schema: { type: 'object' },
  },
)
sdk.registerFunction(
  `provider::${providerId}::abort`,
  async () => ({ aborted: false }),
  {
    description: 'Deterministic local abort.',
    request_schema: { type: 'object' },
  },
)

const registrationPayload = {
  function_id: 'router::provider::register',
  payload: {
    id: providerId,
    display_name: 'OpenAI Codex (provider fixture)',
    config_schema: {
      type: 'object',
      properties: {
        api_url: { type: 'string' },
        max_tokens: { type: 'integer', minimum: 1, writeOnly: true },
      },
    },
    defaults: { max_tokens: 42 },
    models: [model],
    supports_model_listing: false,
  },
}

let registration
for (let attempt = 0; attempt < 20; attempt += 1) {
  try {
    registration = await sdk.trigger(registrationPayload)
    break
  } catch (error) {
    if (
      (error?.code !== 'function_not_found' &&
        !String(error?.message ?? error).includes('function_not_found')) ||
      attempt === 19
    ) {
      throw error
    }
    await new Promise((resolve) => setTimeout(resolve, 250))
  }
}

log({ type: 'registered', provider: providerId, result: registration })
writeFileSync(readyPath, JSON.stringify({ providerId, model }, null, 2), 'utf8')
console.log(JSON.stringify({ fixture_ready: readyPath, registration }))

const shutdown = async () => {
  await sdk.shutdown()
  process.exit(0)
}
process.once('SIGTERM', shutdown)
process.once('SIGINT', shutdown)
await new Promise(() => {})
