// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { PlanStep, ProviderState, ToolScan } from '@/lib/onboarding/plan'
import { ModelsStep } from './ModelsStep'
import type {
  ActivityEntry,
  MachineSnapshot,
  OnboardingController,
} from './use-onboarding'

vi.mock('@/lib/workers-registry', () => ({
  fetchRegistryProviders: () => Promise.resolve([]),
}))
// The sign-in talks to the engine; here it only has to stand in its place.
vi.mock('@/components/chat/DeviceSignIn', () => ({
  DeviceSignIn: ({ provider }: { provider: { providerId: string } }) => (
    <div data-testid="device-sign-in" data-provider={provider.providerId} />
  ),
}))

const CLAUDE: ToolScan = {
  id: 'claude-code',
  name: 'Claude Code',
  installed: true,
  signed_in: true,
  provider_worker: 'provider-claude-code',
}

const CODEX: ToolScan = {
  id: 'codex',
  name: 'Codex',
  installed: true,
  signed_in: true,
  provider_worker: 'provider-openai-codex',
}

const COPILOT: ProviderState = {
  id: 'github-copilot',
  title: 'GitHub Copilot',
  configured: true,
  available: true,
  modelCount: 4,
}

function controller(
  snapshot: Partial<MachineSnapshot>,
  run: OnboardingController['run'] = async () => true,
  extra: Partial<Omit<OnboardingController, 'snapshot' | 'run'>> = {},
): OnboardingController {
  return {
    snapshot: {
      tools: [CLAUDE, CODEX],
      toolsError: null,
      providers: [],
      providersError: null,
      detections: null,
      envFile: '.env',
      installed: new Set(['llm-router']),
      consoleConfig: null,
      judgeProvider: null,
      browser: null,
      browserError: null,
      ...snapshot,
    },
    scanning: false,
    refresh: async () => undefined,
    activity: [],
    running: null,
    run,
    signInNeeded: new Set(),
    skipStep: () => undefined,
    judgeInstalled: false,
    checkChromium: async () => null,
    installChromium: async () => ({ ok: true as const }),
    chromiumProgress: null,
    ...extra,
  }
}

let container: HTMLDivElement
let root: Root

beforeEach(() => {
  ;(
    globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
  ).IS_REACT_ACT_ENVIRONMENT = true
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
})

afterEach(() => {
  act(() => root.unmount())
  container.remove()
})

const checkbox = (title: string) => {
  const box = container.querySelector<HTMLInputElement>(
    `input[aria-label="Connect ${title}"]`,
  )
  if (!box) throw new Error(`no checkbox for ${title}`)
  return box
}

const button = (label: RegExp) => {
  const found = [...container.querySelectorAll('button')].find((entry) =>
    label.test(entry.textContent ?? ''),
  )
  if (!found) throw new Error(`no button ${label}`)
  return found
}

describe('ModelsStep choices', () => {
  it('keeps every checked provider checked when another one connects from here', () => {
    const noop = () => undefined
    act(() =>
      root.render(
        <ModelsStep onboarding={controller({})} onBack={noop} onNext={noop} />,
      ),
    )
    // Both signed-in agents start checked as recommendations. Claude Code is
    // left as it is; Codex is unticked and ticked again, the person's own.
    expect(checkbox('Claude Code').checked).toBe(true)
    expect(checkbox('Codex').checked).toBe(true)
    act(() => checkbox('Codex').click())
    act(() => checkbox('Codex').click())
    expect(checkbox('Codex').checked).toBe(true)

    // GitHub Copilot's sign-in lands: its models are now served.
    act(() =>
      root.render(
        <ModelsStep
          onboarding={controller({
            providers: [COPILOT],
            installed: new Set(['llm-router', 'provider-github-copilot']),
          })}
          onBack={noop}
          onNext={noop}
        />,
      ),
    )
    expect(checkbox('GitHub Copilot').checked).toBe(true)
    expect(checkbox('Claude Code').checked).toBe(true)
    expect(checkbox('Codex').checked).toBe(true)
  })

  it('moves on once the providers are connected', async () => {
    const onNext = vi.fn()
    const run = vi.fn(
      async (_group: string, _steps: readonly PlanStep[]) => true,
    )
    act(() =>
      root.render(
        <ModelsStep
          onboarding={controller({}, run)}
          onBack={() => undefined}
          onNext={onNext}
        />,
      ),
    )
    await act(async () => button(/^Connect/).click())
    expect(run).toHaveBeenCalledWith('models', expect.any(Array))
    expect(onNext).toHaveBeenCalledTimes(1)
  })

  it('stays on the step when connecting fails, so its log can be read', async () => {
    const onNext = vi.fn()
    act(() =>
      root.render(
        <ModelsStep
          onboarding={controller({}, async () => false)}
          onBack={() => undefined}
          onNext={onNext}
        />,
      ),
    )
    await act(async () => button(/^Connect/).click())
    expect(onNext).not.toHaveBeenCalled()
  })
})

describe('ModelsStep optional sign-ins', () => {
  const NOT_SIGNED_IN: ToolScan = { ...CODEX, signed_in: false }
  const MISSING: ToolScan = {
    ...CLAUDE,
    installed: false,
    signed_in: false,
  }

  it('shows the commands for a coding agent once it is ticked, and leaves it out of Connect', async () => {
    const run = vi.fn(
      async (_group: string, _steps: readonly PlanStep[]) => true,
    )
    act(() =>
      root.render(
        <ModelsStep
          onboarding={controller({ tools: [CLAUDE, NOT_SIGNED_IN] }, run)}
          onBack={() => undefined}
          onNext={() => undefined}
        />,
      ),
    )
    expect(checkbox('Codex').checked).toBe(false)
    expect(container.textContent).not.toContain('codex login')

    act(() => checkbox('Codex').click())
    expect(checkbox('Codex').checked).toBe(true)
    expect(container.textContent).toContain('codex login')
    expect(container.textContent).toContain(
      'Codex connects once it is signed in',
    )

    // Connect sets up Claude Code only: Codex has nothing to connect yet.
    await act(async () => button(/^Connect/).click())
    const steps = run.mock.calls[0]?.[1] ?? []
    const added = steps.flatMap((step) =>
      step.kind === 'add-workers' ? step.workers : [],
    )
    expect(added).toContain('provider-claude-code')
    expect(added).not.toContain('provider-openai-codex')

    act(() => checkbox('Codex').click())
    expect(container.textContent).not.toContain('codex login')
  })

  it('shows how to install a missing coding agent once it is ticked', () => {
    act(() =>
      root.render(
        <ModelsStep
          onboarding={controller({ tools: [MISSING, CODEX] })}
          onBack={() => undefined}
          onNext={() => undefined}
        />,
      ),
    )
    expect(container.textContent).not.toContain('npm install -g')
    act(() => checkbox('Claude Code').click())
    expect(container.textContent).toContain(
      'npm install -g @anthropic-ai/claude-code',
    )
    expect(container.textContent).toContain('then type /login')
  })

  it('opens the GitHub sign-in only when GitHub Copilot is ticked', () => {
    act(() =>
      root.render(
        <ModelsStep
          onboarding={controller({})}
          onBack={() => undefined}
          onNext={() => undefined}
        />,
      ),
    )
    const signIn = () =>
      container.querySelector('[data-testid="device-sign-in"]')
    expect(checkbox('GitHub Copilot').checked).toBe(false)
    expect(signIn()).toBeNull()
    act(() => checkbox('GitHub Copilot').click())
    expect(signIn()).not.toBeNull()
    act(() => checkbox('GitHub Copilot').click())
    expect(signIn()).toBeNull()
  })
})

describe('ModelsStep while a connected provider blinks out', () => {
  const CLAUDE_SERVING: ProviderState = {
    id: 'claude-code',
    title: 'Claude Code',
    configured: false,
    available: true,
    modelCount: 12,
    ownsAuthentication: true,
  }
  const installed = new Set(['llm-router', 'provider-claude-code'])
  const render = (providers: ProviderState[]) =>
    act(() =>
      root.render(
        <ModelsStep
          onboarding={controller({ providers, installed })}
          onBack={() => undefined}
          onNext={() => undefined}
        />,
      ),
    )

  it('keeps it checked: a blink is not the person unchecking it', () => {
    render([CLAUDE_SERVING])
    expect(checkbox('Claude Code').checked).toBe(true)
    // Re-registering after another provider signed in: no models for a moment.
    render([{ ...CLAUDE_SERVING, available: false, modelCount: 0 }])
    expect(checkbox('Claude Code').checked).toBe(true)
    render([CLAUDE_SERVING])
    expect(checkbox('Claude Code').checked).toBe(true)
  })

  it('still lets the person uncheck it', () => {
    render([CLAUDE_SERVING])
    act(() => checkbox('Claude Code').click())
    render([{ ...CLAUDE_SERVING, available: false, modelCount: 0 }])
    expect(checkbox('Claude Code').checked).toBe(false)
  })
})

describe('ModelsStep after Connect failed', () => {
  beforeEach(() => {
    // jsdom lays nothing out, so it has no scrollIntoView for the log.
    Element.prototype.scrollIntoView = vi.fn()
  })

  const FAILED: ActivityEntry = {
    id: 1,
    group: 'models',
    title: 'Check that Codex models are ready',
    status: 'failed',
    note: 'Codex rejected the current session. Sign in with ChatGPT again.',
  }

  it('offers a skip beside Connect, so setup can move on', async () => {
    const onNext = vi.fn()
    act(() =>
      root.render(
        <ModelsStep
          onboarding={controller({ tools: [CODEX] }, undefined, {
            activity: [FAILED],
          })}
          onBack={() => undefined}
          onNext={onNext}
        />,
      ),
    )
    expect(button(/^Connect/).disabled).toBe(false)
    await act(async () => button(/^Skip for now$/).click())
    expect(onNext).toHaveBeenCalledTimes(1)
  })

  it('offers the Codex sign-in from here once Codex rejected the local one', async () => {
    const run = vi.fn(
      async (_group: string, _steps: readonly PlanStep[]) => true,
    )
    act(() =>
      root.render(
        <ModelsStep
          onboarding={controller({}, run, {
            activity: [FAILED],
            signInNeeded: new Set(['openai-codex']),
          })}
          onBack={() => undefined}
          onNext={() => undefined}
        />,
      ),
    )
    expect(checkbox('Codex').checked).toBe(true)
    expect(container.textContent).toContain('Sign-in rejected')
    expect(
      container
        .querySelector('[data-testid="device-sign-in"]')
        ?.getAttribute('data-provider'),
    ).toBe('openai-codex')
    expect(container.textContent).toContain('codex login')

    // Connect sets up Claude Code only: Codex waits on its sign-in.
    await act(async () => button(/^Connect/).click())
    const steps = run.mock.calls[0]?.[1] ?? []
    expect(JSON.stringify(steps)).not.toContain('openai-codex')
  })

  it('leaves a provider that cannot sign in from here to its CLI', () => {
    act(() =>
      root.render(
        <ModelsStep
          onboarding={controller({}, undefined, {
            signInNeeded: new Set(['claude-code']),
          })}
          onBack={() => undefined}
          onNext={() => undefined}
        />,
      ),
    )
    expect(container.textContent).toContain('Sign-in rejected')
    expect(container.querySelector('[data-testid="device-sign-in"]')).toBeNull()
    expect(container.textContent).toContain('then type /login')
  })
})
