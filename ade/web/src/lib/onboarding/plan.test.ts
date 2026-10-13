import { describe, expect, it } from 'vitest'
import { envFileName } from '@/lib/secrets'
import { JUDGE_OPTIONS, workerSource } from './catalog'
import { shouldAutoOpenOnboarding } from './open'
import {
  activeJudge,
  connectPlan,
  describeStep,
  judgePlan,
  type KeyDetection,
  type ProviderChoice,
  providerChoices,
  registryChoices,
  servesUsableModels,
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
      'Found your Anthropic key in your shell profile.',
    )
  })

  it('needs only the sign-in, not the CLI program (the desktop apps)', () => {
    const choices = providerChoices({
      tools: [
        { ...signedIn('codex', 'provider-openai-codex'), installed: false },
      ],
      providers: [],
      detections: [],
    })
    const codex = byId(choices, 'openai-codex')
    expect(codex.recommended).toBe(true)
    expect(codex.reason).toMatch(/signed in on this machine/)
    expect(codex.reason).not.toMatch(/not found/)
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

  it('lists other running providers that serve models as connected', () => {
    const choices = providerChoices({
      tools: [],
      providers: [
        // Device flow: the router holds no credential, the worker does.
        {
          id: 'github-copilot',
          title: 'GitHub Copilot',
          configured: false,
          ownsAuthentication: true,
          available: true,
          modelCount: 10,
        },
        // Keyless local server.
        {
          id: 'llamacpp',
          title: 'llama.cpp',
          configured: true,
          available: true,
          modelCount: 2,
        },
        // A key provider the wizard has no recipe for, with no key: its
        // catalog is not usable, so it is not connected.
        {
          id: 'sarvam',
          title: 'Sarvam',
          configured: false,
          available: true,
          modelCount: 3,
        },
      ],
      detections: [],
    })
    const copilot = byId(choices, 'github-copilot')
    expect(copilot).toMatchObject({
      ready: true,
      installed: true,
      worker: 'provider-github-copilot',
      modelCount: 10,
    })
    expect(byId(choices, 'llamacpp').ready).toBe(true)
    expect(choices.some((choice) => choice.providerId === 'sarvam')).toBe(false)
  })
})

describe('device sign-in choices', () => {
  it('offers GitHub Copilot with a browser sign-in until it serves models', () => {
    const fresh = byId(
      providerChoices({ tools: [], providers: [], detections: [] }),
      'github-copilot',
    )
    expect(fresh).toMatchObject({
      kind: 'device',
      worker: 'provider-github-copilot',
      ready: false,
      installed: false,
    })
    expect(fresh.reason).toMatch(/GitHub/)

    const signedIn = byId(
      providerChoices({
        tools: [],
        providers: [
          {
            id: 'github-copilot',
            title: 'GitHub Copilot',
            configured: false,
            ownsAuthentication: true,
            available: true,
            modelCount: 10,
          },
        ],
        detections: [],
      }),
      'github-copilot',
    )
    expect(signedIn).toMatchObject({ kind: 'device', ready: true })
  })
})

describe('servesUsableModels', () => {
  it('needs models and either a credential or its own authentication', () => {
    const state = {
      id: 'x',
      title: 'X',
      available: true,
      modelCount: 4,
    }
    expect(servesUsableModels({ ...state, configured: true })).toBe(true)
    expect(
      servesUsableModels({
        ...state,
        configured: false,
        ownsAuthentication: true,
      }),
    ).toBe(true)
    expect(servesUsableModels({ ...state, configured: false })).toBe(false)
    expect(
      servesUsableModels({ ...state, configured: true, modelCount: 0 }),
    ).toBe(false)
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
      owner: 'Anthropic',
    })
    const store = plan[1]
    if (store.kind !== 'store-secret') throw new Error('expected store-secret')
    expect(store.consumers).toEqual(['llm-router'])
    // Short sentences for someone exploring iii; only workers are named.
    expect(plan.map(describeStep)).toEqual([
      'Add 2 workers: secrets and provider-claude-code',
      'Store your Anthropic key encrypted on this machine',
      'Connect Anthropic with that key',
      'Check that Claude Code models are ready',
      'Check that Anthropic models are ready',
    ])
    for (const step of plan) {
      expect(describeStep(step)).not.toMatch(
        /secret:\/\/|env:\/\/|::|llm-router|ANTHROPIC_API_KEY/,
      )
    }
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
      expect(describeStep(step)).not.toContain('very-secret')
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
    expect(describeStep(plan[0])).toBe(
      'Use your Anthropic key already saved on this machine',
    )
  })

  it('keeps a key in .env as an environment variable when the user prefers it', () => {
    const shared = connectPlan(
      [{ choice: byId(choices, 'anthropic'), key: { mode: 'env' } }],
      new Set(['secrets', 'provider-anthropic']),
    )
    expect(describeStep(shared[0])).toBe(
      'Use your Anthropic key from this project’s .env',
    )
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
    const title = describeStep(pasted[0])
    expect(title).toBe('Save your Anthropic key in this project’s .env')
    expect(title).not.toContain('very-secret')
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
    expect(describeStep(staging[0])).toBe(
      'Save your Anthropic key in this project’s .env.staging',
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
          name: 'provider-sarvam',
          description: 'Sarvam provider worker; needs SARVAM_API_KEY.',
          version: '0.1.11',
        },
      ],
      new Set(),
      choices,
    )
    expect(extra.title).toBe('Sarvam')
    expect(extra.reason).toBe('Set it up after it is added.')
    expect(
      connectPlan([{ choice: extra }], new Set()).map((s) => s.kind),
    ).toEqual(['add-workers'])
  })
})

describe('activeJudge', () => {
  const both = new Set(['judge', 'judge-typesafe', 'judge-openai'])

  it('is the option the hub answers with, not the first installed one', () => {
    expect(activeJudge(both, 'openai')?.id).toBe('openai')
    expect(activeJudge(both, 'typesafe')?.id).toBe('typesafe')
  })

  it('falls back to the first installed option without a usable provider', () => {
    expect(activeJudge(both, null)?.id).toBe('typesafe')
    // The hub names a judge whose worker is gone.
    expect(activeJudge(both, 'clef')?.id).toBe('typesafe')
    expect(activeJudge(new Set(['judge']), 'openai')).toBeUndefined()
  })
})

describe('judgePlan', () => {
  it('lists the judges hosted first, then the local ones', () => {
    expect(JUDGE_OPTIONS.map((option) => option.id)).toEqual([
      'typesafe',
      'openai',
      'clef',
      'laya',
      'decider',
    ])
  })

  it('sets up OpenAI with the key the OpenAI provider may already share', () => {
    const openai = JUDGE_OPTIONS.find((option) => option.id === 'openai')
    if (!openai) throw new Error('no openai')
    const plan = judgePlan(
      openai,
      { mode: 'paste', value: 'sk-test-123456' },
      new Set(['secrets', 'judge']),
    )
    expect(plan[1]).toMatchObject({
      kind: 'store-secret',
      name: 'OPENAI_API_KEY',
      consumers: ['judge-openai'],
    })
    expect(plan[2]).toMatchObject({
      configuration: 'judge-openai',
      path: ['api_key'],
      value: 'secret://OPENAI_API_KEY',
    })
    expect(plan[3]).toMatchObject({ path: ['provider'], value: 'openai' })
    expect(plan.map(describeStep)).toEqual([
      'Add the judge-openai worker',
      'Store your OpenAI key encrypted on this machine',
      'Connect Decisions by OpenAI with that key',
      'Have Judge answer with Decisions by OpenAI',
      'Check that Decisions by OpenAI answers',
    ])
  })

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
    expect(plan.map(describeStep)).toEqual([
      'Add 2 workers: judge and judge-typesafe',
      'Store your TypeSafe key encrypted on this machine',
      'Connect Jev by TypeSafe with that key',
      'Have Judge answer with Jev by TypeSafe',
      'Check that Jev by TypeSafe answers',
    ])
  })

  it.each([
    ['laya', 'judge-laya'],
    ['decider', 'judge-decider'],
    ['clef', 'judge-clef'],
  ])('needs no key for the local judge %s', (id, worker) => {
    const local = JUDGE_OPTIONS.find((option) => option.id === id)
    if (!local) throw new Error(`no ${id}`)
    expect(local.envVar).toBeUndefined()
    const plan = judgePlan(local, undefined, new Set(['judge']))
    expect(plan.map((step) => step.kind)).toEqual([
      'add-workers',
      'set-config',
      'check-judge',
    ])
    const add = plan[0]
    if (add.kind !== 'add-workers') throw new Error('expected add-workers')
    expect(add.workers).toEqual([worker])
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
  it('opens on first run, whatever the router serves', () => {
    // A signed-in Codex or a local llama.cpp server fills the catalog before
    // setup; the person still sees the wizard once.
    expect(shouldAutoOpenOnboarding({ status: 'new' }, false, null)).toBe(true)
    expect(shouldAutoOpenOnboarding({ status: 'new' }, false, 5)).toBe(true)
  })

  it('opens again after setup only when no model is connected', () => {
    for (const status of ['completed', 'dismissed', null]) {
      expect(shouldAutoOpenOnboarding({ status }, false, 0)).toBe(true)
      expect(shouldAutoOpenOnboarding({ status }, false, 3)).toBe(false)
      // A router that cannot answer opens nothing.
      expect(shouldAutoOpenOnboarding({ status }, false, null)).toBe(false)
    }
  })

  it('never opens in a browser under automation', () => {
    // An e2e suite, an agent's browser session or a stories render.
    expect(shouldAutoOpenOnboarding({ status: 'new' }, true, 0)).toBe(false)
  })

  it('stays closed where the ADE turned auto-open off, as a deploy does', () => {
    expect(
      shouldAutoOpenOnboarding({ status: 'new', auto_open: false }, false, 0),
    ).toBe(false)
    expect(
      shouldAutoOpenOnboarding({ status: 'new', auto_open: true }, false, 0),
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
