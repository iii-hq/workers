import { describe, expect, it, vi } from 'vitest'
import type { IiiClient } from '@/lib/iii-client'
import type {
  PendingApprovalRecord,
  PendingResolvedEvent,
} from '@/types/iii-agent-event'
import {
  approvalBelongsToConversationTree,
  listPendingApprovals,
  startApprovalEventsSubscription,
} from './approval-events-live'

function fakeClient(listResult?: unknown) {
  const triggers: Array<{
    type: string
    function_id: string
    config?: unknown
  }> = []
  const handlers = new Map<string, (p: unknown) => void>()
  const offHandler = vi.fn()
  const triggerUnregister = vi.fn()

  const on = vi.fn((fn: string, h: (p: unknown) => void) => {
    handlers.set(fn, h)
    return offHandler
  })
  const registerTrigger = vi.fn(
    (input: { type: string; function_id: string; config?: unknown }) => {
      triggers.push(input)
      return triggerUnregister
    },
  )
  const trigger = vi.fn(async () => listResult)

  const client = {
    browserId: 'console-test',
    on,
    registerTrigger,
    trigger,
    addConnectionStateListener: vi.fn(),
    dispose: vi.fn(async () => {}),
  } as unknown as IiiClient

  return {
    client,
    triggers,
    trigger,
    offHandler,
    triggerUnregister,
    fire: (fn: string, payload: unknown) => handlers.get(fn)?.(payload),
  }
}

const record: PendingApprovalRecord = {
  function_call_id: 'fc-1',
  function_id: 'shell::fs::write',
  session_id: 'sess-1',
  turn_id: 't-1',
  pending_at: 0,
  expires_at: 1000,
  status: 'pending',
}

describe('startApprovalEventsSubscription', () => {
  it('binds both pending triggers scoped to the session', () => {
    const { client, triggers } = fakeClient()
    startApprovalEventsSubscription(client, 'sess-1', {
      onCreated: () => {},
      onResolved: () => {},
    })
    expect(triggers).toEqual([
      {
        type: 'approval::pending-created',
        function_id: 'iii::console::approval_pending_created::console-test',
        config: { session_id: 'sess-1' },
      },
      {
        type: 'approval::pending-resolved',
        function_id: 'iii::console::approval_pending_resolved::console-test',
        config: { session_id: 'sess-1' },
      },
    ])
  })

  it('routes created/resolved payloads to their handlers', () => {
    const { client, fire } = fakeClient()
    const onCreated = vi.fn()
    const onResolved = vi.fn()
    startApprovalEventsSubscription(client, 'sess-1', { onCreated, onResolved })

    fire('iii::console::approval_pending_created', record)
    expect(onCreated).toHaveBeenCalledWith(record)

    const resolved: PendingResolvedEvent = {
      function_call_id: 'fc-1',
      function_id: 'shell::fs::write',
      session_id: 'sess-1',
      turn_id: 't-1',
      outcome: 'deny',
      resolved_at: 9,
    }
    fire('iii::console::approval_pending_resolved', resolved)
    expect(onResolved).toHaveBeenCalledWith(resolved)
  })

  it('unregisters both handlers and triggers on cleanup', () => {
    const { client, offHandler, triggerUnregister } = fakeClient()
    const stop = startApprovalEventsSubscription(client, 'sess-1', {
      onCreated: () => {},
      onResolved: () => {},
    })
    stop()
    expect(offHandler).toHaveBeenCalledTimes(2)
    expect(triggerUnregister).toHaveBeenCalledTimes(2)
  })

  it('can bind unscoped events for a parent chat that filters descendants client-side', () => {
    const { client, triggers, fire } = fakeClient()
    const onCreated = vi.fn()
    const onResolved = vi.fn()
    startApprovalEventsSubscription(client, undefined, {
      onCreated,
      onResolved,
    })
    expect(triggers.map((trigger) => trigger.config)).toEqual([{}, {}])

    fire('iii::console::approval_pending_created', record)
    expect(onCreated).toHaveBeenCalledWith(record)

    const resolved: PendingResolvedEvent = {
      function_call_id: 'fc-1',
      function_id: 'shell::fs::write',
      session_id: 'sess-1',
      turn_id: 't-1',
      outcome: 'allow',
      resolved_at: 9,
    }
    fire('iii::console::approval_pending_resolved', resolved)
    expect(onResolved).toHaveBeenCalledWith(resolved)
  })
})

describe('listPendingApprovals', () => {
  it('returns the pending array from the response', async () => {
    const { client, trigger } = fakeClient({ pending: [record] })
    expect(await listPendingApprovals(client, 'sess-1')).toEqual([record])
    expect(trigger).toHaveBeenCalledWith('approval::list-pending', {
      session_id: 'sess-1',
    })
  })

  it('returns [] when the response carries no pending list', async () => {
    const { client } = fakeClient({})
    expect(await listPendingApprovals(client, 'sess-1')).toEqual([])
  })

  it('reads the full pending inbox when descendant filtering is client-side', async () => {
    const { client, trigger } = fakeClient({ pending: [record] })
    expect(await listPendingApprovals(client)).toEqual([record])
    expect(trigger).toHaveBeenCalledWith('approval::list-pending', {
      limit: 500,
    })
  })
})

describe('approvalBelongsToConversationTree', () => {
  const conversations = [
    { id: 'root' },
    { id: 'child', parentId: 'root' },
    { id: 'grandchild', parentId: 'child' },
    { id: 'other' },
  ]

  it('includes the root, direct children during creation, and known deeper descendants', () => {
    expect(
      approvalBelongsToConversationTree(
        { session_id: 'root', session_metadata: null },
        'root',
        conversations,
      ),
    ).toBe(true)
    expect(
      approvalBelongsToConversationTree(
        {
          session_id: 'new-child',
          session_metadata: { parent_session_id: 'root' },
        },
        'root',
        conversations,
      ),
    ).toBe(true)
    expect(
      approvalBelongsToConversationTree(
        { session_id: 'grandchild', session_metadata: null },
        'root',
        conversations,
      ),
    ).toBe(true)
  })

  it('excludes unrelated sessions', () => {
    expect(
      approvalBelongsToConversationTree(
        { session_id: 'other', session_metadata: null },
        'root',
        conversations,
      ),
    ).toBe(false)
  })
})
