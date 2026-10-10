import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import type { ToolScan } from '@/lib/onboarding/plan'
import type { ExamplePrompt } from '@/lib/onboarding/prompts'
import { ModelsStep } from './ModelsStep'
import { ReadyStep, type TourState } from './ReadyStep'
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
      judgeProvider: null,
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
    installChromium: async () => ({ ok: true as const }),
    chromiumProgress: null,
  }
}

const noop = () => undefined

/** The harness template's four, as `console::onboarding::prompts` serves them. */
const PROMPTS: ExamplePrompt[] = [
  'Build a link shortener',
  'Build an expense tracker',
  'Create a test reviewer agent',
  'Explain this project',
].map((title, index) => ({
  title,
  description: `${title}, described`,
  agent: index === 3 ? 'default' : 'ade-worker-builder',
  prompt: `${title}.`,
  models: [{ provider: 'claude-code', model: 'claude-sonnet-5-5' }],
}))

describe('ModelsStep', () => {
  it('shows real marks for every built-in provider before workers are installed', () => {
    const html = renderToStaticMarkup(
      <ModelsStep
        onboarding={controller({ tools: [] })}
        onBack={noop}
        onNext={noop}
      />,
    )
    // Nine catalog providers plus GitHub Copilot, which signs in from here.
    expect(html.match(/data-provider-icon="mark"/g)).toHaveLength(10)
    expect(html).not.toContain('data-provider-icon="initial"')
  })

  it('reports each coding agent with where its sign-in lives, never what is in it', () => {
    const html = renderToStaticMarkup(
      <ModelsStep
        onboarding={controller({ installed: new Set(['llm-router']) })}
        onBack={noop}
        onNext={noop}
      />,
    )
    expect(html).toContain('Subscriptions')
    expect(html).toContain('Sign-in at')
    expect(html).toContain('~/.claude/.credentials.json')
    // Installed but not signed in: in its place, saying why and how.
    expect(html).toContain('Not signed in')
    expect(html).toContain('not a ChatGPT account')
    expect(html).toContain('codex login')
  })

  it('shows how to install and sign in to a coding agent that is missing', () => {
    const html = renderToStaticMarkup(
      <ModelsStep
        onboarding={controller({ tools: [] })}
        onBack={noop}
        onNext={noop}
      />,
    )
    expect(html).toContain('Not installed')
    expect(html).toContain('npm install -g @anthropic-ai/claude-code')
    expect(html).toContain('then type /login')
  })

  it('keeps a connected provider in its place, checked, with its models', () => {
    const html = renderToStaticMarkup(
      <ModelsStep
        onboarding={controller({
          installed: new Set(['llm-router', 'provider-claude-code']),
          providers: [
            {
              id: 'claude-code',
              title: 'Claude Code',
              configured: false,
              available: true,
              modelCount: 11,
            },
          ],
        })}
        onBack={noop}
        onNext={noop}
      />,
    )
    expect(html).toContain('11 models')
    expect(html).not.toContain('>Connected<')
    // Nothing left to change: straight on.
    expect(html).toContain('Continue')
  })

  it('recommends the signed-in agent and shows the plan before anything runs', () => {
    const html = renderToStaticMarkup(
      <ModelsStep
        onboarding={controller({ installed: new Set(['llm-router']) })}
        onBack={noop}
        onNext={noop}
      />,
    )
    expect(html).toContain('Connect a model')
    expect(html).toContain('Uses your Claude Pro or Max plan.')
    expect(html).toContain('Engine log')
    expect(html).toContain('2 queued')
    expect(html).toContain('Add the provider-claude-code worker')
    expect(html).toContain('router::models::list provider=claude-code')
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
    expect(html).not.toContain('Subscriptions')
    expect(html).not.toContain('Add secrets worker and check for keys')
  })

  it('never asks to add the secrets worker: it comes with llm-router', () => {
    const html = renderToStaticMarkup(
      <ModelsStep onboarding={controller({})} onBack={noop} onNext={noop} />,
    )
    expect(html).not.toContain('Keys you already have')
    expect(html).not.toContain('Add secrets worker')
    // Nothing was looked for, so nothing is claimed about keys either.
    expect(html).not.toContain('none found')
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
    expect(html).toContain('none found in your shell profile or .env')
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
    expect(html).toContain('Key found')
    expect(html).toContain('Use the key from this project&#x27;s .env')
    expect(html).toContain('sk-ant…9f2c')
    expect(html).toContain(
      'Stored encrypted on this machine. It never lands in a file you commit.',
    )
    expect(html).not.toContain('secret://')
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
    available: true,
    ownsAuthentication: true,
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

function ready(
  snapshot: Partial<MachineSnapshot>,
  tour: TourState = { kind: 'idle' },
  activity: ActivityEntry[] = [],
  prompts: readonly ExamplePrompt[] | null = [],
) {
  return renderToStaticMarkup(
    <ReadyStep
      onboarding={controller(snapshot, activity)}
      judges={[]}
      prompts={prompts}
      agentNames={new Map([['ade-worker-builder', 'Create an app or tool']])}
      onPrompt={noop}
      tour={tour}
      onStartTour={noop}
      onStart={noop}
    />,
  )
}

describe('ReadyStep', () => {
  it('sums up what is connected and every worker setup added', () => {
    const html = ready({ providers: CONNECTED }, { kind: 'idle' }, [
      {
        id: 1,
        group: 'models',
        title: 'Add 2 workers',
        detail: 'compose::add secrets provider-claude-code',
        status: 'done',
        workers: ['secrets', 'provider-claude-code'],
      },
    ])
    expect(html).toContain('Your harness is ready')
    expect(html).toContain('Claude Code connected')
    expect(html).toContain('11 models')
    // Where the keys live is not a summary line: it reads as jargon here.
    expect(html).not.toContain('secret://')
    expect(html).not.toContain('Your keys stay out of git')
    expect(html).not.toContain('configuration holds only')
    expect(html).toContain('Workers added')
    expect(html).toContain('provider-claude-code')
    expect(html).not.toContain('ready in every chat')
  })

  it('says where the keys are when one stays an environment variable', () => {
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
    expect(html).not.toContain('env://')
  })

  it('offers the guided tour in place of starter prompts, without naming its worker', () => {
    const html = ready({ providers: CONNECTED })
    expect(html).toContain('Keep going with a guided tour')
    expect(html).toContain('Start the tour')
    expect(html).toContain('Skip the tour')
    expect(html).not.toContain('Try one of these first')
    expect(html).not.toContain('onboarding worker')
  })

  it('shows the tour getting ready, and why it could not start', () => {
    expect(ready({ providers: CONNECTED }, { kind: 'preparing' })).toContain(
      'Preparing the tour…',
    )
    const failed = ready(
      { providers: CONNECTED },
      { kind: 'failed', error: 'compose is not running' },
    )
    expect(failed).toContain('The tour could not start: compose is not running')
    expect(failed).toContain('Try again')
  })

  it('offers the four example prompts as chats to start, each with its agent', () => {
    const html = ready({ providers: CONNECTED }, { kind: 'idle' }, [], PROMPTS)
    expect(html).toContain('Try an example')
    expect(html).toContain('aria-label="Start a chat: Build a link shortener"')
    expect(html).toContain('Build an expense tracker')
    expect(html).toContain('Create a test reviewer agent')
    expect(html).toContain('Explain this project')
    expect(html).toContain('Create an app or tool')
    expect(html).toContain('>Default<')
    // The message itself waits for the click; the card never shows it.
    expect(html).not.toContain('link-shortener')
  })

  it('shows placeholders while the prompts are read, and nothing for a project without any', () => {
    expect(
      ready({ providers: CONNECTED }, { kind: 'idle' }, [], null),
    ).toContain('Loading example prompts')
    expect(
      ready({ providers: CONNECTED }, { kind: 'idle' }, [], []),
    ).not.toContain('Try an example')
    // No model, no prompt to answer it.
    expect(
      ready({ providers: [] }, { kind: 'idle' }, [], PROMPTS),
    ).not.toContain('Try an example')
  })

  it('offers no tour without a model to run it', () => {
    const html = ready({ providers: [] })
    expect(html).not.toContain('Keep going with a guided tour')
    expect(html).toContain('Start building')
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

describe('JudgeStep', () => {
  it('says what Judge does and what to do, with the recommended strategy ticked', async () => {
    const { JudgeStep } = await import('./JudgeStep')
    const html = renderToStaticMarkup(
      <JudgeStep onboarding={controller({})} onBack={noop} onNext={noop} />,
    )
    expect(html).toContain('What Judge does')
    expect(html).toContain('Function search')
    expect(html).toContain('Chooses what to click on a page')
    expect(html).toContain('Choose who answers')
    expect(html).toContain('the first answers by default')
    expect(html).toContain('Recommended')
    expect(html).toMatch(
      /aria-label="Answer with Jev by TypeSafe"[^>]*checked=""/,
    )
    expect(html).toContain('Set up Judge')
    expect(html).toContain('Skip')
  })

  it('shows running strategies as running and offers to apply changes', async () => {
    const { JudgeStep } = await import('./JudgeStep')
    const html = renderToStaticMarkup(
      <JudgeStep
        onboarding={controller({
          installed: new Set(['judge', 'judge-typesafe']),
          judgeProvider: 'typesafe',
        })}
        onBack={noop}
        onNext={noop}
      />,
    )
    expect(html).toContain('Running')
    expect(html).not.toContain('Recommended')
    expect(html).toContain('tick to add, untick to remove')
    expect(html).toContain('Continue')
  })
})
