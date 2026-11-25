import { beforeEach, describe, expect, it, vi } from 'vitest'
import { getIiiClient } from '@/lib/iii-client'
import {
  assignProject,
  type ChatProject,
  createProject,
  describeProjectError,
  getChatSettings,
  listProjects,
  NO_PROJECT,
  parseProjectError,
  projectOf,
  setChatSettings,
  tildePath,
  updateProject,
} from './projects'

vi.mock('@/lib/iii-client', () => ({ getIiiClient: vi.fn() }))

function project(id: string, folders: string[]): ChatProject {
  return {
    id,
    name: id,
    color: 'blue',
    folders,
    created_at_ms: 0,
    updated_at_ms: 0,
  }
}

describe('projectOf (the spec §1.2.2 table)', () => {
  const web = project('web', ['/home/u/work/acme/web'])
  const acme = project('acme', ['/home/u/work/acme'])
  const api = project('api', ['/home/u/work/acme/api', '/srv/api/'])
  const all = [acme, web, api]

  it.each([
    ['equal path', '/home/u/work/acme/web', 'web'],
    ['equal path with a trailing slash', '/home/u/work/acme/web/', 'web'],
    ['prefix followed by /', '/home/u/work/acme/web/src/app', 'web'],
    ['prefix without / is not inside', '/home/u/work/acme/webapp', 'acme'],
    [
      'nested folders: the longest prefix wins',
      '/home/u/work/acme/api/x',
      'api',
    ],
    ['the outer project keeps the rest', '/home/u/work/acme/infra', 'acme'],
    ['a stored trailing slash still matches', '/srv/api/v2', 'api'],
    ['no match', '/home/u/scratch', null],
    ['no match on a sibling prefix', '/home/u/work/acm', null],
  ])('%s', (_, path, expected) => {
    expect(projectOf(path, all)?.id ?? null).toBe(expected)
  })

  it('answers null for an empty path or no projects', () => {
    expect(projectOf(null, all)).toBeNull()
    expect(projectOf('', all)).toBeNull()
    expect(projectOf('/home/u/work/acme', [])).toBeNull()
  })
})

describe('tildePath', () => {
  it('shows the home folder as ~', () => {
    expect(tildePath('/home/u/work/acme/api')).toBe('~/work/acme/api')
    expect(tildePath('/Users/me')).toBe('~')
    expect(tildePath('/srv/data')).toBe('/srv/data')
  })
})

describe('project calls', () => {
  const call = vi.fn()
  beforeEach(() => {
    call.mockReset()
    vi.mocked(getIiiClient).mockResolvedValue({ call } as never)
  })

  it('lists with version 2 and parses defensively', async () => {
    call.mockResolvedValue({
      revision: 7,
      projects: [
        {
          id: 'prj_1',
          name: 'acme-web',
          color: 'teal',
          folders: ['/a', 3],
          account: { mode: 'global' },
        },
        { name: 'no id' },
      ],
    })
    const list = await listProjects()
    expect(call).toHaveBeenCalledWith('harness::projects::list', {
      version: 2,
    })
    expect(list.revision).toBe(7)
    expect(list.projects).toHaveLength(1)
    expect(list.projects[0]).toMatchObject({
      id: 'prj_1',
      color: 'teal',
      folders: ['/a'],
      account: { mode: 'global' },
    })
  })

  it('creates with bring_existing and returns the moved count', async () => {
    call.mockResolvedValue({
      project: { id: 'prj_2', name: 'x', color: 'rose', folders: ['/x'] },
      moved: 4,
    })
    const res = await createProject({
      name: 'x',
      color: 'rose',
      folders: ['/x'],
      bring_existing: true,
    })
    expect(call).toHaveBeenCalledWith('harness::projects::create', {
      name: 'x',
      color: 'rose',
      folders: ['/x'],
      bring_existing: true,
    })
    expect(res.moved).toBe(4)
  })

  it('updates as a patch and never sends account or autocompact', async () => {
    call.mockResolvedValue({
      project: { id: 'prj_2', name: 'y', color: 'rose', folders: ['/x'] },
      moved: 0,
    })
    await updateProject({ id: 'prj_2', name: 'y' })
    expect(call).toHaveBeenCalledWith('harness::projects::update', {
      id: 'prj_2',
      name: 'y',
    })
  })

  it('assigns sessions, with "none" for No project', async () => {
    call.mockResolvedValue({ updated: 2 })
    await expect(assignProject(['s1'], NO_PROJECT)).resolves.toEqual({
      updated: 2,
    })
    expect(call).toHaveBeenCalledWith('harness::projects::assign', {
      session_ids: ['s1'],
      project: 'none',
    })
  })

  it('reads and patches chat settings', async () => {
    call.mockResolvedValueOnce({ descriptions: { agent_writes: false } })
    await expect(getChatSettings()).resolves.toMatchObject({
      descriptions: { agent_writes: false },
    })
    call.mockResolvedValueOnce({
      settings: { descriptions: { agent_writes: true } },
    })
    await expect(
      setChatSettings({ descriptions: { agent_writes: true } }),
    ).resolves.toMatchObject({ descriptions: { agent_writes: true } })
    expect(call).toHaveBeenLastCalledWith('harness::chat-settings::set', {
      patch: { descriptions: { agent_writes: true } },
    })
  })
})

describe('project errors', () => {
  const wire = (body: Record<string, unknown>) => ({
    code: 'invocation_failed',
    message: `handler error: ${JSON.stringify(body)}`,
  })

  it('reads the JSON error out of a handler error', () => {
    expect(
      parseProjectError(
        wire({
          code: 'folder_taken',
          message: 'taken',
          project_id: 'prj_9',
          project_name: 'billing-api',
          folder: '/home/u/work/acme/api',
        }),
      ),
    ).toMatchObject({
      code: 'folder_taken',
      project_id: 'prj_9',
      project_name: 'billing-api',
      folder: '/home/u/work/acme/api',
    })
  })

  it.each([
    ['bad_name', 'name', 'Names have 1 to 40 characters'],
    ['no_folders', 'folders', 'Add at least one folder'],
    ['too_many_folders', 'folders', 'At most 20 folders'],
    ['folder_not_absolute', 'folders', 'x is not an absolute path'],
    ['bad_color', 'form', 'Pick one of blue, purple'],
    ['not_found', 'form', 'No such project'],
    ['bad_request', 'form', 'Bad request'],
  ])(
    'puts the engine code %s under %s with its message (D33)',
    (code, field, message) => {
      expect(
        describeProjectError(wire({ code, message }), {
          name: 'n',
          folders: [],
        }),
      ).toEqual({
        field,
        message: code === 'no_folders' ? 'Add at least one folder' : message,
      })
    },
  )

  it('maps codes to the dialog copy', () => {
    const context = { name: ' acme-web ', folders: ['/home/u/work/acme/api'] }
    expect(
      describeProjectError(
        wire({
          code: 'name_taken',
          message: 'A project named “acme-web” already exists',
          project_id: 'prj_acme',
          project_name: 'acme-web',
        }),
        { ...context, name: 'ACME-WEB' },
      ),
    ).toEqual({
      field: 'name',
      message: 'A project named “acme-web” already exists',
    })
    // Only the code and the project's name survived: the name it has.
    expect(
      describeProjectError(
        { code: 'name_taken', project_name: 'acme-web' },
        { ...context, name: 'ACME-WEB' },
      ),
    ).toEqual({
      field: 'name',
      message: 'A project named “acme-web” already exists',
    })
    expect(
      describeProjectError(
        wire({
          code: 'folder_taken',
          message: 'x',
          project_id: 'p',
          project_name: 'billing-api',
        }),
        context,
      ),
    ).toEqual({
      field: 'folders',
      message: '~/work/acme/api is already in “billing-api”',
    })
    expect(describeProjectError(new Error('engine down'), context)).toEqual({
      field: 'form',
      message: 'engine down',
    })
  })
})
