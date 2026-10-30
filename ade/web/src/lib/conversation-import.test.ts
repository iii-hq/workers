import { describe, expect, it, vi } from 'vitest'
import { getIiiClient } from '@/lib/iii-client'
import {
  conversationTitle,
  discoverConversations,
  discoveryNotices,
  fallbackTitle,
  importConversation,
  previewConversation,
  previewNotices,
  projectName,
  promptText,
  titleFromPrompt,
} from './conversation-import'

vi.mock('@/lib/iii-client', () => ({ getIiiClient: vi.fn() }))
it('uses read discovery/preview functions and imports only explicitly selected ids', async () => {
  const trigger = vi.fn().mockResolvedValue({})
  vi.mocked(getIiiClient).mockResolvedValue({ trigger } as never)
  await discoverConversations({
    source: 'codex',
    query: 'project',
    cursor: 'next',
  })
  await previewConversation('claude-code', 'history-1')
  await importConversation('claude-code', 'history-1')
  expect(trigger.mock.calls).toEqual([
    [
      'console::conversations::discover',
      { source: 'codex', query: 'project', cursor: 'next' },
    ],
    [
      'console::conversations::preview',
      { source: 'claude-code', id: 'history-1' },
    ],
    [
      'console::conversations::import',
      { source: 'claude-code', id: 'history-1' },
    ],
  ])
})

it('keeps separate server IDs when importing the same source twice', async () => {
  const trigger = vi
    .fn()
    .mockResolvedValueOnce({
      session_id: 'new-1',
      imported_messages: 3,
      total_messages: 3,
    })
    .mockResolvedValueOnce({
      session_id: 'new-2',
      imported_messages: 3,
      total_messages: 3,
    })
  vi.mocked(getIiiClient).mockResolvedValue({ trigger } as never)
  const first = await importConversation('codex', 'same-source')
  const second = await importConversation('codex', 'same-source')
  expect(first.session_id).toBe('new-1')
  expect(second.session_id).toBe('new-2')
  expect(trigger).toHaveBeenCalledTimes(2)
})

it('says a timed-out import may still be running instead of reporting a plain failure', async () => {
  const trigger = vi
    .fn()
    .mockRejectedValue(
      new Error(
        'Invocation timeout after 300000ms: console::conversations::import',
      ),
    )
  vi.mocked(getIiiClient).mockResolvedValue({ trigger } as never)
  await expect(importConversation('codex', 'big')).rejects.toThrow(
    /may still be running/,
  )
  trigger.mockRejectedValue(new Error('permission denied'))
  await expect(importConversation('codex', 'big')).rejects.toThrow(
    'permission denied',
  )
})

describe('titles', () => {
  const conductor = [
    '<system_instruction>',
    'You are working inside Conductor.',
    'Do not rename the branch.',
    '</system_instruction>',
    '',
    '<system_instruction>',
    'The user has attached these files. Read them before proceeding.',
    '- .context/attachments/zZIgBQ/image.png (63.3 KB)',
    '</system_instruction>',
    '',
    'you have to redesign this section. the current is not looking good .context/attachments/zZIgBQ/image.png',
  ].join('\n')

  it("keeps the person's words and drops injected context and attachment paths", () => {
    expect(promptText(conductor)).toBe(
      'you have to redesign this section. the current is not looking good',
    )
    expect(titleFromPrompt(conductor)).toBe('you have to redesign this section')
  })

  it("titles Codex and Claude Code prompts by the person's first sentence", () => {
    const codex =
      '# AGENTS.md instructions for /repo\n\n<INSTRUCTIONS>\nbe terse\n</INSTRUCTIONS>\n<environment_context>\n  <cwd>/repo</cwd>\n</environment_context>\nWhy does the release lane fail? It passed yesterday.'
    expect(titleFromPrompt(codex)).toBe('Why does the release lane fail?')
    const reminder =
      '<system-reminder>\nToday is Tuesday.\n</system-reminder>\n## Group the sidebar by project'
    expect(titleFromPrompt(reminder)).toBe('Group the sidebar by project')
  })

  it('has no title for a turn that was all context, even one cut short', () => {
    expect(
      titleFromPrompt(
        '<command-name>/clear</command-name>\n<command-message>clear</command-message>',
      ),
    ).toBeNull()
    // The title an older worker sent: the raw prompt clipped at 100 characters.
    expect(
      titleFromPrompt(
        '<system_instruction> You are working inside Conductor, a Mac app that lets the user run many coding',
      ),
    ).toBeNull()
  })

  it('leaves ordinary angle brackets in a question alone', () => {
    expect(titleFromPrompt('Why is a <div> inside a <p> invalid?')).toBe(
      'Why is a <div> inside a <p> invalid?',
    )
  })

  it('clips a long first line at a word', () => {
    const title = titleFromPrompt('word '.repeat(40)) ?? ''
    expect(title.endsWith('…')).toBe(true)
    expect(Array.from(title).length).toBeLessThanOrEqual(101)
  })

  it('falls back to the project and the day the history started', () => {
    const conversation = {
      id: 'x',
      source: 'codex' as const,
      title:
        '<system_instruction> You are working inside Conductor, a Mac app that lets',
      cwd: '/work/belo-horizonte',
      created_at: Date.UTC(2026, 9, 25, 21),
      updated_at: Date.UTC(2026, 9, 25, 22),
    }
    expect(conversationTitle(conversation)).toBe('belo-horizonte · Oct 25')
    expect(conversationTitle({ ...conversation, cwd: null })).toBe(
      'Conversation · Oct 25',
    )
    expect(
      conversationTitle({ ...conversation, title: 'Group the sidebar' }),
    ).toBe('Group the sidebar')
    expect(projectName('C:\\src\\harness-e2e\\')).toBe('harness-e2e')
    expect(fallbackTitle(undefined, Number.NaN)).toBe('Conversation')
  })
})

describe('notices', () => {
  it('turns an older worker’s boilerplate warnings into counts and flags', () => {
    expect(
      discoveryNotices({
        source: 'codex',
        directory: '/home/a/.codex',
        available: true,
        conversations: [],
        next_cursor: null,
        warnings: [
          '3 empty, unsupported, or unreadable histories were skipped.',
          'The directory is mounted read-only.',
        ],
      }),
    ).toEqual({ skipped: 3, problems: ['The directory is mounted read-only.'] })
    const conversation = {
      id: 'x',
      source: 'codex' as const,
      title: 't',
      cwd: null,
      created_at: 1,
      updated_at: 1,
    }
    expect(
      previewNotices({
        conversation,
        messages: [],
        warnings: [
          'Reasoning, attachments, and subagent activity are not imported.',
          'Preview shows the last 50 messages; import includes the full history.',
          'The last record is still being written and was omitted.',
        ],
      }),
    ).toEqual({
      truncated: true,
      problems: ['The last record is still being written and was omitted.'],
    })
    expect(
      previewNotices({
        conversation,
        messages: [],
        truncated: true,
        warnings: [],
      }),
    ).toEqual({ truncated: true, problems: [] })
  })
})
