import { describe, expect, it } from 'vitest'
import type { FunctionCallMessage } from '@/types/chat'
import {
  observeRunSubtext,
  observeRunTitle,
  summarizeObserveRun,
} from './ObserveRunCard'

const observe = (
  functionId: string | undefined,
  extra: Partial<FunctionCallMessage> = {},
): FunctionCallMessage => ({
  id: `o-${Math.random()}`,
  role: 'function-call',
  functionId: 'harness::observe',
  input: functionId ? { function_id: functionId } : {},
  output: { observer_id: 'sub' },
  createdAt: 0,
  ...extra,
})

const describeRun = (calls: FunctionCallMessage[]) => {
  const summary = summarizeObserveRun(calls)
  return [observeRunTitle(summary), observeRunSubtext(summary), summary.status]
}

describe('an observe run', () => {
  it('names one repeated function in the title and counts its observers', () => {
    expect(
      describeRun([
        observe('configuration::on-updated'),
        observe('configuration::on-updated'),
      ]),
    ).toEqual(['Observing configuration::on-updated', '2 observers', 'done'])
  })

  it('reads an unloaded observe by its intent, then as loading', () => {
    expect(
      describeRun([
        observe(undefined, {
          unloaded: true,
          output: undefined,
          description: 'Arming the observer for sub-agent',
        }),
        observe(undefined, { unloaded: true, output: undefined }),
        observe('secrets::on-changed'),
      ]),
    ).toEqual([
      'Observing 3 functions',
      'sub-agent, secrets::on-changed, 1 loading',
      'done',
    ])
  })

  it('counts failures and lets approval outrank running', () => {
    expect(
      describeRun([
        observe('a::b', { output: { error: 'nope' } }),
        observe('c::d'),
      ]),
    ).toEqual(['Observing 2 functions', 'a::b, c::d · 1 failed', 'error'])
    expect(
      summarizeObserveRun([
        observe('a::b', { running: true }),
        observe('c::d', {
          pendingApproval: true,
        } as Partial<FunctionCallMessage>),
      ]).status,
    ).toBe('pending')
  })
})
