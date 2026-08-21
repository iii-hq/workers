import type { Host } from '@iii-dev/console-ui'
import { expect, it, vi } from 'vitest'
import {
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

it('exposes the Harness default system prompt as read only', async () => {
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
    fine: 'Read only',
    readOnly: true,
  })
  await expect(
    systemPromptsAdapter.load(host, HARNESS_DEFAULT_SYSTEM_PROMPT_KEY),
  ).resolves.toBe('canonical Harness prompt')
})
