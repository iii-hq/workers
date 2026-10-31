import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import type { PreviewMessage } from '@/lib/conversation-import'
import {
  callSummary,
  DiscoveryFailed,
  HistoryRow,
  homePath,
  ImportFooter,
  mergePages,
  PreviewPane,
  previewCounts,
  previewRows,
  previewTitle,
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
  it('shortens a source directory under the home folder', () => {
    expect(homePath('/Users/layon/.codex')).toBe('~/.codex')
    expect(homePath('/home/layon/.claude')).toBe('~/.claude')
    expect(homePath('/srv/codex')).toBe('/srv/codex')
  })

  it('renders elapsed time the way the sidebar does', () => {
    const now = 1_800_000_000_000
    expect(whenLabel(now - 2 * 60 * 60 * 1000, now)).toBe('2h ago')
    expect(whenLabel(now - 3000, now)).toBe('just now')
  })

  it('sums a call up as a verb and the argument that says the most', () => {
    expect(
      callSummary('Bash', {
        command: 'cd /repo/web && pnpm test\nmore',
        description: 'Run the web tests',
      }),
    ).toEqual({ kind: 'run', verb: 'Ran', detail: 'pnpm test' })
    expect(callSummary('exec', null)).toEqual({
      kind: 'run',
      verb: 'Ran a command',
      detail: '',
    })
    expect(callSummary('Read', { file_path: '/repo/src/App.tsx' })).toEqual({
      kind: 'read',
      verb: 'Read',
      detail: 'App.tsx',
    })
    expect(
      callSummary('apply_patch', {
        changes: { '/repo/a.ts': {}, '/repo/b.ts': {}, '/repo/c.ts': {} },
      }),
    ).toEqual({ kind: 'edit', verb: 'Edited', detail: 'a.ts and 2 more files' })
    expect(callSummary('Grep', { pattern: 'TODO' })).toMatchObject({
      verb: 'Searched',
      detail: 'TODO',
    })
    expect(callSummary('mcp__github__create_issue', {})).toMatchObject({
      kind: 'mcp',
      verb: 'create_issue',
      detail: 'github',
    })
    expect(callSummary('linear::search', {})).toMatchObject({ kind: 'mcp' })
    expect(callSummary('update_plan', { plan: [] })).toMatchObject({
      verb: 'Updated the plan',
    })
    expect(callSummary('Custom', { target: 'x', n: 2 })).toEqual({
      kind: 'other',
      verb: 'Custom',
      detail: 'x',
    })
  })
})

const transcript: PreviewMessage[] = [
  {
    id: 'u0',
    role: 'user',
    text: '<environment_context>\n<cwd>/repo</cwd>\n</environment_context>',
    timestamp: 0,
  },
  {
    id: 'u1',
    role: 'user',
    text: '<system-reminder>be brief</system-reminder>\nRun the tests',
    timestamp: 1,
  },
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
    // A prompt shows the person's words; a turn of pure context is left out.
    expect(rows[0]).toMatchObject({ role: 'user', text: 'Run the tests' })
    expect(rows[1]).toEqual({
      kind: 'call',
      id: 'a1:toolu_1',
      functionId: 'Bash',
      call: 'run',
      verb: 'Ran',
      detail: 'pnpm test',
      failed: true,
    })
    // A result whose call fell outside the preview window still shows.
    expect(rows[2]).toMatchObject({
      functionId: 'Read',
      verb: 'Read a file',
      failed: false,
    })
  })

  it('counts text turns and commands separately', () => {
    expect(previewCounts(transcript)).toEqual({ messages: 3, commands: 2 })
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

  it('renders commands as quiet one-line rows under the conversation header', () => {
    const html = renderToStaticMarkup(
      <PreviewPane
        source="claude-code"
        conversation={conversation}
        preview={{ conversation, messages: transcript, warnings: [] }}
        error=""
        loading={false}
      />,
    )
    expect(html).toContain('Group the sidebar')
    expect(html).toContain('data-function-id="Bash"')
    expect(html).toContain('data-function-status="error"')
    expect(html).toContain('pnpm test')
    expect(html).not.toContain('Called')
    expect(html).toContain('3 messages · 2 commands')
    expect(html).toContain('data-preview-role="user"')
    expect(html).not.toContain('environment_context')
  })

  it('says the preview window once, quietly, and keeps warn tone for real problems', () => {
    const html = renderToStaticMarkup(
      <PreviewPane
        source="codex"
        conversation={{ ...conversation, message_count: 48, command_count: 9 }}
        preview={{
          conversation,
          messages: transcript,
          warnings: [
            'Reasoning, attachments, and subagent activity are not imported.',
            'Preview shows the last 50 messages; import includes the full history.',
            'The last record is still being written and was omitted.',
          ],
        }}
        error=""
        loading={false}
      />,
    )
    expect(html).toContain('Showing the last 3 of 48 messages')
    expect(html).toContain('48 messages · 9 commands')
    expect(html).not.toContain('Reasoning, attachments')
    expect(html).toContain('text-warn')
    expect(html).toContain('The last record is still being written')
  })

  it('titles an untitled history by its first prompt when the preview starts there', () => {
    const untitled = {
      ...conversation,
      title: '<system_instruction> You are working inside Conductor, a Mac',
      created_at: Date.UTC(2026, 9, 25),
    }
    expect(
      previewTitle(untitled, {
        conversation: untitled,
        messages: transcript,
        warnings: [],
      }),
    ).toBe('Run the tests')
    expect(
      previewTitle(untitled, {
        conversation: untitled,
        messages: transcript,
        truncated: true,
        warnings: [],
      }),
    ).toBe('workers · Oct 25')
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
    expect(html).toContain('Nothing to preview')
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
    expect(html).toContain('workers · 5h ago')
    expect(html).toContain('history moved during import')
    expect(html).toContain('Failed')
  })

  it('titles a row by the person’s words and counts its messages', () => {
    const html = renderToStaticMarkup(
      <HistoryRow
        conversation={{
          ...conversation,
          title:
            '<system_instruction>\nYou are in Conductor.\n</system_instruction>\nFix the release lane',
          message_count: 12,
        }}
        selected={false}
        open={false}
        busy={false}
        now={1_800_000_000_000}
        onToggle={vi.fn()}
        onOpen={vi.fn()}
      />,
    )
    expect(html).toContain('Fix the release lane')
    expect(html).not.toContain('system_instruction')
    expect(html).toContain('workers · 5h ago · 12 messages')
  })
})

describe('discovery states', () => {
  it('offers the other source when one has no history', () => {
    const html = renderToStaticMarkup(
      <SourceUnavailable
        source="codex"
        directory="/Users/layon/.codex"
        onSwitch={vi.fn()}
        onRetry={vi.fn()}
      />,
    )
    expect(html).toContain('No Codex history on this machine')
    expect(html).toContain('~/.codex')
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
    expect(html).toContain('stay behind')
    expect(html).not.toContain('Open conversation')
  })

  it('names the count on the primary action, and disables it with none', () => {
    const footer = (selectedCount: number) =>
      renderToStaticMarkup(
        <ImportFooter
          phase="browse"
          selectedCount={selectedCount}
          canImport={selectedCount > 0}
          progress={{ done: 0, total: 0, title: '' }}
          summary={null}
          {...handlers}
        />,
      )
    expect(footer(1)).toContain('Import 1 conversation<')
    expect(footer(0)).toMatch(/disabled=""[^>]*>Import</)
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

describe('mergePages', () => {
  const row = (id: string) => ({
    id,
    source: 'codex' as const,
    title: id,
    cwd: null,
    created_at: 1,
    updated_at: 1,
  })
  const page = (ids: string[], next: string | null) => ({
    source: 'codex' as const,
    directory: '/home/layon/.codex/sessions',
    warnings: [],
    available: true,
    conversations: ids.map(row),
    next_cursor: next,
  })

  it('keeps a conversation once when a history that is being written shifts onto the next page', () => {
    const merged = mergePages(
      { ...page(['a', 'b'], '2'), skipped: 1 },
      { ...page(['b', 'c'], null), skipped: 2 },
    )
    expect(merged.conversations.map((c) => c.id)).toEqual(['a', 'b', 'c'])
    expect(merged.next_cursor).toBeNull()
    expect(merged.skipped).toBe(3)
  })
})
