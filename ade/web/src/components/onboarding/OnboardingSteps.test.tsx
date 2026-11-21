import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import type { ToolScan } from '@/lib/onboarding/plan'
import type { ExamplePrompt } from '@/lib/onboarding/prompts'
import { ExampleList, ExploreStep } from './ExploreStep'
import { ModelsStep } from './ModelsStep'
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
    signInNeeded: new Set(),
    skipStep: () => undefined,
    checkChromium: async () => null,
    installChromium: async () => ({ ok: true as const }),
    chromiumProgress: null,
  }
}

const noop = () => undefined

/** The harness template's four, as `ade::onboarding::prompts` serves them. */
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
    // Installed but not signed in: in its place, saying why. Optional: the
    // commands wait until it is ticked (see ModelsStep.interaction.test).
    expect(html).toContain('Not signed in')
    expect(html).toContain('not a ChatGPT account')
    expect(html).toContain('aria-label="Connect Codex"')
    expect(html).not.toContain('codex login')
  })

  it('keeps a missing coding agent and the GitHub sign-in folded until ticked', () => {
    const html = renderToStaticMarkup(
      <ModelsStep
        onboarding={controller({ tools: [] })}
        onBack={noop}
        onNext={noop}
      />,
    )
    expect(html).toContain('Not installed')
    expect(html).toContain('aria-label="Connect Claude Code"')
    expect(html).not.toContain('npm install -g @anthropic-ai/claude-code')
    expect(html).toContain('aria-label="Connect GitHub Copilot"')
    expect(html).not.toContain('GitHub Copilot sign-in')
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
    // llm-router reads env:// too, so the key may stay a variable instead
    // (a shadcn toggle group, one pressed item).
    expect(html).toContain('data-variant="radio"')
    expect(html).toContain('aria-pressed="true"')
    expect(html).toContain('Encrypted')
    expect(html).toContain('Environment variable')
  })
})

function examples(prompts: readonly ExamplePrompt[] | null) {
  return renderToStaticMarkup(
    <ExampleList prompts={prompts} picked={null} onPick={noop} />,
  )
}

function explore(
  connected: boolean,
  prompts: readonly ExamplePrompt[] | null = PROMPTS,
) {
  return renderToStaticMarkup(
    <ExploreStep
      connected={connected}
      prompts={prompts}
      onPrompt={noop}
      tour={{ kind: 'idle' }}
      onTour={noop}
      onOwn={noop}
      onFinish={noop}
      onBack={noop}
    />,
  )
}

describe('ExploreStep', () => {
  it('offers the examples, the tour and exploring alone as one pick', () => {
    const html = explore(true)
    expect(html).toContain('Explore iii')
    expect(html).toContain('Build an example project')
    expect(html).toContain('Take the learn-iii tour')
    expect(html).toContain('Explore on my own')
    // A single-choice group with Explore on my own ticked; the footer's
    // button says what it does, and nothing is disabled.
    expect(html.match(/role="radio"/g)).toHaveLength(3)
    expect(html.match(/aria-checked="false"/g)).toHaveLength(2)
    expect(html.match(/aria-checked="true"/g)).toHaveLength(1)
    expect(html).toMatch(/<footer[\s\S]*Explore on my own[\s\S]*<\/footer>/)
    expect(html).not.toContain('Finish')
    expect(html).not.toContain('disabled=""')
    // Without a model exploring alone is the only pick, and it is ticked.
    const offline = explore(false)
    expect(offline.match(/aria-checked="true"/g)).toHaveLength(1)
    expect(offline).toMatch(/<footer[\s\S]*Explore on my own[\s\S]*<\/footer>/)
  })

  // QA 2026-11-16 F6: the picks drew round radio dots while Models and
  // Judge use checkboxes. Every choice list is a checkbox (single choice
  // ticks one).
  it('draws each pick as a checkbox, never a radio dot', () => {
    const html = explore(true)
    expect(html.match(/data-pick-mark="checkbox"/g)).toHaveLength(3)
    expect(html).not.toMatch(/data-pick-mark="[^"]*"[^>]*rounded-full/)
  })

  // QA 2026-11-16 F17: choosing the tour added registry workers while the
  // wizard said only "Preparing the tour…".
  it('says which worker it adds while the tour gets ready', () => {
    const html = renderToStaticMarkup(
      <ExploreStep
        connected
        prompts={PROMPTS}
        onPrompt={noop}
        tour={{
          kind: 'preparing',
          adding: 'onboarding',
          note: 'Adding to worker-compose.yaml',
        }}
        onTour={noop}
        onOwn={noop}
        onFinish={noop}
        onBack={noop}
      />,
    )
    expect(html).toContain('role="status"')
    expect(html).toContain('Adding the onboarding worker from the registry')
    expect(html).toContain('Adding to worker-compose.yaml')
    expect(html).toContain('Preparing the tour')
  })

  it('needs a model for the examples and the tour, not for exploring alone', () => {
    const html = explore(false)
    expect(html).toContain('Connect a model first.')
    expect(html.match(/disabled=""/g)).toHaveLength(2)
  })

  it('shows why the tour could not start', () => {
    const html = renderToStaticMarkup(
      <ExploreStep
        connected
        prompts={PROMPTS}
        onPrompt={noop}
        tour={{ kind: 'failed', error: 'compose is not running' }}
        onTour={noop}
        onOwn={noop}
        onFinish={noop}
        onBack={noop}
      />,
    )
    expect(html).toContain('The tour could not start: compose is not running')
  })
})

describe('ExampleList', () => {
  it('offers the four example prompts to pick from, each with its agent', () => {
    const html = examples(PROMPTS)
    expect(html.match(/role="radio"/g)).toHaveLength(4)
    expect(html).toContain('Build a link shortener')
    expect(html).toContain('Build an expense tracker')
    expect(html).toContain('Create a test reviewer agent')
    expect(html).toContain('Explain this project')
    // The message itself waits for the click; the card never shows it.
    expect(html).not.toContain('link-shortener')
  })

  it('shows placeholders while the prompts are read', () => {
    expect(examples(null)).toContain('Loading example prompts')
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
