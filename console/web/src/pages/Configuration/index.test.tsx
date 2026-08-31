import { renderToStaticMarkup } from 'react-dom/server'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { WorkersConfigurationRoute } from '@/hooks/use-hash-route'
import { Configuration } from '.'
import type { ConfigurationSchemaView } from './tabs/WorkersTab/api'

const harness = vi.hoisted(() => ({
  narrow: false,
  entries: [
    {
      id: 'browser',
      name: 'Browser',
      description: 'Browser sessions and automation.',
      schema: { type: 'object' },
    },
    {
      id: 'database',
      name: 'Database',
      description: 'Database connections.',
      schema: { type: 'object' },
    },
  ] as ConfigurationSchemaView[],
  route: {
    open: false,
    configurationId: null,
    fieldPath: [],
  } as WorkersConfigurationRoute,
  navigateWorker: vi.fn(),
}))

vi.mock('@/hooks/use-container-narrow', () => ({
  useContainerNarrow: () => [() => undefined, harness.narrow],
}))

vi.mock('@/hooks/use-hash-route', () => ({
  useWorkersConfigurationRoute: () => [
    harness.route,
    harness.navigateWorker,
    vi.fn(),
  ],
}))

vi.mock('./tabs/WorkersTab/hooks', () => ({
  useConfigurationsList: () => ({
    data: harness.entries,
    isLoading: false,
    isError: false,
    error: null,
  }),
  useWorkerRegistryReactivity: () => undefined,
}))

vi.mock('./tabs/ConsoleSettingsTab', () => ({
  ConsoleSettingsTab: () => <div data-testid="general-settings" />,
}))

vi.mock('./tabs/WorkersTab/WorkerEditor', () => ({
  WorkerEditor: ({ entry }: { entry: { id: string } }) => (
    <div data-testid="worker-editor" data-worker-id={entry.id} />
  ),
}))

const KNOWN_WORKER_CONFIGURATION_IDS = [
  'a2ui',
  'approval-gate',
  'bridge',
  'browser',
  'canvas',
  'claude-code',
  'code-runner',
  'codex',
  'computer',
  'console',
  'context-manager',
  'cron',
  'cursor',
  'database',
  'devin',
  'document',
  'editor',
  'email',
  'fp',
  'github',
  'grok',
  'harness',
  'http',
  'iii-directory',
  'llm-router',
  'memory',
  'memory-consolidate',
  'opencode',
  'openwiki',
  'pdf',
  'pi',
  'provider-xai',
  'pubsub',
  'queue',
  'rbac-proxy',
  'sandbox-code-runner',
  'scrapling',
  'security-scan',
  'session-manager',
  'shell',
  'slack',
  'state',
  'storage',
  'tailscale',
  'telegram-bot',
  'vscode',
  'web',
  'workflow',
  'worktree',
] as const

function renderConfiguration() {
  return renderToStaticMarkup(
    <Configuration
      theme="dark"
      onThemeChange={vi.fn()}
      onDirtyChange={vi.fn()}
      tryNavigate={(action) => {
        action()
        return true
      }}
    />,
  )
}

describe('Configuration settings workspace', () => {
  beforeEach(() => {
    harness.narrow = false
    harness.entries = [
      {
        id: 'browser',
        name: 'Browser',
        description: 'Browser sessions and automation.',
        schema: { type: 'object' },
      },
      {
        id: 'database',
        name: 'Database',
        description: 'Database connections.',
        schema: { type: 'object' },
      },
    ]
    harness.route = {
      open: false,
      configurationId: null,
      fieldPath: [],
    }
    harness.navigateWorker.mockClear()
  })

  it('shows General and every registered worker in the desktop sidebar', () => {
    const html = renderConfiguration()

    expect(html).toContain('data-testid="general-settings"')
    expect(html).toContain('Browser')
    expect(html).toContain('Database')
    expect(html).toContain('aria-current="page"')
  })

  it('renders the route-selected worker with the shared editor shell', () => {
    harness.route = {
      open: true,
      configurationId: 'browser',
      fieldPath: [],
    }

    const html = renderConfiguration()

    expect(html).toContain('data-testid="worker-editor"')
    expect(html).toContain('data-worker-id="browser"')
    expect(html).not.toContain('data-testid="general-settings"')
  })

  it('uses the worker root route as the narrow navigation landing page', () => {
    harness.narrow = true
    harness.route = {
      open: true,
      configurationId: null,
      fieldPath: [],
    }

    const html = renderConfiguration()

    expect(html).toContain('aria-label="Settings navigation"')
    expect(html).not.toContain('<main')
  })

  it('omits Console-owned persistence records from the worker catalog', () => {
    harness.entries = [
      ...harness.entries,
      {
        id: 'coder',
        name: 'Coder tombstone',
        description: 'Internal migration record.',
        schema: { type: 'object' },
      },
      {
        id: 'shell-ui',
        name: 'Shell UI state',
        description: 'Internal per-tab persistence.',
        schema: { type: 'object' },
      },
    ]

    const html = renderConfiguration()

    expect(html).not.toContain('Coder tombstone')
    expect(html).not.toContain('Shell UI state')
  })

  it('assigns an intentional icon to every known worker configuration', () => {
    harness.entries = KNOWN_WORKER_CONFIGURATION_IDS.map((id) => ({
      id,
      name: id,
      description: `${id} settings`,
      schema: { type: 'object' },
    }))

    const html = renderConfiguration()

    expect(html).not.toContain('lucide-box')
    expect(html).toContain('lucide-layout-template')
    expect(html).toContain('lucide-workflow')
  })

  it('keeps the family identity while distinguishing named instances', () => {
    harness.entries = [
      {
        id: 'browser-team-a',
        name: 'Browser',
        description: 'Team A browser sessions.',
        schema: { type: 'object' },
        metadata: { ui_form: 'browser' },
      },
    ]

    const html = renderConfiguration()

    expect(html).toContain('Browser · browser-team-a')
    expect(html).toContain('lucide-earth')
    expect(html).not.toContain('lucide-box')
  })
})
