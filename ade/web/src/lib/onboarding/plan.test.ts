import { describe, expect, it } from 'vitest'
import { envFileName } from '@/lib/secrets'
import { JUDGE_OPTIONS, workerSource } from './catalog'
import { shouldAutoOpenOnboarding } from './open'
import {
  connectPlan,
  describeStep,
  judgePlan,
  type KeyDetection,
  type ProviderChoice,
  providerChoices,
  registryChoices,
  setPath,
  sourceLabel,
  type ToolScan,
} from './plan'

const signedIn = (id: string, worker: string): ToolScan => ({
  id,
  name: id,
  installed: true,
  signed_in: true,
  provider_worker: worker,
})

const shellKey = (name: string): KeyDetection => ({
  name,
  stored: false,
  sources: [
    {
      kind: 'login_shell',
      location: 'login shell (zsh)',
      hint: 'sk-ant…9f2c',
      matches_stored: false,
    },
    {
      kind: 'dotenv',
      location: '.env',
      hint: 'sk-ant…0000',
      matches_stored: false,
    },
  ],
})

function byId(choices: ProviderChoice[], id: string): ProviderChoice {
  const choice = choices.find((entry) => entry.providerId === id)
  if (!choice) throw new Error(`no ${id}`)
  return choice
}

describe('providerChoices', () => {
  it('recommends signed-in coding agents and found keys, ahead of the rest', () => {
    const choices = providerChoices({
      tools: [
        signedIn('claude-code', 'provider-claude-code'),
        { ...signedIn('codex', 'provider-openai-codex'), signed_in: false },
      ],
      providers: [],
      detections: [shellKey('ANTHROPIC_API_KEY')],
    })
    expect(choices.slice(0, 2).map((choice) => choice.providerId)).toEqual([
      'claude-code',
      'anthropic',
    ])
    const codex = byId(choices, 'openai-codex')
    expect(codex.recommended).toBe(false)
    expect(codex.reason).toMatch(/not signed in/)
    expect(byId(choices, 'anthropic').reason).toBe(
      'Found ANTHROPIC_API_KEY in your shell profile.',
    )
  })

  it('puts providers that already serve models first and marks them ready', () => {
    const choices = providerChoices({
      tools: [],
      providers: [
        {
          id: 'openai',
          title: 'OpenAI',
          configured: true,
          available: true,
          modelCount: 7,
        },
      ],
      detections: [],
    })
    expect(choices[0].providerId).toBe('openai')
    expect(choices[0].ready).toBe(true)
    expect(choices[0].installed).toBe(true)
  })
})

describe('connectPlan', () => {
  const choices = providerChoices({
    tools: [signedIn('claude-code', 'provider-claude-code')],
    providers: [
      {
        id: 'anthropic',
        title: 'Anthropic',
        configured: false,
        available: true,
        modelCount: 0,
      },
    ],
    detections: [shellKey('ANTHROPIC_API_KEY')],
  })

  it('adds missing workers once, the secrets worker first, then stores and references the key', () => {
    const plan = connectPlan(
      [
        { choice: byId(choices, 'claude-code') },
        {
          choice: byId(choices, 'anthropic'),
          key: { mode: 'import', source: 'login_shell' },
        },
      ],
      new Set(['llm-router', 'provider-anthropic']),
    )
    expect(plan.map((step) => step.kind)).toEqual([
      'add-workers',
      'store-secret',
      'set-config',
      'wait-models',
      'wait-models',
    ])
    const add = plan[0]
    if (add.kind !== 'add-workers') throw new Error('expected add-workers')
    // provider-anthropic is already running: only what is missing is added.
    expect(add.workers).toEqual(['secrets', 'provider-claude-code'])
    expect(plan[2]).toEqual({
      kind: 'set-config',
      configuration: 'llm-router',
      path: ['providers', 'anthropic', 'api_key'],
      value: 'secret://ANTHROPIC_API_KEY',
    })
    const store = plan[1]
    if (store.kind !== 'store-secret') throw new Error('expected store-secret')
    expect(store.consumers).toEqual(['llm-router'])
    expect(store.from).toBe('your shell profile')
  })

  it('never puts a pasted key in a description', () => {
    const plan = connectPlan(
      [
        {
          choice: byId(choices, 'anthropic'),
          key: { mode: 'paste', value: 'sk-ant-very-secret-value' },
        },
      ],
      new Set(['secrets']),
    )
    for (const step of plan) {
      const { title, detail } = describeStep(step)
      expect(`${title} ${detail}`).not.toContain('very-secret')
    }
  })

  it('reuses a stored key, making sure the router may read it', () => {
    const plan = connectPlan(
      [{ choice: byId(choices, 'anthropic'), key: { mode: 'stored' } }],
      new Set(['secrets']),
    )
    expect(plan.map((step) => step.kind)).toEqual([
      'store-secret',
      'set-config',
      'wait-models',
    ])
    expect(describeStep(plan[0]).title).toBe(
      'Let llm-router read ANTHROPIC_API_KEY',
    )
  })

  it('keeps a key in .env as an environment variable when the user prefers it', () => {
    const shared = connectPlan(
      [{ choice: byId(choices, 'anthropic'), key: { mode: 'env' } }],
      new Set(['secrets', 'provider-anthropic']),
    )
    expect(describeStep(shared[0])).toEqual({
      title: 'Let llm-router read ANTHROPIC_API_KEY from this project’s .env',
      detail: 'secrets::access ANTHROPIC_API_KEY → env://ANTHROPIC_API_KEY',
    })
    expect(shared[1]).toMatchObject({
      kind: 'set-config',
      path: ['providers', 'anthropic', 'api_key'],
      value: 'env://ANTHROPIC_API_KEY',
    })
    const pasted = connectPlan(
      [
        {
          choice: byId(choices, 'anthropic'),
          key: {
            mode: 'paste',
            value: 'sk-ant-very-secret-value',
            store: 'env',
          },
        },
      ],
      new Set(['secrets', 'provider-anthropic']),
    )
    const { title, detail } = describeStep(pasted[0])
    expect(title).toBe(
      'Write ANTHROPIC_API_KEY to this project’s .env, from the key you pasted',
    )
    expect(detail).toBe(
      'secrets::set ANTHROPIC_API_KEY store=env → env://ANTHROPIC_API_KEY',
    )
    expect(`${title} ${detail}`).not.toContain('very-secret')
    expect(pasted[1]).toMatchObject({ value: 'env://ANTHROPIC_API_KEY' })
    // A namespace whose secrets worker uses another env file says so.
    const staging = connectPlan(
      [
        {
          choice: byId(choices, 'anthropic'),
          key: {
            mode: 'paste',
            value: 'sk-ant-very-secret-value',
            store: 'env',
          },
        },
      ],
      new Set(['secrets', 'provider-anthropic']),
      '.env.staging',
    )
    expect(describeStep(staging[0]).title).toBe(
      'Write ANTHROPIC_API_KEY to this project’s .env.staging, from the key you pasted',
    )
  })

  it('does nothing for providers that are already connected', () => {
    const ready = providerChoices({
      tools: [],
      providers: [
        {
          id: 'openai',
          title: 'OpenAI',
          configured: true,
          available: true,
          modelCount: 3,
        },
      ],
      detections: [],
    })
    expect(connectPlan([{ choice: byId(ready, 'openai') }], new Set())).toEqual(
      [],
    )
  })

  it('adds a registry provider without waiting for models it may not have yet', () => {
    const [extra] = registryChoices(
      [
        {
          name: 'provider-github-copilot',
          description:
            'GitHub Copilot subscription provider worker; sign in once.',
          version: '0.1.11',
        },
      ],
      new Set(),
      choices,
    )
    expect(extra.title).toBe('Github Copilot')
    expect(extra.reason).toBe('GitHub Copilot subscription provider worker')
    expect(
      connectPlan([{ choice: extra }], new Set()).map((s) => s.kind),
    ).toEqual(['add-workers'])
  })
})

describe('judgePlan', () => {
  it('sets up the hosted judge with its key behind a reference', () => {
    const jev = JUDGE_OPTIONS[0]
    const plan = judgePlan(
      jev,
      { mode: 'paste', value: 'ts-key-123456' },
      new Set(['secrets']),
    )
    expect(plan.map((step) => step.kind)).toEqual([
      'add-workers',
      'store-secret',
      'set-config',
      'set-config',
      'check-judge',
    ])
    expect(plan[2]).toMatchObject({
      configuration: 'judge-typesafe',
      path: ['api_key'],
      value: 'secret://TYPESAFE_API_KEY',
    })
    expect(plan[3]).toMatchObject({
      configuration: 'judge',
      path: ['provider'],
      value: 'typesafe',
    })
  })

  it('needs no key for a local judge', () => {
    const laya = JUDGE_OPTIONS.find((option) => option.id === 'laya')
    if (!laya) throw new Error('no laya')
    const plan = judgePlan(laya, undefined, new Set(['judge']))
    expect(plan.map((step) => step.kind)).toEqual([
      'add-workers',
      'set-config',
      'check-judge',
    ])
    const add = plan[0]
    if (add.kind !== 'add-workers') throw new Error('expected add-workers')
    expect(add.workers).toEqual(['judge-laya'])
  })
})

describe('setPath', () => {
  it('writes a nested value without touching siblings', () => {
    const value = {
      providers: { openai: { api_url: 'x' } },
      settings: { a: 1 },
    }
    expect(setPath(value, ['providers', 'anthropic', 'api_key'], 'r')).toEqual({
      providers: { openai: { api_url: 'x' }, anthropic: { api_key: 'r' } },
      settings: { a: 1 },
    })
    expect(value.providers).toEqual({ openai: { api_url: 'x' } })
    expect(setPath(null, ['provider'], 'laya')).toEqual({ provider: 'laya' })
  })
})

describe('workerSource', () => {
  it('uses a development override when the console configuration maps one', () => {
    const config = {
      onboarding: { worker_sources: { secrets: '/src/workers/secrets' } },
    }
    expect(workerSource('secrets', config)).toBe('/src/workers/secrets')
    expect(workerSource('judge', config)).toBe('judge')
    expect(workerSource('judge', null)).toBe('judge')
  })

  it('passes a container object through for a worker without a start script', () => {
    const container = {
      worker: '/src/workers/judge-typesafe',
      scripts: { run: 'cargo run --bin judge-typesafe' },
    }
    const config = {
      onboarding: { worker_sources: { 'judge-typesafe': container } },
    }
    expect(workerSource('judge-typesafe', config)).toEqual(container)
    expect(
      workerSource('judge', { onboarding: { worker_sources: { judge: {} } } }),
    ).toBe('judge')
  })
})

describe('judgeFailure', () => {
  it('names a rejected key on a hosted judge, and only then', async () => {
    const { judgeFailure } = await import('./api')
    const rejected = {
      code: 'http',
      http_status: 401,
      status: 'error',
      provider_error: {
        detail: { message: 'Cannot authenticate with the server.' },
      },
    }
    expect(judgeFailure(rejected, 'Jev by TypeSafe', true)).toBe(
      'Jev by TypeSafe rejected this key — Cannot authenticate with the server. Paste a different key and set up again.',
    )
    expect(judgeFailure(rejected, 'Laya', false)).toBeNull()
    expect(judgeFailure({ models: [] }, 'Jev', true)).toBeNull()
    expect(judgeFailure(null, 'Jev', true)).toBeNull()
  })
})

describe('shouldAutoOpenOnboarding', () => {
  it('opens by itself only for a person on first run', () => {
    expect(shouldAutoOpenOnboarding({ status: 'new' }, false)).toBe(true)
    // An e2e suite, an agent's browser session or a stories render.
    expect(shouldAutoOpenOnboarding({ status: 'new' }, true)).toBe(false)
    expect(shouldAutoOpenOnboarding({ status: 'dismissed' }, false)).toBe(false)
    expect(shouldAutoOpenOnboarding({ status: 'completed' }, false)).toBe(false)
    expect(shouldAutoOpenOnboarding({ status: null }, false)).toBe(false)
  })

  it('stays closed where the ADE turned auto-open off, as a deploy does', () => {
    expect(
      shouldAutoOpenOnboarding({ status: 'new', auto_open: false }, false),
    ).toBe(false)
    expect(
      shouldAutoOpenOnboarding({ status: 'new', auto_open: true }, false),
    ).toBe(true)
  })
})

describe('envFileName', () => {
  it('names the configured env file, .env by default', () => {
    expect(envFileName('/home/me/project/.env.staging')).toBe('.env.staging')
    expect(envFileName('C:\\project\\.env.prod')).toBe('.env.prod')
    expect(envFileName(undefined)).toBe('.env')
    expect(
      sourceLabel({
        kind: 'dotenv',
        location: '/p/.env.staging',
        hint: 'x',
        matches_stored: false,
      }),
    ).toBe("this project's .env.staging")
  })
})
