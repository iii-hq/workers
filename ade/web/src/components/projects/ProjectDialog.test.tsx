// @vitest-environment jsdom

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import { seedChatSettingsForTests } from '@/hooks/use-chat-settings'
import { seedClaudeAccountsForTests } from '@/lib/backend/claude-accounts'
import type { ChatProject } from '@/lib/backend/projects'
import { setChatCapabilitiesForTests } from '@/lib/chat-capabilities'
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
// No engine here: every call fails, so the seeded stores stand.
vi.mock('@/lib/iii-client', () => ({
  getIiiClient: async () => ({
    call: async () => {
      throw new Error('no engine in tests')
    },
  }),
}))

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

describe("the project's Auto-compact (§3.3.2)", () => {
  const section = () =>
    dialog().querySelector('[data-project-autocompact]') as HTMLElement | null
  const label = (name: string) =>
    [...dialog().querySelectorAll('label')].find(
      (l) => l.textContent === name,
    ) as HTMLLabelElement | undefined
  const field = () =>
    dialog().querySelector(
      'input[name="project-autocompact-tokens"]',
    ) as HTMLInputElement
  const preset = (tokens: number) =>
    dialog().querySelector(
      `[data-autocompact-preset="${tokens}"]`,
    ) as HTMLButtonElement
  const checked = () =>
    [...(section()?.querySelectorAll('[role="radio"]') ?? [])].findIndex(
      (r) => r.getAttribute('aria-checked') === 'true',
    )

  function typeTokens(value: string) {
    const setter = Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      'value',
    )?.set
    act(() => {
      setter?.call(field(), value)
      field().dispatchEvent(new Event('input', { bubbles: true }))
    })
  }

  const withPhase3 = () => {
    setChatCapabilitiesForTests({ autocompact: true, chatSettings: true })
    seedChatSettingsForTests({
      descriptions: { agent_writes: true },
      autocompact: { mode: 'tokens', tokens: 400_000 },
    })
  }

  afterEach(() => {
    setChatCapabilitiesForTests(null)
    seedChatSettingsForTests(null)
  })

  it('is absent on a harness without phase 3 (D168)', async () => {
    const { save } = render()
    expect(section()).toBeNull()
    typeName('acme-web')
    await act(async () => {
      button('Create project')?.click()
    })
    expect(save.mock.calls[0]?.[0]).not.toHaveProperty('autocompact')
  })

  it("has the spec's three choices, the presets and the hint", () => {
    withPhase3()
    render()
    const text = section()?.textContent ?? ''
    expect(text).toContain('Auto-compact')
    expect(text).toContain('Use the global setting')
    expect(text).toContain('at 400k')
    expect(text).toContain('Compact at')
    expect(text).toContain("Only at the model's limit")
    expect(text).toContain(
      "Whichever comes first: this size or the model's own limit. A 200k Claude model's limit is about 116k, so above that it still compacts at its own limit, as it does today.",
    )
    expect(
      [...dialog().querySelectorAll('[data-autocompact-preset]')].map(
        (b) => b.textContent,
      ),
    ).toEqual(['100k', '160k', '200k', '400k', '800k'])
    // A new project follows the global.
    expect(checked()).toBe(0)
  })

  it('creates with a preset size', async () => {
    withPhase3()
    const { save } = render()
    typeName('acme-web')
    act(() => preset(160_000).click())
    expect(preset(160_000).getAttribute('aria-pressed')).toBe('true')
    expect(field().value).toBe('160k')
    expect(checked()).toBe(1)
    await act(async () => {
      button('Create project')?.click()
    })
    expect(save).toHaveBeenCalledWith(
      expect.objectContaining({
        autocompact: { mode: 'tokens', tokens: 160_000 },
      }),
    )
  })

  it('typing a size picks "Compact at" and refuses one out of range', async () => {
    withPhase3()
    const { save } = render()
    // The dialog focuses its name once it is open; the refusal's focus
    // comes after that, as it does for a person.
    await act(async () => {
      for (let i = 0; i < 100 && document.activeElement !== nameInput(); i++)
        await new Promise((resolve) => requestAnimationFrame(resolve))
    })
    expect(document.activeElement).toBe(nameInput())
    typeName('acme-web')
    typeTokens('3M')
    expect(checked()).toBe(1)
    await act(async () => {
      button('Create project')?.click()
    })
    expect(save).not.toHaveBeenCalled()
    expect(section()?.querySelector('[role="alert"]')?.textContent).toBe(
      'Between 20k and 2M tokens',
    )
    expect(document.activeElement).toBe(field())
    typeTokens('1.2M')
    await act(async () => {
      button('Create project')?.click()
    })
    expect(save).toHaveBeenCalledWith(
      expect.objectContaining({
        autocompact: { mode: 'tokens', tokens: 1_200_000 },
      }),
    )
  })

  it("edits a project's size into the patch", async () => {
    withPhase3()
    const { save } = render({
      project: { ...billing, autocompact: { mode: 'tokens', tokens: 160_000 } },
      initialFolders: undefined,
    })
    expect(field().value).toBe('160k')
    expect(checked()).toBe(1)
    act(() => label("Only at the model's limit")?.click())
    await act(async () => {
      button('Save')?.click()
    })
    expect(save).toHaveBeenCalledWith(
      expect.objectContaining({
        autocompact: { mode: 'off' },
        changed: {
          name: false,
          color: false,
          folders: false,
          autocompact: true,
        },
      }),
    )
  })

  it('leaves an unchanged value out of the patch', async () => {
    withPhase3()
    const { save } = render({
      project: { ...billing, autocompact: { mode: 'off' } },
      initialFolders: undefined,
    })
    expect(checked()).toBe(2)
    typeName('billing')
    await act(async () => {
      button('Save')?.click()
    })
    expect(save).toHaveBeenCalledWith(
      expect.objectContaining({
        changed: expect.objectContaining({ autocompact: false }),
      }),
    )
  })

  it("shows the harness's typed rejection under the section (D167)", async () => {
    withPhase3()
    const failing = vi.fn(() =>
      Promise.reject({
        code: 'invocation_failed',
        message: `handler error: ${JSON.stringify({
          code: 'bad_tokens',
          message: 'Between 20k and 2M tokens',
        })}`,
      }),
    )
    render({ save: failing as never })
    typeName('acme-web')
    act(() => preset(800_000).click())
    await act(async () => {
      button('Create project')?.click()
    })
    expect(section()?.querySelector('[role="alert"]')?.textContent).toBe(
      'Between 20k and 2M tokens',
    )
  })

  it('opens on the section with its first choice focused', async () => {
    withPhase3()
    render({ project: billing, initialSection: 'autocompact' })
    await act(async () => {
      for (
        let i = 0;
        i < 100 && !section()?.contains(document.activeElement);
        i++
      )
        await new Promise((resolve) => requestAnimationFrame(resolve))
    })
    expect(section()?.contains(document.activeElement)).toBe(true)
  })
})
