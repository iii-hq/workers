import { describe, expect, it } from 'vitest'
import type { ChatProject } from '@/lib/backend/projects'
import { resolutionText } from '@/lib/claude-accounts-words'
import { chatAccountResolution, lockedAccountOf } from './conversation-account'

const NAMES: Record<string, string> = { default: 'Work', personal: 'Personal' }
const nameOf = (id: string) => NAMES[id] ?? id
const projects: Pick<ChatProject, 'id' | 'name' | 'account'>[] = [
  {
    id: 'prj_billing',
    name: 'billing-api',
    account: { mode: 'pinned', account_id: 'personal' },
  },
]
const base = { projects, global: 'default' }
const lockedSide = {
  sessionMetadata: {
    side_chat_of: 'root',
    side_chat_lock: {
      model: 'claude-code::claude-opus-4-5',
      account: 'personal',
    },
  },
}

describe("a conversation's account, as the engine runs it", () => {
  it('reads a side chat on its lock, while the global is Work', () => {
    const r = chatAccountResolution({ ...base, conversation: lockedSide })
    expect(r).toEqual({
      account: 'personal',
      source: 'session',
      locked: 'side-chat',
    })
    expect(resolutionText(r, nameOf)).toBe(
      "Runs on Personal, locked to its conversation's account.",
    )
    // A side chat shown as one but locked before phase 4: its own resolution.
    expect(
      chatAccountResolution({
        ...base,
        conversation: { sessionMetadata: {} },
        sideChat: true,
      }),
    ).toEqual({ account: 'default', source: 'global' })
  })

  it("reads a sub-agent on its parent's resolution", () => {
    const parent = { id: 'p1', projectId: 'prj_billing', sessionMetadata: {} }
    const r = chatAccountResolution({
      ...base,
      conversation: { parentId: 'p1', sessionMetadata: {} },
      conversations: [parent],
    })
    expect(r).toMatchObject({
      account: 'personal',
      source: 'project',
      locked: 'sub-agent',
    })
    expect(resolutionText(r, nameOf)).toBe(
      "Runs on Personal, its conversation's account.",
    )
    // A sub-agent of a sub-agent of a side chat: the side chat's lock.
    const nested = chatAccountResolution({
      ...base,
      conversation: { parentId: 'p2', sessionMetadata: {} },
      conversations: [
        { id: 'p2', parentId: 's1', sessionMetadata: {} },
        { id: 's1', ...lockedSide },
      ],
    })
    expect(nested).toMatchObject({ account: 'personal', locked: 'sub-agent' })
  })

  it("reads a sub-agent whose parent isn't loaded from the parent turn, else its own", () => {
    const child = {
      parentId: 'p1',
      sessionMetadata: { claude_account: 'default' },
    }
    expect(
      chatAccountResolution({
        ...base,
        conversation: child,
        parentTurnAccount: { id: 'personal', source: 'global' },
      }),
    ).toEqual({ account: 'personal', source: 'global', locked: 'sub-agent' })
    expect(chatAccountResolution({ ...base, conversation: child })).toEqual({
      account: 'default',
      source: 'session',
    })
  })

  it('tells a side chat from a sub-agent', () => {
    expect(lockedAccountOf(lockedSide)).toBe('side-chat')
    expect(lockedAccountOf({ sessionMetadata: {} }, true)).toBe('side-chat')
    expect(lockedAccountOf({ parentId: 'p1' })).toBe('sub-agent')
    expect(lockedAccountOf({})).toBeNull()
  })
})
