import type { FunctionCallMessage } from '@/types/chat'
import { wrapHarness } from './sandbox-fixtures'

const now = Date.now()

function base(
  id: string,
  functionId: string,
  input: unknown,
  output?: unknown,
  extra?: Partial<FunctionCallMessage>,
): FunctionCallMessage {
  return {
    id,
    role: 'function-call',
    functionId,
    input,
    output,
    durationMs: 214,
    createdAt: now,
    ...extra,
  }
}

/* ---------------- a realistic multi-provider catalog ---------------- */

const catalog = {
  models: [
    {
      id: 'claude-fable-5',
      provider: 'anthropic',
      display_name: 'Claude Fable 5',
      context_window: 1_000_000,
      max_output_tokens: 128_000,
      supports_thinking: true,
      supports_xhigh: true,
      supports_tools: true,
      supports_vision: true,
      supports_cache: true,
      supports_structured_output: false,
      pricing: { input: 10, output: 50, cache_read: 1, cache_write: 12.5 },
    },
    {
      id: 'claude-haiku-4-6',
      provider: 'anthropic',
      display_name: 'Claude Haiku 4.6',
      context_window: 200_000,
      max_output_tokens: 64_000,
      supports_tools: true,
      supports_vision: true,
      supports_cache: true,
      supports_structured_output: true,
      pricing: { input: 1, output: 5, cache_read: 0.1, cache_write: 1.25 },
    },
    {
      id: 'gpt-5.2',
      provider: 'openai',
      display_name: 'GPT-5.2',
      context_window: 400_000,
      max_output_tokens: 128_000,
      supports_thinking: true,
      supports_tools: true,
      supports_vision: true,
      supports_structured_output: true,
      pricing: { input: 5, output: 20 },
    },
  ],
}

/* ---------------- router::models::list ---------------- */

export const routerModelsList = base(
  'router-models-ok',
  'router::models::list',
  {},
  wrapHarness(catalog),
)

export const routerModelsListFiltered = base(
  'router-models-filtered',
  'router::models::list',
  { provider: 'anthropic', capability: 'vision' },
  wrapHarness({
    models: catalog.models.filter((m) => m.provider === 'anthropic'),
  }),
)

export const routerModelsListRunning = base(
  'router-models-running',
  'router::models::list',
  {},
  undefined,
  { running: true },
)

export const routerModelsListEmpty = base(
  'router-models-empty',
  'router::models::list',
  { provider: 'cohere' },
  wrapHarness({ models: [] }),
)

export const routerFixtures = [
  routerModelsList,
  routerModelsListFiltered,
  routerModelsListRunning,
  routerModelsListEmpty,
] as const
