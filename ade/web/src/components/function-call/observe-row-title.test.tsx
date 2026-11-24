/**
 * A `harness::observe` row reads in iii's words: "Observing <function>" —
 * never the agent's own "Arming the wake …" prose, and never "tools".
 */

import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import type { FunctionCallMessage } from '@/types/chat'
import { FunctionCallCard } from './FunctionCallCard'
import { observeRowTitle } from './observe-title'

vi.mock('@/lib/ui-slots', () => ({
  useExtRenderers: () => [],
  useExtObserverActivityRenderers: () => [],
}))

function observe(
  extra: Partial<FunctionCallMessage> = {},
): FunctionCallMessage {
  return {
    id: 'obs-1',
    role: 'function-call',
    functionId: 'harness::observe',
    createdAt: 0,
    description: 'Arming the wake for round 2 wave 1 results',
    input: {
      function_id: 'state::on-changed',
      filters: { 'input.key': { $eq: 'round-2-wave-1' } },
    },
    output: { observer_id: 'sub_1', once: true },
    durationMs: 4,
    ...extra,
  }
}

const text = (markup: string) => markup.replace(/<[^>]+>/g, '')

describe('observe row title', () => {
  it('names the observed function instead of the agent prose', () => {
    const out = renderToStaticMarkup(<FunctionCallCard message={observe()} />)
    expect(text(out)).toContain('Observing state::on-changed')
    expect(out).toContain('data-observe-target="state::on-changed"')
    expect(text(out)).not.toMatch(/\barm(ing|ed)?\b|\bwakes?\b|\btools?\b/i)
  })

  it('keeps the same title while running, on error and once expanded', () => {
    for (const extra of [
      { running: true, output: undefined, durationMs: undefined },
      { output: { error: 'binding store unavailable' } },
    ] satisfies Partial<FunctionCallMessage>[]) {
      const out = text(
        renderToStaticMarkup(<FunctionCallCard message={observe(extra)} />),
      )
      expect(out).toContain('Observing state::on-changed')
      expect(out).not.toMatch(/\barming\b|\bwake\b/i)
    }
    const open = renderToStaticMarkup(
      <FunctionCallCard message={observe()} defaultOpen />,
    )
    expect(text(open)).toContain('Observing state::on-changed')
    expect(text(open)).not.toMatch(/\barming\b/i)
  })

  it('falls back to the intent when the function is not known yet', () => {
    const out = text(
      renderToStaticMarkup(
        <FunctionCallCard
          message={observe({
            input: {},
            output: undefined,
            unloaded: true,
            description: 'Arming the observer for sub-agent',
          })}
        />,
      ),
    )
    expect(out).toContain('Observing sub-agent')
    expect(out).not.toMatch(/\barming\b/i)
  })

  it('reads the intent out of the agent prose', () => {
    const intent = (description?: string, input: unknown = {}) =>
      observeRowTitle(input, description)
    expect(intent('Arming the wake for round 2 wave 1 results')).toEqual({
      intent: 'round 2 wave 1 results',
    })
    expect(intent('Arming the observer for sub-agent')).toEqual({
      intent: 'sub-agent',
    })
    expect(intent('Arm a wake on the child-a result')).toEqual({
      intent: 'the child-a result',
    })
    expect(intent('Re-arming wakes for the fan-in')).toEqual({
      intent: 'the fan-in',
    })
    expect(intent('Registering observer: build finished')).toEqual({
      intent: 'build finished',
    })
    expect(intent('Setting up a watch until the deploy lands')).toEqual({
      intent: 'the deploy lands',
    })
    expect(intent('Observing sub-agent')).toEqual({ intent: 'sub-agent' })
    expect(intent('Watching the test run')).toEqual({ intent: 'the test run' })
    // Prose with no observer verb is the agent's own intent, kept as written.
    expect(intent('Wait for the review to finish')).toEqual({
      text: 'Wait for the review to finish',
    })
    // "Creating the ticket" is not an observer: setup verbs need the noun.
    expect(intent('Creating the ticket')).toEqual({
      text: 'Creating the ticket',
    })
    expect(intent(undefined)).toBeNull()
    expect(intent('Arming the wake')).toBeNull()
    // The function id always wins over prose.
    expect(
      intent('Arming the wake', { function_id: 'kanban::on-change' }),
    ).toEqual({ functionId: 'kanban::on-change' })
  })
})
