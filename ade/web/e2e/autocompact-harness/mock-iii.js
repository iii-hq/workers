/*
 * The auto-compact harness's engine: in memory, deterministic, no socket.
 * It answers the reads and writes Settings › Chat › Auto-compact and the
 * project dialog make, the way phase 3's harness does (contract C/D), and
 * records every call on `window.__AUTOCOMPACT_HARNESS__.calls`.
 */

const parameters = new URLSearchParams(location.search)

export const PROJECTS = [
  {
    id: 'prj_web',
    name: 'acme-web',
    color: 'blue',
    folders: ['/home/demo/acme/web'],
    autocompact: { mode: 'tokens', tokens: 160_000 },
    created_at_ms: 1,
    updated_at_ms: 1,
  },
  {
    id: 'prj_billing',
    name: 'billing-api',
    color: 'green',
    folders: ['/home/demo/acme/billing'],
    created_at_ms: 1,
    updated_at_ms: 1,
  },
  {
    id: 'prj_mobile',
    name: 'mobile',
    color: 'rose',
    folders: ['/home/demo/acme/mobile'],
    autocompact: { mode: 'off' },
    created_at_ms: 1,
    updated_at_ms: 1,
  },
]

const MODELS = [
  {
    provider: 'anthropic',
    model: 'claude-sonnet-5-5',
    name: 'Claude Sonnet 5.5 (Claude Code)',
    context_window: 1_000_000,
    usable: 916_000,
    last_used_ms: Date.now() - 60_000,
  },
  {
    provider: 'anthropic',
    model: 'claude-opus-5-5',
    name: 'Claude Opus 5.5 (Claude Code)',
    context_window: 200_000,
    usable: 116_000,
    last_used_ms: Date.now() - 120_000,
  },
]

const LOG = [
  {
    session_id: 's-ledger',
    title: 'Ledger reconciliation',
    at_ms: Date.now() - 5 * 60_000,
    trigger: 'auto',
    threshold_tokens: 600_000,
    context_before: 601_000,
    context_after: 40_000,
    rule: { scope: 'session' },
  },
  {
    session_id: 's-webhooks',
    title: 'Webhook retries',
    at_ms: Date.now() - 2 * 3_600_000,
    trigger: 'inline',
    threshold_tokens: 160_000,
    context_before: 183_000,
    context_after: 31_000,
    rule: { scope: 'project', project_id: 'prj_web', project_name: 'acme-web' },
  },
]

const state = {
  settings: {
    descriptions: { agent_writes: true },
    autocompact: {
      mode: 'tokens',
      tokens: 400_000,
      instructions: 'Keep file paths and the PR numbers.',
    },
  },
  projects: structuredClone(PROJECTS),
  revision: 1,
}

export const harness = {
  calls: [],
  /** Conversations opened from the history. */
  opened: [],
  /** Details saved from an overrides row. */
  saved: [],
  /** The next chat-settings write fails with this typed rejection. */
  rejectNext: null,
  models: parameters.has('noModels') ? [] : MODELS,
}
window.__AUTOCOMPACT_HARNESS__ = harness

function rejection(code, message) {
  const error = new Error(`handler error: ${JSON.stringify({ code, message })}`)
  error.code = 'invocation_failed'
  return error
}

async function call(functionId, input = {}) {
  harness.calls.push({ functionId, input: structuredClone(input) })
  switch (functionId) {
    case 'harness::chat-settings::get':
      return structuredClone(state.settings)
    case 'harness::chat-settings::set': {
      if (harness.rejectNext) {
        const { code, message } = harness.rejectNext
        harness.rejectNext = null
        throw rejection(code, message)
      }
      state.settings = { ...state.settings, ...input.patch }
      return { settings: structuredClone(state.settings) }
    }
    case 'harness::autocompact::models':
      return structuredClone(harness.models)
    case 'harness::autocompact::log':
      return structuredClone(LOG.slice(0, input.limit ?? 20))
    case 'harness::projects::list':
      return {
        projects: structuredClone(state.projects),
        revision: state.revision,
      }
    case 'harness::projects::update': {
      const project = state.projects.find((p) => p.id === input.id)
      if (!project) throw rejection('not_found', 'No such project')
      const { id: _id, bring_existing: _bring, ...patch } = input
      Object.assign(project, patch)
      state.revision += 1
      return { project: structuredClone(project), moved: 0 }
    }
    default:
      return null
  }
}

const iii = {
  browserId: 'autocompact-harness',
  call,
  on: () => () => {},
  observe: () => () => {},
}

export const getIiiClient = async () => iii
