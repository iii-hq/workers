import type { ReactElement } from 'react'
import { describe, expect, it } from 'vitest'
import type { CheckpointGroup } from '@/lib/backend/coder-checkpoints'
import { GroupRow, renderBody } from './CheckpointsDialog'

const group = (over: Partial<CheckpointGroup>): CheckpointGroup => ({
  key: 't1',
  turnId: 't1',
  ts: 1000,
  functionIds: ['coder::update-file'],
  files: ['/w/src/a.rs'],
  isRevert: false,
  records: [],
  ...over,
})

/** GroupRow renders <section><div>[meta, button]</div></section>. */
function buttonOf(el: ReactElement): {
  disabled?: boolean
  title?: string
  children: unknown
} {
  const inner = (el.props as { children: ReactElement }).children
  const kids = (inner.props as { children: ReactElement[] }).children
  return kids[1].props as {
    disabled?: boolean
    title?: string
    children: unknown
  }
}

describe('GroupRow', () => {
  it('labels a normal group "undo"', () => {
    const el = GroupRow({
      group: group({}),
      canUndo: true,
      busy: false,
      inFlight: false,
      onUndo: () => {},
    })
    expect(buttonOf(el).children).toBe('undo')
  })

  it('labels a revert group "redo"', () => {
    const el = GroupRow({
      group: group({ isRevert: true }),
      canUndo: true,
      busy: false,
      inFlight: false,
      onUndo: () => {},
    })
    expect(buttonOf(el).children).toBe('redo')
  })

  it('disables with a hint when the group is not undoable', () => {
    const el = GroupRow({
      group: group({ turnId: undefined, key: 'seq-4' }),
      canUndo: false,
      busy: false,
      inFlight: false,
      onUndo: () => {},
    })
    const btn = buttonOf(el)
    expect(btn.disabled).toBe(true)
    expect(btn.title).toMatch(/turn attribution/)
  })
})

describe('renderBody', () => {
  const handlers = {
    undoingKey: null,
    onUndo: () => {},
    onRetry: () => {},
  }

  it('prompts for a working directory when none is set', () => {
    const el = renderBody({ status: 'idle' }, { ...handlers, workingDir: null })
    expect((el.props as { text: string }).text).toBe(
      'set a working directory first.',
    )
  })

  it('shows the empty state when there are no checkpoints', () => {
    const el = renderBody(
      { status: 'ready', groups: [], truncated: false },
      { ...handlers, workingDir: '/w' },
    )
    expect((el.props as { text: string }).text).toBe('no checkpoints yet.')
  })
})
