import { describe, expect, it, vi } from 'vitest'
import {
  CHECKPOINTS_FUNCTION_ID,
  type CheckpointRecord,
  groupCheckpoints,
  listCheckpoints,
  UNDO_FUNCTION_ID,
  undoCheckpoint,
} from './coder-checkpoints'

const rec = (over: Partial<CheckpointRecord>): CheckpointRecord => ({
  seq: 1,
  ts: 1000,
  functionId: 'coder::update-file',
  files: [],
  ...over,
})

describe('listCheckpoints', () => {
  it('sends the full fs_scope shape and coerces snake_case records', async () => {
    const trigger = vi.fn().mockResolvedValue({
      records: [
        {
          seq: 7,
          ts: 42,
          session_id: 's1',
          turn_id: 't1',
          function_id: 'coder::update-file',
          files: ['/w/a.rs', 3],
        },
        { seq: 'bad' }, // dropped: no numeric seq / function_id
      ],
      truncated: true,
    })
    const out = await listCheckpoints('/w', { limit: 50, trigger })
    expect(trigger).toHaveBeenCalledWith(CHECKPOINTS_FUNCTION_ID, {
      fs_scope: { root: '/w', grants: [] },
      limit: 50,
    })
    expect(out.truncated).toBe(true)
    expect(out.records).toEqual([
      {
        seq: 7,
        ts: 42,
        sessionId: 's1',
        turnId: 't1',
        functionId: 'coder::update-file',
        files: ['/w/a.rs'],
      },
    ])
  })

  it('tolerates a missing records array', async () => {
    const trigger = vi.fn().mockResolvedValue({})
    const out = await listCheckpoints('/w', { trigger })
    expect(out).toEqual({ records: [], truncated: false })
  })
})

describe('undoCheckpoint', () => {
  it('reverses a turn by turn_id (no steps)', async () => {
    const trigger = vi.fn().mockResolvedValue({
      undone: [
        {
          seq: 3,
          function_id: 'coder::update-file',
          restored: ['/w/a.rs'],
          removed: [],
          skipped: [],
        },
      ],
    })
    const out = await undoCheckpoint('/w', { turnId: 't1', trigger })
    expect(trigger).toHaveBeenCalledWith(UNDO_FUNCTION_ID, {
      turn_id: 't1',
      fs_scope: { root: '/w', grants: [] },
    })
    expect(out).toHaveLength(1)
    expect(out[0].restored).toEqual(['/w/a.rs'])
  })

  it('reverses by step count when no turn_id (no turn_id key sent)', async () => {
    const trigger = vi.fn().mockResolvedValue({ undone: [] })
    await undoCheckpoint('/w', { steps: 1, trigger })
    expect(trigger).toHaveBeenCalledWith(UNDO_FUNCTION_ID, {
      steps: 1,
      fs_scope: { root: '/w', grants: [] },
    })
  })
})

describe('groupCheckpoints', () => {
  it('merges contiguous records of the same turn and unions their files', () => {
    const groups = groupCheckpoints([
      rec({
        seq: 9,
        turnId: 't2',
        functionId: 'coder::update-file',
        files: ['/w/b.rs'],
      }),
      rec({
        seq: 8,
        turnId: 't2',
        functionId: 'coder::create-file',
        files: ['/w/c.rs'],
      }),
      rec({
        seq: 7,
        turnId: 't1',
        functionId: 'coder::update-file',
        files: ['/w/a.rs'],
      }),
    ])
    expect(groups).toHaveLength(2)
    expect(groups[0].turnId).toBe('t2')
    expect(groups[0].functionIds).toEqual([
      'coder::update-file',
      'coder::create-file',
    ])
    expect(groups[0].files).toEqual(['/w/b.rs', '/w/c.rs'])
    expect(groups[1].turnId).toBe('t1')
  })

  it('keeps turn-less records as their own group with a stable key', () => {
    const groups = groupCheckpoints([
      rec({ seq: 5, turnId: undefined }),
      rec({ seq: 4, turnId: undefined }),
    ])
    expect(groups).toHaveLength(2)
    expect(groups.map((g) => g.key)).toEqual(['seq-5', 'seq-4'])
  })

  it('flags a coder::undo record as a revert (redo target)', () => {
    const [group] = groupCheckpoints([
      rec({
        seq: 6,
        turnId: 't3',
        functionId: UNDO_FUNCTION_ID,
        files: ['/w/a.rs'],
      }),
    ])
    expect(group.isRevert).toBe(true)
  })
})
