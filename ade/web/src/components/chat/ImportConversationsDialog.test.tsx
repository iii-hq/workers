import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import type { PreviewMessage } from '@/lib/conversation-import'
import {
  argumentsDigest,
  DiscoveryFailed,
  HistoryRow,
  ImportFooter,
  PreviewPane,
  previewCounts,
  previewRows,
  projectLabel,
  SourceUnavailable,
  summaryText,
  whenLabel,
} from './ImportConversationsDialog'

describe('summaryText', () => {
  it('announces a single imported conversation', () => {
    expect(
      summaryText({
        imported: 1,
        messages: 67,
        commands: 0,
        failed: 0,
        lastSessionId: 'session-a',
      }),
    ).toBe('Imported 1 conversation · 67 messages')
  })

  it('announces a batch with the commands that came across', () => {
    expect(
      summaryText({
        imported: 2,
        messages: 70,
        commands: 12,
        failed: 0,
        lastSessionId: 'session-b',
      }),
    ).toBe('Imported 2 conversations · 70 messages · 12 commands')
  })

  it('keeps a partial failure visible beside what was imported', () => {
    expect(
      summaryText({
        imported: 1,
        messages: 1,
        commands: 1,
        failed: 1,
        lastSessionId: 'session-c',
      }),
    ).toBe('Imported 1 conversation · 1 message · 1 command · 1 failed')
  })

  it('does not claim an import when every conversation failed', () => {
    expect(
      summaryText({
        imported: 0,
        messages: 0,
        commands: 0,
        failed: 2,
        lastSessionId: null,
      }),
    ).toBe('Nothing imported · 2 failed')
  })
})

describe('row copy', () => {
  it('names a project by its last path segment', () => {
    expect(projectLabel('/home/layon/workspaces/workers')).toBe('workers')
    expect(projectLabel('C:\\src\\harness-e2e\\')).toBe('harness-e2e')
    expect(projectLabel(null)).toBe('No project')
  })

  it('renders elapsed time the way the sidebar does', () => {
    const now = 1_800_000_000_000
    expect(whenLabel(now - 2 * 60 * 60 * 1000, now)).toBe('2h ago')
    expect(whenLabel(now - 3000, now)).toBe('just now')
  })

  it('digests call arguments like the chat function rows', () => {
    expect(
      argumentsDigest({
        command: 'pnpm -C ade/web test',
        description: 'Run the web tests',
        extra: true,
      }),
    ).toBe('command: "pnpm -C ade/web test", description: "Run the web tests"')
    expect(argumentsDigest({ changes: { a: 1 }, paths: [1, 2, 3] })).toBe(
      'changes: {…}, paths: [3]',
    )
    expect(argumentsDigest(null)).toBe('')
  })
})

const transcript: PreviewMessage[] = [
  { id: 'u1', role: 'user', text: 'Run the tests', timestamp: 1 },
  {
    id: 'a1',
    role: 'assistant',
    text: '',
    timestamp: 2,
    calls: [
      {
        id: 'toolu_1',
        function_id: 'Bash',
        arguments: { command: 'pnpm test' },
      },
    ],
  },
  {
    id: 'r1',
    role: 'function_result',
    text: 'FAIL 1 test',
    timestamp: 3,
    call_id: 'toolu_1',
    function_id: 'Bash',
    is_error: true,
  },
  {
    id: 'r-orphan',
    role: 'function_result',
    text: 'ok',
    timestamp: 4,
    call_id: 'toolu_lost',
    function_id: 'Read',
  },
  { id: 'a2', role: 'assistant', text: 'One test fails.', timestamp: 5 },
]

describe('previewRows', () => {
  it('places each call where it happened and folds its result in', () => {
    const rows = previewRows(transcript)
    expect(rows.map((row) => row.kind)).toEqual([
      'text',
      'call',
      'call',
      'text',
    ])
    expect(rows[1]).toEqual({
      kind: 'call',
      id: 'a1:toolu_1',
      functionId: 'Bash',
      digest: 'command: "pnpm test"',
      failed: true,
      answered: true,
    })
    // A result whose call fell outside the preview window still shows.
    expect(rows[2]).toMatchObject({ functionId: 'Read', failed: false })
  })

  it('counts text turns and commands separately', () => {
    expect(previewCounts(transcript)).toEqual({ messages: 2, commands: 2 })
  })
})

describe('preview pane', () => {
  const conversation = {
    id: '0198ff66-9317-7000-8000-000000000001',
    source: 'claude-code' as const,
    title: 'Group the sidebar',
    cwd: '/home/layon/workspaces/workers',
    created_at: 1,
    updated_at: 2,
  }

  it('renders commands as the console function row', () => {
    const html = renderToStaticMarkup(
      <PreviewPane
        source="claude-code"
        conversation={conversation}
        preview={{ conversation, messages: transcript, warnings: [] }}
        error=""
        loading={false}
      />,
    )
    expect(html).toContain('data-function-id="Bash"')
    expect(html).toContain('data-function-status="error"')
    expect(html).toContain('Failed <span class="text-ink">Bash</span>')
    expect(html).toContain('2 messages · 2 commands')
    expect(html).toContain('data-preview-role="user"')
  })

  it('asks for a selection before there is anything to show', () => {
    const html = renderToStaticMarkup(
      <PreviewPane
        source="codex"
        conversation={null}
        preview={null}
        error=""
        loading={false}
      />,
    )
    expect(html).toContain('Select a conversation to preview it.')
  })
})

describe('history row', () => {
  const conversation = {
    id: '0198ff66-9317-7000-8000-000000000002',
    source: 'codex' as const,
    title: 'Release lane next is dead',
    cwd: '/home/layon/workspaces/workers',
    created_at: 1,
    updated_at: 1_800_000_000_000 - 5 * 60 * 60 * 1000,
  }

  it('marks selection, the open preview, and the import outcome on the row', () => {
    const html = renderToStaticMarkup(
      <HistoryRow
        conversation={conversation}
        selected
        open
        status="failed"
        failure="history moved during import"
        busy={false}
        now={1_800_000_000_000}
        onToggle={vi.fn()}
        onOpen={vi.fn()}
      />,
    )
    expect(html).toContain('data-selected="true"')
    expect(html).toContain('type="checkbox"')
    expect(html).toContain('checked=""')
    expect(html).toContain('aria-current="true"')
    expect(html).toContain('data-status="failed"')
    expect(html).toContain('workers')
    expect(html).toContain('5h ago')
    expect(html).toContain('history moved during import')
    expect(html).toContain('Failed')
  })
})

describe('discovery states', () => {
  it('offers the other source when one has no history', () => {
    const html = renderToStaticMarkup(
      <SourceUnavailable
        source="codex"
        directory="/home/layon/.codex"
        onSwitch={vi.fn()}
        onRetry={vi.fn()}
      />,
    )
    expect(html).toContain('No Codex history on this machine')
    expect(html).toContain('/home/layon/.codex')
    expect(html).toContain('Try Claude Code')
    expect(html).toContain('Check again')
  })

  it('keeps the raw error in mono beside a retry', () => {
    const html = renderToStaticMarkup(
      <DiscoveryFailed
        error="permission denied (os error 13)"
        onRetry={vi.fn()}
      />,
    )
    expect(html).toContain('role="alert"')
    expect(html).toContain('Could not read the history directory')
    expect(html).toContain('permission denied (os error 13)')
    expect(html).toContain('Retry')
    expect(html).toContain('Copy error')
  })
})

describe('footer', () => {
  const noop = vi.fn()
  const handlers = {
    onCancel: noop,
    onImport: noop,
    onImportMore: noop,
    onRetryFailed: noop,
    onOpen: noop,
  }

  it('shows one primary action while browsing', () => {
    const html = renderToStaticMarkup(
      <ImportFooter
        phase="browse"
        selectedCount={3}
        canImport
        progress={{ done: 0, total: 0, title: '' }}
        summary={null}
        {...handlers}
      />,
    )
    expect(html).toContain('Import 3 conversations')
    expect(html).toContain('Cancel')
    expect(html).not.toContain('Open conversation')
  })

  it('reports determinate progress while importing', () => {
    const html = renderToStaticMarkup(
      <ImportFooter
        phase="importing"
        selectedCount={3}
        canImport={false}
        progress={{ done: 1, total: 3, title: 'Second conversation' }}
        summary={null}
        {...handlers}
      />,
    )
    expect(html).toContain('Importing 2 of 3')
    expect(html).toContain('Second conversation')
    expect(html).toContain('width:33%')
    expect(html).not.toContain('Cancel')
  })

  it('summarises a partial batch and keeps a way to retry', () => {
    const html = renderToStaticMarkup(
      <ImportFooter
        phase="done"
        selectedCount={0}
        canImport={false}
        progress={{ done: 3, total: 3, title: '' }}
        summary={{
          imported: 2,
          messages: 41,
          commands: 9,
          failed: 1,
          lastSessionId: 'session-z',
        }}
        {...handlers}
      />,
    )
    expect(html).toContain(
      'Imported 2 conversations · 41 messages · 9 commands · 1 failed',
    )
    expect(html).toContain('Open last imported')
    expect(html).toContain('Import more')
  })

  it('offers a retry when nothing was imported', () => {
    const html = renderToStaticMarkup(
      <ImportFooter
        phase="done"
        selectedCount={0}
        canImport={false}
        progress={{ done: 1, total: 1, title: '' }}
        summary={{
          imported: 0,
          messages: 0,
          commands: 0,
          failed: 1,
          lastSessionId: null,
        }}
        {...handlers}
      />,
    )
    expect(html).toContain('role="alert"')
    expect(html).toContain('Nothing imported · 1 failed')
    expect(html).toContain('Retry failed')
  })
})
