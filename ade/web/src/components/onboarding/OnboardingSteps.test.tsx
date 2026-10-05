import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import type { ToolScan } from '@/lib/onboarding/plan'
import { MachineStep } from './MachineStep'
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
      installed,
      consoleConfig: null,
      ...snapshot,
    },
    scanning: false,
    refresh: async () => undefined,
    activity,
    running: null,
    run: async () => true,
    secretsInstalled: installed.has('secrets'),
    judgeInstalled: installed.has('judge'),
  }
}

const noop = () => undefined

describe('MachineStep', () => {
  it('reports each coding agent with where its sign-in lives, never what is in it', () => {
    const html = renderToStaticMarkup(
      <MachineStep onboarding={controller({})} onBack={noop} onNext={noop} />,
    )
    expect(html).toContain('Signed in')
    expect(html).toContain('~/.claude/.credentials.json')
    expect(html).toContain('Not signed in')
    expect(html).toContain('not a ChatGPT account')
  })

  it('explains the secrets worker before offering to add it', () => {
    const html = renderToStaticMarkup(
      <MachineStep onboarding={controller({})} onBack={noop} onNext={noop} />,
    )
    expect(html).toContain('Add secrets worker and check for keys')
    expect(html).toContain('Values never reach the browser')
  })

  it('lists found keys by name, source and masked hint once the secrets worker runs', () => {
    const html = renderToStaticMarkup(
      <MachineStep
        onboarding={controller({
          installed: new Set(['secrets']),
          detections: [
            {
              name: 'ANTHROPIC_API_KEY',
              stored: false,
              sources: [
                {
                  kind: 'login_shell',
                  location: 'login shell (zsh)',
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
    expect(html).toContain('ANTHROPIC_API_KEY')
    expect(html).toContain('Found in your shell profile')
    expect(html).toContain('sk-ant…9f2c')
  })
})

describe('ModelsStep', () => {
  it('recommends the signed-in agent and shows the plan before anything runs', () => {
    const html = renderToStaticMarkup(
      <ModelsStep
        onboarding={controller({ installed: new Set(['llm-router']) })}
        onBack={noop}
        onNext={noop}
      />,
    )
    expect(html).toContain('Recommended for this machine')
    expect(html).toContain('uses your Claude Pro or Max plan, no API key')
    expect(html).toContain('What happens when you continue')
    expect(html).toContain('Add the provider-claude-code worker')
    expect(html).toContain('router::models::list provider=claude-code')
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
  })
})

describe('ReadyStep', () => {
  it('sums up what is connected and every worker setup added', () => {
    const html = renderToStaticMarkup(
      <ReadyStep
        onboarding={controller(
          {
            providers: [
              {
                id: 'claude-code',
                title: 'Claude Code',
                configured: false,
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
            ],
          },
          [
            {
              id: 1,
              group: 'models',
              title: 'Add 2 workers',
              detail: 'compose::add secrets provider-claude-code',
              status: 'done',
              workers: ['secrets', 'provider-claude-code'],
            },
          ],
        )}
        judge={null}
        onStart={noop}
      />,
    )
    expect(html).toContain('Your harness is ready')
    expect(html).toContain('Claude Code connected')
    expect(html).toContain('key at secret://ANTHROPIC_API_KEY')
    expect(html).toContain('Your keys stay out of git')
    expect(html).toContain('Workers added during setup (2)')
    expect(html).toContain('Give the agents a kanban board')
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
