import { beforeEach, describe, expect, it, vi } from 'vitest'
import { getIiiClient } from '@/lib/iii-client'
import type { Conversation } from '@/types/chat'
import { getSession } from './api'
import { getRemovalPreview } from './removal-preview'
import type { SessionMeta } from './types'

vi.mock('@/lib/iii-client', () => ({ getIiiClient: vi.fn() }))
vi.mock('./api', () => ({ getSession: vi.fn() }))
const trigger = vi.fn()
const conversation = (patch: Partial<Conversation> = {}): Conversation => ({
  id: 'target',
  title: 'Target',
  model: null,
  messages: [],
  createdAt: 1,
  updatedAt: 1,
  ...patch,
})
const meta = (
  id = 'target',
  patch: Partial<SessionMeta> = {},
): SessionMeta => ({
  session_id: id,
  title: id,
  description: '',
  status: 'done',
  message_count: 0,
  created_at: 1,
  updated_at: 1,
  ...patch,
})
const tree = (ids = ['target']) => ({
  root_session_id: 'target',
  complete: true,
  sessions: ids.map((session_id) => ({ session_id })),
})

beforeEach(() => {
  vi.resetAllMocks()
  vi.mocked(getIiiClient).mockResolvedValue({ trigger } as never)
  trigger.mockResolvedValue(tree())
  vi.mocked(getSession).mockImplementation(async (id) => meta(id))
})

describe('getRemovalPreview', () => {
  it('skips network for a local empty draft', async () => {
    expect(
      await getRemovalPreview(conversation({ draft: true })),
    ).toMatchObject({
      empty: true,
      hasRunningWork: false,
      hasChildren: false,
    })
    expect(getIiiClient).not.toHaveBeenCalled()
  })

  it('checks a persisted empty leaf without requiring transcript hydration', async () => {
    expect(
      await getRemovalPreview(conversation({ hydrated: false })),
    ).toMatchObject({
      empty: true,
      hasRunningWork: false,
      hasChildren: false,
    })
    expect(trigger).toHaveBeenCalledWith(
      'harness::session-tree',
      { root_session_id: 'target' },
      { timeoutMs: 10_000 },
    )
    expect(getSession).toHaveBeenCalledTimes(1)
  })

  it('does not confuse an unloaded nonempty transcript with an empty chat', async () => {
    vi.mocked(getSession).mockResolvedValue(
      meta('target', { message_count: 12 }),
    )
    expect(
      (await getRemovalPreview(conversation({ hydrated: false }))).empty,
    ).toBe(false)
  })

  it.each([
    { draftText: 'Unsent message' },
    { started: true },
    { messages: [{ id: 'm1', role: 'user', content: 'Hello', timestamp: 1 }] },
    {
      draftAttachments: [
        { id: 'a1', name: 'image.png', mime: 'image/png', size: 1 },
      ],
    },
  ] as Partial<Conversation>[])('preserves local content %j', async (patch) => {
    expect(
      (await getRemovalPreview(conversation({ draft: true, ...patch }))).empty,
    ).toBe(false)
    expect((await getRemovalPreview(conversation(patch))).empty).toBe(false)
  })

  it('preserves a saved unsent draft', async () => {
    vi.mocked(getSession).mockResolvedValue(
      meta('target', { draft: 'not yet sent' }),
    )
    expect((await getRemovalPreview(conversation())).empty).toBe(false)
  })

  it('uses the full tree and sees a running grandchild outside the sidebar', async () => {
    trigger.mockResolvedValue(tree(['target', 'child', 'grandchild']))
    vi.mocked(getSession).mockImplementation(async (id) =>
      meta(id, { status: id === 'grandchild' ? 'working' : 'done' }),
    )
    expect(
      await getRemovalPreview(conversation({ status: 'done' })),
    ).toMatchObject({
      empty: false,
      hasRunningWork: true,
      hasChildren: true,
    })
    expect(getSession).toHaveBeenCalledTimes(3)
  })

  it('requires confirmation for children even when the root is empty', async () => {
    trigger.mockResolvedValue(tree(['target', 'child']))
    expect(await getRemovalPreview(conversation())).toMatchObject({
      empty: false,
      hasRunningWork: false,
      hasChildren: true,
    })
  })

  it('reports running work on a leaf and refreshes stale local status', async () => {
    vi.mocked(getSession).mockResolvedValueOnce(
      meta('target', { status: 'working' }),
    )
    expect(await getRemovalPreview(conversation())).toMatchObject({
      empty: false,
      hasRunningWork: true,
    })
    expect(
      await getRemovalPreview(conversation({ status: 'working' })),
    ).toMatchObject({ hasRunningWork: false })
  })

  it('does not climb to a running parent or include siblings', async () => {
    vi.mocked(getSession).mockResolvedValue(
      meta('target', {
        metadata: { parent_session_id: 'outside-parent' },
        message_count: 3,
      }),
    )
    expect(await getRemovalPreview(conversation())).toMatchObject({
      parentId: 'outside-parent',
      hasRunningWork: false,
      hasChildren: false,
    })
    expect(getSession).toHaveBeenCalledExactlyOnceWith('target')
  })

  it.each([
    { ...tree(), complete: false },
    { ...tree(), root_session_id: 'wrong' },
    tree(['other']),
    null,
  ])('fails closed for an incomplete or invalid tree %j', async (response) => {
    trigger.mockResolvedValue(response)
    await expect(getRemovalPreview(conversation())).rejects.toThrow(
      'Could not verify',
    )
  })

  it.each([
    null,
    { ...meta(), message_count: undefined },
    { ...meta(), status: 'unknown' },
  ])(
    'does not assume empty on missing or invalid metadata',
    async (response) => {
      vi.mocked(getSession).mockResolvedValue(response as SessionMeta | null)
      await expect(getRemovalPreview(conversation())).rejects.toThrow(
        'could not be verified',
      )
    },
  )

  it('propagates transport failure without falling back to the sidebar', async () => {
    trigger.mockRejectedValue(new Error('disconnected'))
    await expect(
      getRemovalPreview(conversation({ started: false })),
    ).rejects.toThrow('disconnected')
  })

  it('checks a working grandchild in the second metadata batch', async () => {
    const ids = [
      'target',
      ...Array.from({ length: 8 }, (_, index) => `child-${index}`),
    ]
    trigger.mockResolvedValue(tree(ids))
    vi.mocked(getSession).mockImplementation(async (id) =>
      meta(id, { status: id === 'child-7' ? 'working' : 'done' }),
    )

    await expect(getRemovalPreview(conversation())).resolves.toMatchObject({
      hasChildren: true,
      hasRunningWork: true,
      empty: false,
    })
    expect(getSession).toHaveBeenCalledTimes(9)
  })

  it('rejects a null metadata result in the second batch', async () => {
    const ids = [
      'target',
      ...Array.from({ length: 8 }, (_, index) => `child-${index}`),
    ]
    trigger.mockResolvedValue(tree(ids))
    vi.mocked(getSession).mockImplementation(async (id) =>
      id === 'child-7' ? null : meta(id),
    )

    await expect(getRemovalPreview(conversation())).rejects.toThrow(
      'could not be verified',
    )
  })
})
