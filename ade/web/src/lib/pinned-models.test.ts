import { describe, expect, it } from 'vitest'
import type { ModelOption } from '@/types/chat'
import {
  defaultPinnedModels,
  movePinnedModel,
  PINNED_MODELS_MAX,
  pinModel,
  resolvePinnedModels,
  unpinModel,
  withPinnedEffort,
} from './pinned-models'

const OPTIONS: ModelOption[] = [
  { id: 'anthropic::claude-opus-4-7', label: 'claude opus 4.7' },
  { id: 'anthropic::claude-sonnet-4-6', label: 'claude sonnet 4.6' },
  { id: 'openai::gpt-5', label: 'gpt-5' },
  { id: 'openai::gpt-5-mini', label: 'gpt-5 mini' },
  { id: 'openai::gpt-4.1', label: 'gpt-4.1' },
  { id: 'xai::grok-4', label: 'grok 4' },
  { id: 'claude-code::claude-opus-4-7', label: 'claude opus 4.7' },
]

describe('defaultPinnedModels', () => {
  it('leads with the selection, then its provider, then the rest, five at most', () => {
    const pinned = defaultPinnedModels(OPTIONS, 'openai::gpt-5-mini')
    expect(pinned.map((p) => p.id)).toEqual([
      'openai::gpt-5-mini',
      'openai::gpt-5',
      'openai::gpt-4.1',
      'anthropic::claude-opus-4-7',
      'anthropic::claude-sonnet-4-6',
    ])
    expect(pinned.every((p) => p.effort === 'default')).toBe(true)
  })

  it('takes the catalog order when nothing is selected', () => {
    expect(defaultPinnedModels(OPTIONS, null).map((p) => p.id)).toEqual(
      OPTIONS.slice(0, PINNED_MODELS_MAX).map((o) => o.id),
    )
  })
})

describe('resolvePinnedModels', () => {
  it('drops pins the catalog no longer offers', () => {
    const resolved = resolvePinnedModels(
      [
        { id: 'openai::gpt-5', effort: 'high' },
        { id: 'gone::model', effort: 'low' },
      ],
      OPTIONS,
      null,
    )
    expect(resolved).toEqual([{ id: 'openai::gpt-5', effort: 'high' }])
  })

  it('falls back to the defaults when nothing stored survives', () => {
    const resolved = resolvePinnedModels(
      [{ id: 'gone::model', effort: 'low' }],
      OPTIONS,
      'xai::grok-4',
    )
    expect(resolved[0]?.id).toBe('xai::grok-4')
  })
})

describe('pin, unpin, move, effort', () => {
  const list = [
    { id: 'a::1', effort: 'default' },
    { id: 'a::2', effort: 'high' },
    { id: 'b::1', effort: 'default' },
  ]

  it('pins at the end, or at an index, without doubling', () => {
    expect(
      pinModel(list, { id: 'c::1', effort: 'low' }).map((p) => p.id),
    ).toEqual(['a::1', 'a::2', 'b::1', 'c::1'])
    expect(
      pinModel(list, { id: 'c::1', effort: 'low' }, 0).map((p) => p.id),
    ).toEqual(['c::1', 'a::1', 'a::2', 'b::1'])
    expect(pinModel(list, { id: 'b::1', effort: 'medium' }, 0)).toEqual([
      { id: 'b::1', effort: 'medium' },
      { id: 'a::1', effort: 'default' },
      { id: 'a::2', effort: 'high' },
    ])
  })

  it('makes room at the end when full', () => {
    const full = ['1', '2', '3', '4', '5'].map((n) => ({
      id: `p::${n}`,
      effort: 'default',
    }))
    const next = pinModel(full, { id: 'p::6', effort: 'default' }, 0)
    expect(next).toHaveLength(PINNED_MODELS_MAX)
    expect(next.map((p) => p.id)).toEqual([
      'p::6',
      'p::1',
      'p::2',
      'p::3',
      'p::4',
    ])
  })

  it('unpins and moves', () => {
    expect(unpinModel(list, 'a::2').map((p) => p.id)).toEqual(['a::1', 'b::1'])
    expect(movePinnedModel(list, 'b::1', 0).map((p) => p.id)).toEqual([
      'b::1',
      'a::1',
      'a::2',
    ])
    expect(movePinnedModel(list, 'a::1', 99).map((p) => p.id)).toEqual([
      'a::2',
      'b::1',
      'a::1',
    ])
    expect(movePinnedModel(list, 'zz::9', 0)).toEqual(list)
  })

  it('remembers the effort a pinned model was last used at', () => {
    expect(withPinnedEffort(list, 'a::1', 'high')[0]).toEqual({
      id: 'a::1',
      effort: 'high',
    })
    expect(withPinnedEffort(list, 'nope::1', 'high')).toEqual(list)
  })
})
