import type { Host } from '@iii-dev/console-ui'
import { expect, it, vi } from 'vitest'
import {
  agentsAdapter,
  HARNESS_DEFAULT_SYSTEM_PROMPT_KEY,
  systemPromptsAdapter,
} from './index'

vi.mock('@iii-dev/console-ui', () => ({
  Button: () => null,
  CodeEditor: () => null,
  Input: () => null,
  MarkdownPreview: () => null,
  PageHeader: () => null,
  PageShell: () => null,
  PageSidebar: () => null,
  SegmentedControl: () => null,
}))

it('exposes the Harness default as an editable copy-on-write entry', async () => {
  const trigger = vi.fn(async (functionId: string, payload?: unknown) => {
    if (functionId === 'directory::system-prompts::list') {
      return { prompts: [] }
    }
    if (functionId === 'harness::system-prompt::get') {
      expect(payload).toEqual({
        session_id: 'iii-directory:browser-1',
        default_only: true,
      })
      return {
        parts: [
          {
            kind: 'built_in',
            name: 'embedded harness default',
            body: 'canonical Harness prompt',
          },
        ],
      }
    }
    throw new Error(`unexpected function: ${functionId}`)
  })
  const host = {
    iii: { browserId: 'browser-1', trigger },
  } as unknown as Host

  const rows = await systemPromptsAdapter.list(host)
  const builtIn = rows.find(
    (row) => row.key === HARNESS_DEFAULT_SYSTEM_PROMPT_KEY,
  )

  expect(builtIn).toMatchObject({
    title: 'default',
    description: 'Harness default system prompt',
    fine: 'Built-in · edits save a local override',
    noDelete: true,
  })
  expect(builtIn?.readOnly).toBeUndefined()
  // The editor draft is wrapped in frontmatter so a save can create the
  // local `default` entry with the required name + description.
  const loaded = await systemPromptsAdapter.load(
    host,
    HARNESS_DEFAULT_SYSTEM_PROMPT_KEY,
  )
  expect(loaded).toContain('name: default')
  expect(loaded.endsWith('canonical Harness prompt')).toBe(true)
})

it('saving the built-in default creates the local override entry', async () => {
  const trigger = vi.fn(async (functionId: string, payload?: unknown) => {
    if (functionId === 'directory::system-prompts::create') {
      expect(payload).toMatchObject({ name: 'default' })
      return { name: 'default' }
    }
    throw new Error(`unexpected function: ${functionId}`)
  })
  const host = { iii: { browserId: 'browser-1', trigger } } as unknown as Host
  const draft =
    '---\nname: default\ndescription: Local override\n---\nEdited prompt.'
  await expect(
    systemPromptsAdapter.save?.(host, HARNESS_DEFAULT_SYSTEM_PROMPT_KEY, draft),
  ).resolves.toBe('default')

  // A renamed draft would create an entry that overrides nothing — refused.
  const renamed =
    '---\nname: my-prompt\ndescription: Local override\n---\nEdited prompt.'
  await expect(
    systemPromptsAdapter.save?.(
      host,
      HARNESS_DEFAULT_SYSTEM_PROMPT_KEY,
      renamed,
    ),
  ).rejects.toThrow('must keep the name "default"')
})

it('hides the built-in row once a local default exists', async () => {
  const trigger = vi.fn(async (functionId: string) => {
    if (functionId === 'directory::system-prompts::list') {
      return {
        prompts: [
          { name: 'default', description: 'Local override', modified_at: '' },
        ],
      }
    }
    throw new Error(`unexpected function: ${functionId}`)
  })
  const host = { iii: { browserId: 'browser-1', trigger } } as unknown as Host
  const rows = await systemPromptsAdapter.list(host)
  expect(
    rows.some((row) => row.key === HARNESS_DEFAULT_SYSTEM_PROMPT_KEY),
  ).toBe(false)
  expect(rows.some((row) => row.key === 'default')).toBe(true)
})

it('lists the bundled base agent as an editable copy-on-write row', async () => {
  const trigger = vi.fn(async (functionId: string) => {
    if (functionId === 'directory::agents::list') {
      return {
        agents: [
          {
            id: 'iii',
            name: 'iii',
            description: 'Base.',
            logo: null,
            icon: null,
            color: null,
            modified_at: '',
            builtin: true,
          },
          {
            id: 'lead',
            name: 'Lead',
            description: 'Leads.',
            logo: null,
            icon: 'code',
            color: null,
            modified_at: '2026-01-01T00:00:00Z',
          },
        ],
      }
    }
    throw new Error(`unexpected function: ${functionId}`)
  })
  const host = { iii: { browserId: 'browser-1', trigger } } as unknown as Host

  const rows = await agentsAdapter.list(host)
  expect(rows.find((row) => row.key === 'iii')).toMatchObject({
    title: 'iii',
    fine: 'Built-in · edits save a local override',
    noDelete: true,
  })
  expect(rows.find((row) => row.key === 'lead')?.noDelete).toBeUndefined()
})
