import type { ReactNode } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'

// The real components are supplied by the Console import map, not Node.
vi.mock('@iii-dev/console-ui', () => ({
  Tooltip: ({ children }: { children: ReactNode }) => children,
  Toolbar: ({ children, end }: { children: ReactNode; end?: ReactNode }) => (
    <div role="toolbar">
      {children}
      {end}
    </div>
  ),
  StatusBar: ({ children, end }: { children: ReactNode; end?: ReactNode }) => (
    <div>
      {children}
      {end}
    </div>
  ),
  useConfirm: () => ({ confirm: async () => true, dialog: null }),
}))

import {
  closeTerminalPanes,
  pruneTerminalConnectionCoordinators,
  reconcileTerminalWorkspaceLeases,
  TerminalWorkspace,
} from '../TerminalWorkspace'
import type { TerminalWorkspaceState } from '../terminal-layout'
import { createTerminalConnectionCoordinator } from '../terminal-session-state'

function twoTabWorkspace(): TerminalWorkspaceState {
  return {
    tabs: [
      {
        id: 'tab-1',
        title: 'zsh 1',
        layout: { type: 'pane', paneId: 'pane-1' },
      },
      {
        id: 'tab-2',
        title: 'zsh 2',
        layout: { type: 'pane', paneId: 'pane-2' },
      },
    ],
    panes: {
      'pane-1': { id: 'pane-1', cwd: '/repo' },
      'pane-2': { id: 'pane-2', cwd: '/repo' },
    },
    activeTabId: 'tab-2',
    focusedPaneId: 'pane-2',
  }
}

function threePaneWorkspace(): TerminalWorkspaceState {
  return {
    tabs: [
      {
        id: 'tab-1',
        title: 'zsh 1',
        layout: {
          type: 'split',
          id: 'split-horizontal',
          direction: 'horizontal',
          ratio: 0.5,
          first: { type: 'pane', paneId: 'pane-1' },
          second: {
            type: 'split',
            id: 'split-vertical',
            direction: 'vertical',
            ratio: 0.5,
            first: { type: 'pane', paneId: 'pane-2' },
            second: { type: 'pane', paneId: 'pane-3' },
          },
        },
      },
    ],
    panes: {
      'pane-1': { id: 'pane-1', cwd: '/repo' },
      'pane-2': { id: 'pane-2', cwd: '/repo' },
      'pane-3': { id: 'pane-3', cwd: '/repo' },
    },
    activeTabId: 'tab-1',
    focusedPaneId: 'pane-1',
  }
}

describe('TerminalWorkspace', () => {
  it('renders and selects terminal tabs', () => {
    const html = renderToStaticMarkup(
      <TerminalWorkspace
        state={twoTabWorkspace()}
        dispatch={() => undefined}
        root="/repo"
        visible={false}
        router={null}
        leaseStore={null}
        storageKey="test"
        connectionCoordinators={new Map()}
      />,
    )

    expect(html).toContain('zsh 1')
    expect(html).toContain('zsh 2')
    expect(html).toContain('aria-selected="true"')
  })

  it('keeps every tab mounted, the inactive one hidden', () => {
    const html = renderToStaticMarkup(
      <TerminalWorkspace
        state={twoTabWorkspace()}
        dispatch={() => undefined}
        root="/repo"
        visible={false}
        router={null}
        leaseStore={null}
        storageKey="test"
        connectionCoordinators={new Map()}
      />,
    )

    expect(html).toContain('data-terminal-pane-id="pane-1"')
    expect(html).toContain('data-terminal-pane-id="pane-2"')
    expect(
      html.match(/class="shui-terminal-tab-layout" hidden=""/g) ?? [],
    ).toHaveLength(1)
  })

  it('renders horizontal and vertical split separators', () => {
    const html = renderToStaticMarkup(
      <TerminalWorkspace
        state={threePaneWorkspace()}
        dispatch={() => undefined}
        root="/repo"
        visible={false}
        router={null}
        leaseStore={null}
        storageKey="test"
        connectionCoordinators={new Map()}
      />,
    )

    expect(html).toContain('aria-orientation="horizontal"')
    expect(html).toContain('aria-orientation="vertical"')
    expect(html.match(/role="separator"/g) ?? []).toHaveLength(2)
  })

  it('reclaims stored leases whose panes were not restored', async () => {
    const reclaimed: string[] = []
    const warnings = await reconcileTerminalWorkspaceLeases(
      [
        {
          paneId: 'pane-1',
          sessionId: 'session-1',
          reconnectToken: 'token-1',
          lastSequence: 1,
        },
        {
          paneId: 'orphan-pane',
          sessionId: 'orphan-session',
          reconnectToken: 'orphan-token',
          lastSequence: 2,
        },
      ],
      new Set(['pane-1']),
      async (lease) => {
        reclaimed.push(lease.sessionId)
        return null
      },
    )

    expect(reclaimed).toEqual(['orphan-session'])
    expect(warnings).toEqual([])
  })

  it('closes every pane at once and keeps the ones that failed', async () => {
    const started: string[] = []
    const settle = new Map<
      string,
      {
        resolve: (warning: string | null) => void
        reject: (error: unknown) => void
      }
    >()
    const closing = closeTerminalPanes(
      ['pane-1', 'pane-2', 'pane-3', 'pane-4'],
      (paneId) => {
        started.push(paneId)
        return new Promise((resolve, reject) => {
          settle.set(paneId, { resolve, reject })
        })
      },
    )

    // No close waits for another's shell to die.
    expect(started).toEqual(['pane-1', 'pane-2', 'pane-3', 'pane-4'])
    settle.get('pane-3')?.resolve(null)
    settle.get('pane-2')?.reject(new Error('terminal close failed'))
    settle.get('pane-1')?.resolve('lease storage is full')
    // The bus rejects with the handler's error body, not an Error.
    settle.get('pane-4')?.reject({ message: 'handler error: {"code":"S300","message":"pty close timed out"}' })

    await expect(closing).resolves.toEqual({
      closed: ['pane-1', 'pane-3'],
      messages: ['lease storage is full', 'terminal close failed', 'S300: pty close timed out'],
    })
  })

  it('prunes connection coordinators for closed panes', () => {
    const coordinators = new Map([
      ['pane-1', createTerminalConnectionCoordinator(() => 'request-1')],
      ['closed-pane', createTerminalConnectionCoordinator(() => 'request-2')],
    ])

    pruneTerminalConnectionCoordinators(coordinators, new Set(['pane-1']))

    expect([...coordinators.keys()]).toEqual(['pane-1'])
  })
})
