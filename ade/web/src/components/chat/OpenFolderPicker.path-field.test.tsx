// @vitest-environment jsdom

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { OpenFolderPicker } from './OpenFolderPicker'

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

const { HOME, tree, validateWorkspaceDir } = vi.hoisted(() => {
  const HOME = '/Users/me'
  const tree: Record<string, Array<{ name: string; git?: boolean }>> = {
    [HOME]: [{ name: 'code' }],
    [`${HOME}/code`]: [
      { name: 'iii-effects-design', git: true },
      { name: 'iii-engine', git: true },
    ],
    [`${HOME}/code/iii-effects-design`]: [
      { name: 'packages' },
      { name: 'web', git: true },
    ],
    [`${HOME}/code/iii-effects-design/packages`]: [],
  }
  const validateWorkspaceDir = vi.fn(async (dir: string) => ({
    ok: true as const,
    path: dir,
  }))
  return { HOME, tree, validateWorkspaceDir }
})

vi.mock('@/hooks/use-media-query', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@/hooks/use-media-query')>()),
  // A desktop pointer: the field takes focus and keyboard hints show.
  useMediaQuery: (query: string) => query.includes('pointer: fine'),
}))

vi.mock('@/lib/project-index', async (importOriginal) => {
  const real = await importOriginal<typeof import('@/lib/project-index')>()
  const index = {
    home: HOME,
    roots: [HOME],
    folders: Object.entries(tree).flatMap(([dir, kids]) =>
      kids.map((k) => ({
        path: `${dir}/${k.name}`,
        name: k.name,
        git: !!k.git,
      })),
    ),
  }
  return {
    ...real,
    cachedProjectIndex: () => index,
    loadProjectIndex: async () => index,
    listFolder: async (dir: string) => ({
      folders: (tree[dir] ?? []).map((k) => ({
        path: `${dir}/${k.name}`,
        name: k.name,
        git: !!k.git,
      })),
      project: dir === `${HOME}/code/iii-effects-design`,
    }),
  }
})

// No saved folder (harness::folders): the picker starts at home, the field
// first (OpenFolderPicker.test.tsx covers the saved folders' top level).
vi.mock('@/lib/backend/folders', () => ({
  listSavedFolders: vi.fn().mockResolvedValue([]),
  upsertSavedFolder: vi.fn().mockResolvedValue(undefined),
}))

vi.mock('@/lib/working-dir', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@/lib/working-dir')>()),
  validateWorkspaceDir: (dir: string) => validateWorkspaceDir(dir),
}))

vi.mock('@/lib/iii-client', () => ({
  getIiiClient: async () => ({ call: async () => ({}) }),
}))

let root: Root
let host: HTMLDivElement
const onAdded = vi.fn()

beforeEach(() => {
  Element.prototype.scrollIntoView = vi.fn()
  host = document.createElement('div')
  document.body.appendChild(host)
  root = createRoot(host)
})

afterEach(() => {
  act(() => root.unmount())
  host.remove()
  onAdded.mockReset()
  validateWorkspaceDir.mockClear()
})

const settle = () => act(async () => {})
const input = () =>
  document.body.querySelector('input[name="project-path"]') as HTMLInputElement
const footer = () =>
  document.body.querySelector('[role="dialog"] .border-t')?.textContent ?? ''
/** The highlighted rows by folder name (each row's title is its path). */
const highlighted = () =>
  [...document.body.querySelectorAll('[data-active="true"]')].map((el) =>
    el.getAttribute('title')?.split('/').pop(),
  )

async function mount() {
  await act(async () => {
    root.render(
      <OpenFolderPicker open onOpenChange={vi.fn()} onAdded={onAdded} />,
    )
  })
  await settle()
}

/** Type into the field the way React sees it: a value, then an input event. */
async function type(value: string) {
  const field = input()
  const setter = Object.getOwnPropertyDescriptor(
    HTMLInputElement.prototype,
    'value',
  )?.set
  await act(async () => {
    setter?.call(field, value)
    field.dispatchEvent(new Event('input', { bubbles: true }))
  })
  await settle()
}

async function press(key: string, init: KeyboardEventInit = {}) {
  const field = input()
  field.setSelectionRange(field.value.length, field.value.length)
  await act(async () => {
    field.dispatchEvent(
      new KeyboardEvent('keydown', { key, bubbles: true, ...init }),
    )
  })
  await settle()
}

describe('OpenFolderPicker target folder', () => {
  it('a Tab-completed folder is the target, not its first child', async () => {
    await mount()
    await type('~/code/iii-')
    // Narrowing highlights the best match and offers it as the completion.
    expect(highlighted()).toContain('iii-effects-design')
    expect(footer()).toContain('~/code/iii-effects-design')

    await press('Tab')
    expect(input().value).toBe('~/code/iii-effects-design/')
    // The field now names a whole folder: nothing under it is highlighted
    // and the footer names that folder rather than `packages` or `web`.
    expect(highlighted()).toEqual([])
    expect(footer()).toContain('~/code/iii-effects-design')
    expect(footer()).not.toContain('packages')

    await press('Enter', { metaKey: true })
    expect(validateWorkspaceDir).toHaveBeenCalledWith(
      `${HOME}/code/iii-effects-design`,
    )
    expect(onAdded).toHaveBeenCalledWith(`${HOME}/code/iii-effects-design`)
  })

  it('a typed path ending in / adds that folder on Enter', async () => {
    await mount()
    await type('~/code/iii-effects-design/')
    expect(highlighted()).toEqual([])
    await press('Enter')
    expect(onAdded).toHaveBeenCalledWith(`${HOME}/code/iii-effects-design`)
  })

  it('arrows still reach the children when nothing is highlighted', async () => {
    await mount()
    await type('~/code/iii-effects-design/')
    await press('ArrowDown')
    // Repositories lead the listing, so `web` comes before `packages`.
    expect(highlighted()).toEqual(['web'])
    expect(footer()).toContain('~/code/iii-effects-design/web')
    await press('ArrowUp')
    expect(highlighted()).toEqual(['packages'])
  })

  it('ArrowUp from nothing highlighted lands on the last row', async () => {
    await mount()
    await type('~/code/iii-effects-design/')
    await press('ArrowUp')
    expect(highlighted()).toEqual(['packages'])
  })

  it('stepping into a plain folder makes it the target', async () => {
    await mount()
    await type('~/code/iii-effects-design/pack')
    expect(highlighted()).toEqual(['packages'])
    await press('Enter')
    expect(input().value).toBe('~/code/iii-effects-design/packages/')
    expect(highlighted()).toEqual([])
    expect(footer()).toContain('~/code/iii-effects-design/packages')
    await press('Enter')
    expect(onAdded).toHaveBeenCalledWith(
      `${HOME}/code/iii-effects-design/packages`,
    )
  })
})
