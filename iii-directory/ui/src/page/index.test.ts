import type { Host } from '@iii-dev/console-ui'
import { expect, it, vi } from 'vitest'
import { resolveBrowserPaneVisibility } from './browser'
import { agentsAdapter, COLLECTIONS } from './index'

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

/* System prompts are NOT a collection here: they have no authoring surface in
   the console (the chat picker only reads them). Pinned so the tab cannot
   quietly come back. */
it('browses skills and agent profiles only', () => {
  expect(COLLECTIONS.map((c) => c.value)).toEqual(['skills', 'agents'])
})

it('shows only the creation form when a narrow browser starts a new entry', () => {
  expect(
    resolveBrowserPaneVisibility({
      narrow: true,
      selected: null,
      creating: true,
    }),
  ).toEqual({ showSide: false, showDoc: true })

  expect(
    resolveBrowserPaneVisibility({
      narrow: true,
      selected: 'existing-agent',
      creating: false,
    }),
  ).toEqual({ showSide: false, showDoc: true })

  expect(
    resolveBrowserPaneVisibility({
      narrow: true,
      selected: null,
      creating: false,
    }),
  ).toEqual({ showSide: true, showDoc: false })

  expect(
    resolveBrowserPaneVisibility({
      narrow: false,
      selected: null,
      creating: true,
    }),
  ).toEqual({ showSide: true, showDoc: true })
})
