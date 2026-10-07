import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import type { ToolScan } from '@/lib/onboarding/plan'
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
  it('reports each coding agent with where its sign-in lives, never what is in it', () => {
    const html = renderToStaticMarkup(
      <ModelsStep
        onboarding={controller({ installed: new Set(['llm-router']) })}
        onBack={noop}
        onNext={noop}
      />,
    )
    expect(html).toContain('Recommended for this machine')
    expect(html).toContain('Sign-in at')
    expect(html).toContain('~/.claude/.credentials.json')
    // Installed but not signed in: beside the recommendations, saying why.
    expect(html).toContain('Not signed in')
    expect(html).toContain('not a ChatGPT account')
  })

  it('recommends the signed-in agent and shows the plan before anything runs', () => {
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
    expect(html).toContain('What happens when you continue')
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
    expect(html).toContain('secret://ANTHROPIC_API_KEY')
    expect(html).toContain('Point llm-router at secret://ANTHROPIC_API_KEY')
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

function ready(
  snapshot: Partial<MachineSnapshot>,
  tour: TourState = { kind: 'idle' },
  activity: ActivityEntry[] = [],
) {
  return renderToStaticMarkup(
    <ReadyStep
      onboarding={controller(snapshot, activity)}
      judge={null}
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
    expect(html).toContain('key at secret://ANTHROPIC_API_KEY')
    expect(html).toContain('Your keys stay out of git')
    expect(html).toContain('configuration holds only secret:// references')
    expect(html).toContain('Workers added during setup (2)')
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
    expect(html).toContain('key at env://OPENAI_API_KEY')
    expect(html).toContain('Your keys stay out of configuration')
    expect(html).toContain(
      'configuration holds only secret:// and env:// references',
    )
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
