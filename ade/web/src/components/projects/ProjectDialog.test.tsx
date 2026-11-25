// @vitest-environment jsdom

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import { seedClaudeAccountsForTests } from '@/lib/backend/claude-accounts'
import type { ChatProject } from '@/lib/backend/projects'
import { usageFixture } from '@/stories/fixtures/claude-accounts-fixtures'
;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

// The engine's folder picker is its own suite; here it hands back one path.
vi.mock('@/components/chat/DirectoryPicker', () => ({
  DirectoryPicker: ({
    onChange,
    emptyLabel,
  }: {
    onChange: (dir: string) => void
    emptyLabel: string
  }) => (
    <button type="button" onClick={() => onChange(pickNext.path)}>
      {emptyLabel}
    </button>
  ),
}))
const pickNext = { path: '/home/u/work/acme/api' }

import { ProjectDialog } from './ProjectDialog'

const billing: ChatProject = {
  id: 'prj_billing',
  name: 'billing-api',
  color: 'blue',
  folders: ['/home/u/work/billing'],
  created_at_ms: 0,
  updated_at_ms: 0,
}

const conversations = [
  { id: 'a', workingDir: '/home/u/work/acme/web', projectId: undefined },
  { id: 'b', workingDir: '/home/u/work/acme/web/src', projectId: 'none' },
  { id: 'c', workingDir: '/home/u/work/acme/api' },
  { id: 'd', workingDir: '/home/u/work/acme/web', projectId: 'prj_other' },
  { id: 'child', workingDir: '/home/u/work/acme/web', parentId: 'a' },
]

let root: Root | null = null
let host: HTMLElement | null = null

function render(props: Partial<Parameters<typeof ProjectDialog>[0]> = {}) {
  const save = vi.fn(async (input: { name: string }) => ({
    project: { ...billing, id: 'prj_new', name: input.name },
    moved: 3,
  }))
  const onSaved = vi.fn()
  host = document.createElement('div')
  document.body.appendChild(host)
  root = createRoot(host)
  act(() => {
    root?.render(
      <TooltipProvider delayDuration={0}>
        <ProjectDialog
          open
          onOpenChange={vi.fn()}
          projects={[billing]}
          conversations={conversations}
          initialFolders={['/home/u/work/acme/web/']}
          save={save as never}
          onSaved={onSaved}
          {...props}
        />
      </TooltipProvider>,
    )
  })
  return { save, onSaved }
}

const dialog = () =>
  document.body.querySelector('[role="dialog"]') as HTMLElement
const button = (text: string) =>
  [...dialog().querySelectorAll('button')].find(
    (b) =>
      b.textContent?.trim() === text || b.getAttribute('aria-label') === text,
  ) as HTMLButtonElement | undefined
const nameInput = () =>
  dialog().querySelector('input[name="project-name"]') as HTMLInputElement

function typeName(value: string) {
  const setter = Object.getOwnPropertyDescriptor(
    HTMLInputElement.prototype,
    'value',
  )?.set
  act(() => {
    setter?.call(nameInput(), value)
    nameInput().dispatchEvent(new Event('input', { bubbles: true }))
  })
}

afterEach(() => {
  act(() => root?.unmount())
  host?.remove()
  root = null
  pickNext.path = '/home/u/work/acme/api'
})

describe('New project / Edit project (§1.3.2)', () => {
  it("has the spec's copy, six swatches and a counted folder row", () => {
    render()
    const text = dialog().textContent ?? ''
    expect(text).toContain('New project')
    expect(text).toContain(
      'A project groups folders and the conversations started in them.',
    )
    expect(text).toContain('Name')
    expect(text).toContain('Colour')
    expect(text).toContain('Folders')
    expect(text).toContain('~/work/acme/web')
    // Roots only: the subagent is not counted.
    expect(text).toContain('3 conversations')
    expect(text).toContain('Add folder…')
    expect(button('Create project')).toBeDefined()
    const swatches = [...dialog().querySelectorAll('[role="radio"]')].map((s) =>
      s.getAttribute('aria-label'),
    )
    // billing-api took blue, so a new project starts on the next colour.
    expect(swatches).toEqual([
      'Blue',
      'Violet, selected',
      'Teal',
      'Green',
      'Amber',
      'Rose',
    ])
    // Two conversations have no project (missing, or "none").
    expect(text).toContain('Bring the 2 conversations already in these folders')
  })

  it('says what is missing before calling the engine', async () => {
    const { save } = render({ initialFolders: [] })
    await act(async () => {
      button('Create project')?.click()
    })
    const text = dialog().textContent ?? ''
    expect(text).toContain('Name is required')
    expect(text).toContain('Add at least one folder')
    expect(save).not.toHaveBeenCalled()
  })

  it('refuses a name or a folder another project has', async () => {
    const { save } = render()
    typeName('Billing-API')
    await act(async () => {
      button('Create project')?.click()
    })
    expect(dialog().textContent).toContain(
      'A project named “billing-api” already exists',
    )
    pickNext.path = '/home/u/work/billing'
    act(() => button('Add folder…')?.click())
    expect(dialog().textContent).toContain(
      '~/work/billing is already in “billing-api”',
    )
    expect(save).not.toHaveBeenCalled()
  })

  it('creates with the folders and bring_existing, and removes a folder', async () => {
    const { save, onSaved } = render()
    typeName(' acme-web ')
    act(() => button('Add folder…')?.click())
    expect(dialog().textContent).toContain('~/work/acme/api')
    expect(dialog().textContent).toContain('1 conversation')
    await act(async () => {
      button('Create project')?.click()
    })
    expect(save).toHaveBeenCalledWith(
      expect.objectContaining({
        name: 'acme-web',
        color: 'purple',
        folders: ['/home/u/work/acme/web', '/home/u/work/acme/api'],
        bringExisting: true,
      }),
    )
    expect(onSaved).toHaveBeenCalled()
  })

  it("shows the engine's name_taken copy, naming the existing project", async () => {
    const failing = vi.fn(() =>
      Promise.reject({
        code: 'invocation_failed',
        message: `handler error: ${JSON.stringify({
          code: 'name_taken',
          message: 'A project named “acme-web” already exists',
          project_id: 'prj_acme',
          project_name: 'acme-web',
        })}`,
      }),
    )
    render({ save: failing as never })
    typeName('ACME-WEB')
    await act(async () => {
      button('Create project')?.click()
    })
    const text = dialog().textContent ?? ''
    expect(text).toContain('A project named “acme-web” already exists')
    expect(text).not.toContain('“ACME-WEB”')
  })

  it("puts the engine's JSON error under its field", async () => {
    const failing = vi.fn(() =>
      Promise.reject({
        code: 'invocation_failed',
        message: `handler error: ${JSON.stringify({
          code: 'folder_taken',
          message: 'taken',
          project_id: 'prj_x',
          project_name: 'infra',
          folder: '/home/u/work/acme/web',
        })}`,
      }),
    )
    render({ save: failing as never })
    typeName('acme-web')
    await act(async () => {
      button('Create project')?.click()
    })
    expect(dialog().textContent).toContain(
      '~/work/acme/web is already in “infra”',
    )
  })

  it('edits a project: its values, Save, and a patch of what changed', async () => {
    const { save } = render({
      project: billing,
      initialFolders: undefined,
    })
    expect(dialog().textContent).toContain('Edit project')
    expect(nameInput().value).toBe('billing-api')
    typeName('billing')
    await act(async () => {
      button('Save')?.click()
    })
    expect(save).toHaveBeenCalledWith(
      expect.objectContaining({
        id: 'prj_billing',
        name: 'billing',
        changed: { name: true, color: false, folders: false },
      }),
    )
    act(() => button('Remove folder')?.click())
    expect(dialog().textContent).not.toContain('~/work/billing')
  })
})

describe("the project's Claude Code account (§4.3.2)", () => {
  const section = () =>
    dialog().querySelector('[data-project-account]') as HTMLElement | null
  const label = (name: string) =>
    [...dialog().querySelectorAll('label')].find(
      (l) => l.textContent === name,
    ) as HTMLLabelElement | undefined

  afterEach(() => seedClaudeAccountsForTests(null))

  it('is absent short of the whole feature (§4.3.6)', () => {
    seedClaudeAccountsForTests({ usage: usageFixture(0), gate: 'update' })
    const { save } = render()
    expect(section()).toBeNull()
    expect(save).not.toHaveBeenCalled()
  })

  it('follows the global account by default, and creates with it', async () => {
    seedClaudeAccountsForTests({ usage: usageFixture(0) })
    const { save } = render()
    expect(section()?.textContent).toContain('Claude Code account')
    expect(section()?.textContent).toContain('Follow the global account')
    expect(section()?.textContent).toContain('Work now')
    expect(section()?.textContent).toContain(
      'Other providers (Codex, API keys) ignore this.',
    )
    // A pin starts on another account than the global: none has room
    // (Personal is at its 5-hour limit), so the first signed-in one.
    expect(
      section()?.querySelector('[data-project-account-pin]')?.textContent,
    ).toBe('Personal · 22% this week')
    typeName('acme-web')
    await act(async () => {
      button('Create project')?.click()
    })
    expect(save).toHaveBeenCalledWith(
      expect.objectContaining({ account: { mode: 'global' } }),
    )
  })

  it("edits a pinned project's account as part of the patch", async () => {
    seedClaudeAccountsForTests({ usage: usageFixture(0) })
    const { save } = render({
      project: {
        ...billing,
        account: { mode: 'pinned', account_id: 'personal' },
      },
      initialFolders: undefined,
    })
    expect(
      section()?.querySelector('[data-project-account-pin]')?.textContent,
    ).toBe('Personal · 22% this week')
    act(() => label('Follow the global account')?.click())
    await act(async () => {
      button('Save')?.click()
    })
    expect(save).toHaveBeenCalledWith(
      expect.objectContaining({
        account: { mode: 'global' },
        changed: { name: false, color: false, folders: false, account: true },
      }),
    )
  })
})
