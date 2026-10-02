import { describe, expect, it, vi } from 'vitest'
import setup from '../../../page'
import {
  type TerminalOutputRouter,
  terminalOutputRouterHost,
} from '../terminal-output-router'

vi.mock('../../function-trigger', () => ({
  createShellTriggerRenderer: () => ({}),
}))
vi.mock('../../function-trigger/AgentRunView', () => ({
  createAgentRunRenderer: () => ({}),
}))
vi.mock('../../function-trigger/FileChangesView', () => ({
  createFileChangesRenderer: () => ({}),
}))
vi.mock('../../function-trigger/FindRelevantView', () => ({
  createFindRelevantRenderer: () => ({}),
}))
vi.mock('../index', () => ({ ShellExplorerPage: () => null }))
vi.mock('../ShellTurnSummary', () => ({ ShellTurnSummary: () => null }))
vi.mock('../WorktreeSwitcher', () => ({ createWorktreeSwitcher: () => () => null }))

describe('terminal output router lifecycle', () => {
  it('owns one router for the loaded UI asset and disposes it on teardown', () => {
    const offOutput = vi.fn()
    const pageRegister = vi.fn((_registration: unknown) => vi.fn())
    const registerComposerControl = vi.fn((_control: unknown) => vi.fn())
    const host = {
      iii: {
        browserId: 'console-test',
        on: vi.fn(() => offOutput),
      },
      pages: { register: pageRegister },
      functionTriggers: { register: vi.fn(() => vi.fn()) },
      chat: { registerTurnSummary: vi.fn(() => vi.fn()), registerComposerControl },
    } as never

    const teardown = setup(host)
    const registration = pageRegister.mock.calls[0]?.[0] as {
      render: (props: object) => {
        props: { terminalRouter: TerminalOutputRouter }
      }
    }
    const terminalRouter = registration.render({}).props.terminalRouter

    expect(terminalOutputRouterHost(terminalRouter)).toBe(host)
    expect(offOutput).not.toHaveBeenCalled()
    // The worktree switcher sits in the composer's project strip.
    expect(registerComposerControl).toHaveBeenCalledWith(
      expect.objectContaining({ id: 'ide-worktree-switcher', placement: 'project' }),
    )

    teardown()

    expect(offOutput).toHaveBeenCalledOnce()
    expect(() => terminalOutputRouterHost(terminalRouter)).toThrow(
      'terminal output router is disposed',
    )
  })
})
