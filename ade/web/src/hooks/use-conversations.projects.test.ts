import { describe, expect, it } from 'vitest'
import {
  CONSOLE_METADATA_KEYS,
  consoleMetadataPatch,
  conversationProjectFields,
} from './use-conversations'

describe('metadata writes through session::patch-meta (D9)', () => {
  it('sends only console-owned keys, set or unset, and drops the legacy mode', () => {
    const patch = consoleMetadataPatch({
      surface: 'console',
      model: 'm',
      description_history: [{ text: 'agent wrote this' }],
      project_id: 'prj_1',
    })
    expect(patch.set).toEqual({ surface: 'console', model: 'm' })
    // A stale copy of another writer's keys is never sent back.
    expect(patch.set).not.toHaveProperty('description_history')
    expect(patch.set).not.toHaveProperty('project_id')
    expect(patch.unset).toContain('mode')
    expect(patch.unset).toContain('title_manual')
    for (const key of patch.unset) {
      expect(patch.set).not.toHaveProperty(key)
    }
    expect([...Object.keys(patch.set), ...patch.unset].sort()).toEqual(
      [...CONSOLE_METADATA_KEYS, 'mode'].sort(),
    )
  })

  it('adds the keys a write named (a worker control), set or unset', () => {
    const patch = consoleMetadataPatch({ side_chat_quotes: [1] }, [
      'side_chat_quotes',
      'auto_fix',
    ])
    expect(patch.set.side_chat_quotes).toEqual([1])
    expect(patch.unset).toContain('auto_fix')
  })
})

describe('chat-projects keys on a conversation (§0.4)', () => {
  it('reads the description, its author and time, the project and title_by', () => {
    expect(
      conversationProjectFields('  Backoff with jitter ', {
        description_by: 'agent',
        description_at_ms: 42,
        project_id: 'prj_1',
        title_by: 'agent',
      }),
    ).toEqual({
      description: 'Backoff with jitter',
      descriptionBy: 'agent',
      descriptionAtMs: 42,
      projectId: 'prj_1',
      titleBy: 'agent',
    })
  })

  it('leaves absent and malformed values out', () => {
    expect(
      conversationProjectFields('', { description_by: 'robot', project_id: 3 }),
    ).toEqual({
      description: undefined,
      descriptionBy: undefined,
      descriptionAtMs: undefined,
      projectId: undefined,
      titleBy: undefined,
    })
    expect(
      conversationProjectFields(undefined, { project_id: 'none' }).projectId,
    ).toBe('none')
  })
})
