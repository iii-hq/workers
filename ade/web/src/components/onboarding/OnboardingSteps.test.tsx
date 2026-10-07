import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import type { ToolScan } from '@/lib/onboarding/plan'
import type { ExamplePrompt } from '@/lib/onboarding/prompts'
import { JudgeStep } from './JudgeStep'
import { ModelsStep } from './ModelsStep'
import { ReadyStep } from './ReadyStep'
import {
  type ActivityEntry,
  type MachineSnapshot,
  type OnboardingController,
  withWorkerPresence,
} from './use-onboarding'

// The models step reads the registry on mount; static rendering never runs
// effects, so the import only has to resolve.
vi.mock('@/lib/workers-registry', () => ({
  fetchRegistryProviders: () => Promise.resolve([]),
}))

const CLAUDE: ToolScan = {
  id: 'claude-code',
  name: 'Claude Code',
  installed: true,
  binary_path: '~/.local/bin/claude',
  version: '2.1.288 (Claude Code)',
  signed_in: true,
  credentials_path: '~/.claude/.credentials.json',
  provider_worker: 'provider-claude-code',
}

const CODEX: ToolScan = {
  id: 'codex',
  name: 'Codex',
  installed: true,
  binary_path: '~/.local/bin/codex',
  signed_in: false,
  sign_in_note: 'Codex is signed in with an API key, not a ChatGPT account',
  provider_worker: 'provider-openai-codex',
}

function controller(
  snapshot: Partial<MachineSnapshot>,
  activity: ActivityEntry[] = [],
): OnboardingController {
  const installed = snapshot.installed ?? new Set<string>()
  return {
    snapshot: {
      tools: [CLAUDE, CODEX],
      toolsError: null,
      providers: [],
      providersError: null,
      detections: null,
      envFile: '.env',
      installed,
      consoleConfig: null,
      browser: null,
      browserError: null,
      ...snapshot,
    },
    scanning: false,
    refresh: async () => undefined,
    activity,
    running: null,
    run: async () => true,
    judgeInstalled: installed.has('judge'),
    checkChromium: async () => null,
    installChromium: async () => ({ ok: true }),
    chromiumProgress: null,
  }
}

const noop = () => undefined

describe('ModelsStep', () => {
  it('reports each coding agent without its sign-in file or CLI details', () => {
    const html = renderToStaticMarkup(
      <ModelsStep
        onboarding={controller({ installed: new Set(['llm-router']) })}
        onBack={noop}
        onNext={noop}
      />,
    )
    expect(html).toContain('Recommended for this machine')
    expect(html).not.toContain('Sign-in at')
    expect(html).not.toContain('~/.claude/.credentials.json')
    expect(html).not.toContain('~/.local/bin/claude')
    expect(html).not.toContain('2.1.288')
    // Installed but not signed in: beside the recommendations, saying why.
    expect(html).toContain('Not signed in')
    expect(html).toContain('not a ChatGPT account')
  })

  it('recommends the signed-in agent, says which worker it adds, and shows the plan before anything runs', () => {
    const html = renderToStaticMarkup(
      <ModelsStep
        onboarding={controller({ installed: new Set(['llm-router']) })}
        position={{ index: 1, total: 2 }}
        onBack={noop}
        onNext={noop}
      />,
    )
    expect(html).toContain('Step 1 of 2')
    expect(html).toContain('uses your Claude Pro or Max plan, no API key')
    expect(html).toContain('Adds a new worker: ')
    expect(html).toContain('What happens when you continue')
    expect(html).toContain('Add the provider-claude-code worker')
    expect(html).toContain('Check that Claude Code models are ready')
    expect(html).toContain('iii is composable')
    // No function ids or versions.
    expect(html).not.toContain('router::models::list')
    expect(html).not.toContain('compose::add')
    expect(html).not.toMatch(/provider-claude-code@/)
  })

  it('holds the choices behind a skeleton until the machine is scanned', () => {
    const html = renderToStaticMarkup(
      <ModelsStep
        onboarding={controller({ tools: null })}
        onBack={noop}
        onNext={noop}
      />,
    )
    expect(html).toContain('Looking at this machine')
    expect(html).not.toContain('Recommended for this machine')
    expect(html).not.toContain('Add secrets worker and check for keys')
  })

  it('never asks to add the secrets worker: it comes with llm-router', () => {
    const html = renderToStaticMarkup(
      <ModelsStep onboarding={controller({})} onBack={noop} onNext={noop} />,
    )
    expect(html).not.toContain('Keys you already have')
    expect(html).not.toContain('Add secrets worker')
    // Nothing was looked for, so nothing is claimed about keys either.
    expect(html).not.toContain('No provider keys')
  })

  it('says so when the secrets worker found no provider key', () => {
    const html = renderToStaticMarkup(
      <ModelsStep
        onboarding={controller({
          installed: new Set(['secrets']),
          detections: [],
        })}
        onBack={noop}
        onNext={noop}
      />,
    )
    expect(html).toContain('No provider keys in your shell profile')
    expect(html).not.toContain('Add secrets worker and check for keys')
  })

  it('offers the found key, its masked hint and where it will be stored', () => {
    const html = renderToStaticMarkup(
      <ModelsStep
        onboarding={controller({
          tools: [],
          installed: new Set(['secrets']),
          detections: [
            {
              name: 'ANTHROPIC_API_KEY',
              stored: false,
              sources: [
                {
                  kind: 'dotenv',
                  location: '/p/.env',
                  hint: 'sk-ant…9f2c',
                  matches_stored: false,
                },
              ],
            },
          ],
        })}
        onBack={noop}
        onNext={noop}
      />,
    )
    expect(html).toContain('Use the key from this project&#x27;s .env')
    expect(html).toContain('sk-ant…9f2c')
    expect(html).toContain(
      'Stored encrypted on this machine. It never lands in a file you commit.',
    )
    expect(html).toContain('Store your Anthropic key encrypted on this machine')
    expect(html).toContain('Connect Anthropic with that key')
    expect(html).not.toContain('secret://')
    expect(html).not.toContain('llm-router')
    // llm-router reads env:// too, so the key may stay a variable instead.
    expect(html).toContain('role="radiogroup"')
    expect(html).toContain('Encrypted')
    expect(html).toContain('Environment variable')
  })
})

const CONNECTED: MachineSnapshot['providers'] = [
  {
    id: 'claude-code',
    title: 'Claude Code',
    configured: false,
    ownsAuthentication: true,
    available: true,
    modelCount: 11,
  },
  {
    id: 'anthropic',
    title: 'Anthropic',
    configured: true,
    available: true,
    modelCount: 9,
    credentialRef: 'secret://ANTHROPIC_API_KEY',
  },
]

const PROMPTS: ExamplePrompt[] = [
  {
    title: 'Build a TODO app',
    description: 'A todo list with notes',
    agent: 'ade-worker-builder',
    prompt: 'Build a TODO app.',
    models: [{ provider: 'claude-code', model: 'claude-sonnet-5-5' }],
  },
  {
    title: 'Explain this project',
    agent: 'default',
    prompt: 'Explain this project.',
    models: [],
  },
]

function ready(
  snapshot: Partial<MachineSnapshot>,
  activity: ActivityEntry[] = [],
  prompts: ExamplePrompt[] | null = PROMPTS,
) {
  return renderToStaticMarkup(
    <ReadyStep
      onboarding={controller(snapshot, activity)}
      judge={null}
      prompts={prompts}
      agentNames={new Map([['ade-worker-builder', 'Create an app or tool']])}
      onPrompt={noop}
      onFinish={noop}
    />,
  )
}

describe('ReadyStep', () => {
  it('sums up what is connected and every worker setup added, saying iii is composable', () => {
    const html = ready({ providers: CONNECTED }, [
      {
        id: 1,
        group: 'models',
        title: 'Add 2 workers: secrets and provider-claude-code',
        status: 'done',
        workers: ['secrets', 'provider-claude-code'],
      },
    ])
    expect(html).toContain('Your harness is ready')
    expect(html).toContain('Claude Code connected')
    expect(html).toContain('Workers added to your project (2)')
    expect(html).toContain('iii is composable: each worker adds new behavior')
    expect(html).toContain('worker-compose.yaml')
    expect(html).toContain('provider-claude-code')
  })

  it('never shows key references', () => {
    const html = ready({
      providers: [
        ...(CONNECTED ?? []),
        {
          id: 'openai',
          title: 'OpenAI',
          configured: true,
          available: true,
          modelCount: 4,
          credentialRef: 'env://OPENAI_API_KEY',
        },
      ],
    })
    expect(html).toContain('OpenAI connected')
    expect(html).toContain('4 models')
    expect(html).not.toContain('secret://')
    expect(html).not.toContain('env://')
  })

  it('offers the example prompts and Finish, not the tour', () => {
    const html = ready({ providers: CONNECTED })
    expect(html).toContain('Try an example')
    expect(html).toContain('aria-label="Start a chat: Build a TODO app"')
    expect(html).toContain('A todo list with notes')
    // Each card names the agent profile it runs with, as the gallery does.
    expect(html).toContain('Create an app or tool')
    expect(html).toContain('Default')
    expect(html).not.toContain('ade-worker-builder')
    expect(html).toContain('Finish')
    expect(html).not.toContain('Start the tour')
    expect(html).not.toContain('guided tour')
  })

  it('holds the cards behind a skeleton while the prompts are read', () => {
    const html = ready({ providers: CONNECTED }, [], null)
    expect(html).toContain('Loading example prompts')
    expect(html).not.toContain('Build a TODO app')
  })

  it('shows no examples section when the project declares none', () => {
    const html = ready({ providers: CONNECTED }, [], [])
    expect(html).not.toContain('Try an example')
    expect(html).toContain('Finish')
  })

  it('offers no prompts without a model to answer them', () => {
    const html = ready({ providers: [] })
    expect(html).not.toContain('Try an example')
    expect(html).toContain('Finish')
  })
})

describe('JudgeStep', () => {
  it('says which workers each choice adds, in plain words', () => {
    const html = renderToStaticMarkup(
      <JudgeStep
        onboarding={controller({ installed: new Set(['secrets']) })}
        onBack={noop}
        onNext={noop}
      />,
    )
    expect(html).toContain('Adds new workers: ')
    expect(html).toContain('judge-typesafe')
    expect(html).toContain('Have Judge answer with Jev by TypeSafe')
    expect(html).not.toContain('judge::models::list')
    expect(html).not.toContain('secret://')
    expect(html).not.toContain('GGUF')
  })
})

describe('withWorkerPresence', () => {
  it('drops a provider whose worker left, even while the router still lists it', () => {
    const providers = [
      {
        id: 'claude-code',
        title: 'Claude Code',
        configured: false,
        available: true,
        modelCount: 11,
      },
      {
        id: 'cursor',
        title: 'Cursor',
        configured: true,
        available: true,
        modelCount: 2,
      },
    ]
    const [claude, cursor] = withWorkerPresence(
      providers,
      new Set(['llm-router']),
    )
    expect(claude).toMatchObject({ available: false, modelCount: 0 })
    // Unknown worker name: the router's word stands.
    expect(cursor.modelCount).toBe(2)
    expect(withWorkerPresence(providers, null)[0].modelCount).toBe(11)
  })
})
