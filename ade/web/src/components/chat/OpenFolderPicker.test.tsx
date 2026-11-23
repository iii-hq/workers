// @vitest-environment jsdom

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { SavedFolder } from '@/lib/backend/folders'

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

const mocks = vi.hoisted(() => ({
  saved: [] as SavedFolder[],
  listed: [] as string[],
  upsert: vi.fn(async (path: string) => ({
    path,
    name: path,
    last_used_at: 9,
  })),
}))

vi.mock('@/lib/backend/folders', () => ({
  listSavedFolders: vi.fn(async () => mocks.saved),
  upsertSavedFolder: mocks.upsert,
}))
vi.mock('@/lib/project-index', async (importOriginal) => ({
  ...(await importOriginal<object>()),
  cachedProjectIndex: () => null,
  loadProjectIndex: vi.fn(async () => ({
    home: '/home/u',
    roots: ['/home/u'],
    folders: [],
  })),
  listFolder: vi.fn(async (dir: string) => {
    mocks.listed.push(dir)
    return {
      folders: [{ path: `${dir}/Documents`, name: 'Documents', git: false }],
      project: false,
    }
  }),
}))
vi.mock('@/lib/working-dir', async (importOriginal) => ({
  ...(await importOriginal<object>()),
  validateWorkspaceDir: vi.fn(async (path: string) => ({ ok: true, path })),
}))
vi.mock('@/hooks/use-media-query', async (importOriginal) => ({
  ...(await importOriginal<object>()),
  useMediaQuery: () => true,
}))

import { OpenFolderPicker } from './OpenFolderPicker'

let root: Root | null = null
let host: HTMLElement | null = null

async function open(onAdded = vi.fn()) {
  host = document.createElement('div')
  document.body.appendChild(host)
  root = createRoot(host)
  await act(async () => {
    root?.render(
      <OpenFolderPicker open onOpenChange={() => {}} onAdded={onAdded} />,
    )
  })
  // The saved folders and the index answer on later ticks.
  for (let i = 0; i < 4; i += 1) {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0))
    })
  }
  return onAdded
}

const dialog = () =>
  document.body.querySelector('[role="dialog"]') as HTMLElement
const field = () =>
  dialog().querySelector('input[name="project-path"]') as HTMLInputElement
const options = () =>
  [...dialog().querySelectorAll('[role="option"]')].map((o) =>
    o.textContent?.replace(/Add$/, '').trim(),
  )

beforeEach(() => {
  // jsdom has no layout; the highlight scrolls into view.
  Element.prototype.scrollIntoView ??= () => {}
  mocks.listed = []
  mocks.upsert.mockClear()
  mocks.saved = [
    { path: '/tmp/acme/web', name: 'acme-web', last_used_at: 4 },
    { path: '/tmp/acme/api', name: 'acme-api', last_used_at: 3 },
    { path: '/tmp/acme/infra', name: 'acme-infra', last_used_at: 2 },
    { path: '/tmp/acme/scratch', name: 'acme-scratch', last_used_at: 1 },
  ]
})

afterEach(() => {
  act(() => root?.unmount())
  host?.remove()
  root = null
})

describe('Add folder (OpenFolderPicker)', () => {
  it('opens on the saved folders, titled Add folder, without listing home', async () => {
    await open()
    const text = dialog().textContent ?? ''
    expect(text).toContain('Add folder')
    expect(text).toContain('Saved folders')
    expect(text).toContain('4 folders')
    expect(field().value).toBe('')
    expect(options()).toEqual([
      'acme-web/tmp/acme',
      'acme-api/tmp/acme',
      'acme-infra/tmp/acme',
      'acme-scratch/tmp/acme',
    ])
    // Home is listed only when asked for.
    expect(mocks.listed).toEqual([])
    expect(text).not.toContain('Documents')
  })

  it('adds a saved folder in one click', async () => {
    const onAdded = await open()
    const row = dialog().querySelector('[role="option"]') as HTMLElement
    await act(async () => {
      row.click()
    })
    expect(mocks.upsert).toHaveBeenCalledWith('/tmp/acme/web')
    expect(onAdded).toHaveBeenCalledWith('/tmp/acme/web')
  })

  it('browses home on Browse, with the saved folders as chips', async () => {
    await open()
    const browse = [...dialog().querySelectorAll('button')].find(
      (b) => b.textContent?.trim() === 'Browse',
    ) as HTMLButtonElement
    await act(async () => {
      browse.click()
    })
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0))
    })
    expect(field().value).toBe('~/')
    expect(mocks.listed).toEqual(['/home/u'])
    expect(dialog().textContent).toContain('Recent')
  })

  it('starts at home when nothing is saved', async () => {
    mocks.saved = []
    await open()
    expect(field().value).toBe('~/')
    expect(dialog().textContent).not.toContain('Saved folders')
  })
})
