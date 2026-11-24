// @vitest-environment jsdom
import { act, useEffect } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { IiiClient } from '@/lib/iii-client'
import type { SessionMeta } from '@/lib/sessions/types'
import type { DecisionChatRequest } from '@/types/injectable-ui'
import {
  decisionChatCapabilities,
  registerDecisionChatWorkspace,
  useDecisionChatBinding,
} from './host'

const mocks = vi.hoisted(() => ({
  create: vi.fn(),
  find: vi.fn(),
}))
vi.mock('./side-chat', () => ({
  createDecisionSideChat: mocks.create,
  findDecisionSideChat: mocks.find,
}))

const copy = (id = 'copy'): SessionMeta => ({
  session_id: id,
  title: 'Decision copy',
  description: '',
  status: 'idle',
  created_at: 1,
  updated_at: 1,
  message_count: 1,
  metadata: { side_chat_question: 'source/question' },
})
const request = (
  overrides: Partial<DecisionChatRequest> = {},
): DecisionChatRequest => ({
  key: 'source/question',
  sessionId: 'source',
  kind: 'question',
  title: 'What should we do?',
  createdAt: '2026-11-22T00:00:00.000Z',
  questionId: 'question',
  questions: [
    {
      header: 'Choice',
      question: 'What should we do?',
      options: [{ label: 'A' }],
    },
  ],
  onUseAnswer: () => {},
  ...overrides,
})

afterEach(() => {
  mocks.create.mockReset()
  mocks.find.mockReset()
})

describe('decision chat host capability', () => {
  it('places the same copy id beside the caller for repeated opens', async () => {
    const open = vi.fn()
    const stop = registerDecisionChatWorkspace({
      caller: () => ({ tabId: 'tab', paneId: 'control-plane-pane' }),
      open,
      close: vi.fn(),
    })
    mocks.create.mockResolvedValue({ meta: copy() })
    const capabilities = decisionChatCapabilities(
      { call: vi.fn() as IiiClient['call'] },
      'control-plane',
    )

    await Promise.all([
      capabilities.openDecisionChat?.(request()),
      capabilities.openDecisionChat?.(request()),
    ])

    expect(mocks.create).toHaveBeenCalledTimes(2)
    expect(open).toHaveBeenCalledWith('copy', {
      tabId: 'tab',
      paneId: 'control-plane-pane',
    })
    stop()
  })

  it('rebinds an existing copy without forking, and returns null without one', async () => {
    const stop = registerDecisionChatWorkspace({
      caller: () => ({ tabId: 'tab', paneId: 'control-plane-pane' }),
      open: vi.fn(),
      close: vi.fn(),
    })
    const capabilities = decisionChatCapabilities({
      call: vi.fn(async () => ({
        value: { tabs: [{ id: 'control-plane', screens: ['chat:copy'] }] },
      })) as IiiClient['call'],
    })
    mocks.find.mockResolvedValueOnce(null).mockResolvedValueOnce(copy())

    await expect(
      capabilities.rebindDecisionChat?.(request()),
    ).resolves.toBeNull()
    await expect(
      capabilities.rebindDecisionChat?.(request()),
    ).resolves.toBeTruthy()
    expect(mocks.create).not.toHaveBeenCalled()
    stop()
  })

  it('does not rebind a copy without a workspace column', async () => {
    const open = vi.fn()
    const stop = registerDecisionChatWorkspace({
      caller: () => ({ tabId: 'tab', paneId: 'control-plane-pane' }),
      open,
      close: vi.fn(),
    })
    mocks.find.mockResolvedValue(copy())
    const capabilities = decisionChatCapabilities({
      call: vi.fn(async () => ({
        value: {
          tabs: [{ id: 'control-plane', screens: ['ext:control-plane'] }],
        },
      })) as IiiClient['call'],
    })
    const rebound = await capabilities.rebindDecisionChat?.(request())

    expect(rebound).toBeNull()
    expect(open).not.toHaveBeenCalled()
    expect(mocks.create).not.toHaveBeenCalled()
    stop()
  })

  it('closes through the workspace bridge, while a detached handle stays inert', async () => {
    const close = vi.fn()
    const stop = registerDecisionChatWorkspace({
      caller: () => ({ tabId: 'tab', paneId: 'control-plane-pane' }),
      open: vi.fn(),
      close,
    })
    mocks.create.mockResolvedValue({ meta: copy() })
    const handle = await decisionChatCapabilities({
      call: vi.fn() as IiiClient['call'],
    }).openDecisionChat?.(request())

    handle?.close()
    expect(close).toHaveBeenCalledWith('copy')
    handle?.detach()
    handle?.close()
    expect(close).toHaveBeenCalledTimes(1)
    stop()
  })

  it('keeps the newer rebind when an older lookup resolves late', async () => {
    const root = createRoot(document.createElement('div'))
    const seen = vi.fn()
    const Probe = () => {
      const binding = useDecisionChatBinding('newest')
      useEffect(() => {
        seen(binding?.request.agent)
      }, [binding])
      return null
    }
    await act(() => root.render(<Probe />))
    const stop = registerDecisionChatWorkspace({
      caller: () => ({ tabId: 'tab', paneId: 'control-plane-pane' }),
      open: vi.fn(),
      close: vi.fn(),
    })
    let resolveOld: (value: SessionMeta | null) => void = () => {}
    const old = new Promise<SessionMeta | null>((resolve) => {
      resolveOld = resolve
    })
    mocks.find
      .mockImplementationOnce(() => old)
      .mockResolvedValueOnce(copy('newest'))
    const capabilities = decisionChatCapabilities({
      call: vi.fn(async () => ({
        value: { tabs: [{ id: 'control-plane', screens: ['chat:newest'] }] },
      })) as IiiClient['call'],
    })
    const older = capabilities.rebindDecisionChat({
      ...request(),
      key: 'source/newest',
      agent: 'old',
    })
    const newer = await capabilities.rebindDecisionChat({
      ...request(),
      key: 'source/newest',
      agent: 'new',
    })
    await act(() => Promise.resolve())
    resolveOld(copy('newest'))
    const stale = await older
    await act(() => Promise.resolve())
    stale?.detach()

    expect(newer).toBeTruthy()
    expect(seen).toHaveBeenLastCalledWith('new')
    stop()
    await act(() => root.unmount())
  })

  it('keeps an opening copy bound when its column is not persisted yet', async () => {
    const root = createRoot(document.createElement('div'))
    const seen = vi.fn()
    const Probe = () => {
      const binding = useDecisionChatBinding('copy')
      useEffect(() => {
        seen(binding?.request.agent)
      }, [binding])
      return null
    }
    await act(() => root.render(<Probe />))
    const open = vi.fn()
    const stop = registerDecisionChatWorkspace({
      caller: () => ({ tabId: 'tab', paneId: 'control-plane-pane' }),
      open,
      close: vi.fn(),
    })
    let resolve: (value: { meta: SessionMeta }) => void = () => {}
    mocks.create.mockReturnValue(
      new Promise<{ meta: SessionMeta }>((done) => {
        resolve = done
      }),
    )
    const capabilities = decisionChatCapabilities({
      call: vi.fn(async () => ({
        value: {
          tabs: [{ id: 'control-plane', screens: ['ext:control-plane'] }],
        },
      })) as IiiClient['call'],
    })
    const opening = capabilities.openDecisionChat({
      ...request(),
      agent: 'open',
    })
    const rebound = capabilities.rebindDecisionChat({
      ...request(),
      agent: 'page after reload',
    })
    resolve({ meta: copy() })

    await expect(opening).resolves.toBeTruthy()
    await expect(rebound).resolves.toBeNull()
    await act(() => Promise.resolve())
    expect(mocks.find).not.toHaveBeenCalled()
    expect(mocks.create).toHaveBeenCalledOnce()
    expect(open).toHaveBeenCalledOnce()
    expect(seen).toHaveBeenLastCalledWith('open')
    stop()
    await act(() => root.unmount())
  })
})
