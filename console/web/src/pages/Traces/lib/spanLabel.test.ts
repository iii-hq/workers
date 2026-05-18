// Coverage for the engine-routing heuristics that drive the
// "Hide engine routing" and "Collapse routing pairs" toggles in
// WaterfallChart. The predicates rely on the engine's distinctive verb
// prefixes (`handle_invocation X` / `call X`) — service_name is not
// gated because operators routinely override `OTEL_SERVICE_NAME` and
// the toggle must keep working in those deployments.

import { describe, expect, it } from 'vitest'
import {
  formatSpanLabel,
  getSpanKindIndicator,
  isEngineRoutingPair,
  isEngineRoutingSpan,
} from './spanLabel'

describe('getSpanKindIndicator', () => {
  it('maps each OTel span kind to a compact icon + label', () => {
    expect(getSpanKindIndicator('server')).toEqual({
      icon: '▶',
      label: 'server (handles incoming)',
    })
    expect(getSpanKindIndicator('client')).toEqual({
      icon: '↗',
      label: 'client (outgoing call)',
    })
    expect(getSpanKindIndicator('producer')).toEqual({
      icon: '↥',
      label: 'producer (sends to queue)',
    })
    expect(getSpanKindIndicator('consumer')).toEqual({
      icon: '↧',
      label: 'consumer (reads from queue)',
    })
    expect(getSpanKindIndicator('internal')).toEqual({
      icon: '•',
      label: 'internal',
    })
  })

  it('is case-insensitive on the kind string', () => {
    expect(getSpanKindIndicator('SERVER')?.icon).toBe('▶')
    expect(getSpanKindIndicator('Client')?.icon).toBe('↗')
  })

  it('returns null for an unrecognized kind', () => {
    expect(getSpanKindIndicator('weird')).toBeNull()
  })

  it('returns null for missing kind', () => {
    expect(getSpanKindIndicator(undefined)).toBeNull()
  })
})

describe('formatSpanLabel', () => {
  it('strips the `handle_invocation ` engine verb prefix', () => {
    expect(
      formatSpanLabel({
        name: 'handle_invocation fn-foo',
        service_name: 'iii',
      }),
    ).toBe('fn-foo')
  })

  it('strips the `call ` engine verb prefix', () => {
    expect(formatSpanLabel({ name: 'call fn-foo', service_name: 'iii' })).toBe(
      'fn-foo',
    )
  })

  it('strips the service-name dot prefix when it appears AFTER the verb strip', () => {
    // Real-world case: span name is `call billing.charge`, service is
    // `billing`. We strip the `call ` first, then strip `billing.` to
    // produce just `charge`. The verb strip must happen first because
    // the service prefix doesn't appear at the start of the original
    // name.
    expect(
      formatSpanLabel({ name: 'call billing.charge', service_name: 'billing' }),
    ).toBe('charge')
  })

  it('does NOT strip a service-name prefix that is just a substring (only leading)', () => {
    expect(formatSpanLabel({ name: 'fooBar', service_name: 'foo' })).toBe(
      'fooBar',
    )
  })

  it('returns the name unchanged when no prefix matches', () => {
    expect(formatSpanLabel({ name: 'simple-name', service_name: 'svc' })).toBe(
      'simple-name',
    )
  })

  it('handles missing service_name', () => {
    expect(
      formatSpanLabel({ name: 'call fn-foo', service_name: undefined }),
    ).toBe('fn-foo')
  })

  it('only strips the first matching verb prefix, not chained ones', () => {
    // Defensive: a name like `call handle_invocation foo` is pathological
    // but should still produce a sensible result (strip outermost only).
    expect(
      formatSpanLabel({
        name: 'call handle_invocation foo',
        service_name: 'iii',
      }),
    ).toBe('handle_invocation foo')
  })
})

describe('isEngineRoutingSpan', () => {
  it('matches `handle_invocation X` regardless of service_name', () => {
    expect(
      isEngineRoutingSpan({
        name: 'handle_invocation fn-foo',
        service_name: 'iii',
      }),
    ).toBe(true)
  })

  it('matches `call X` regardless of service_name', () => {
    expect(
      isEngineRoutingSpan({ name: 'call fn-foo', service_name: 'iii' }),
    ).toBe(true)
  })

  it('still matches when service_name is overridden (e.g. by OTEL_SERVICE_NAME)', () => {
    // Regression guard for the "hide engine routing" toggle going dark
    // in deployments that override the engine's default service name.
    expect(
      isEngineRoutingSpan({
        name: 'handle_invocation fn-foo',
        service_name: 'iii-engine',
      }),
    ).toBe(true)
    expect(
      isEngineRoutingSpan({
        name: 'call fn-foo',
        service_name: 'engine',
      }),
    ).toBe(true)
  })

  it('does NOT match a non-routing name', () => {
    expect(
      isEngineRoutingSpan({ name: 'process_event', service_name: 'iii' }),
    ).toBe(false)
  })

  it('still matches when service_name is missing (verb prefix is sufficient signal)', () => {
    expect(
      isEngineRoutingSpan({ name: 'call fn-foo', service_name: undefined }),
    ).toBe(true)
  })
})

describe('isEngineRoutingPair', () => {
  it('matches when parent.handle_invocation and child.call name the same function', () => {
    expect(
      isEngineRoutingPair(
        { name: 'handle_invocation fn-foo', service_name: 'iii' },
        { name: 'call fn-foo', service_name: 'iii' },
      ),
    ).toBe(true)
  })

  it('does NOT match when the function names differ', () => {
    expect(
      isEngineRoutingPair(
        { name: 'handle_invocation fn-foo', service_name: 'iii' },
        { name: 'call fn-bar', service_name: 'iii' },
      ),
    ).toBe(false)
  })

  it('does NOT match when the parent is not a handle_invocation', () => {
    expect(
      isEngineRoutingPair(
        { name: 'process_event', service_name: 'iii' },
        { name: 'call fn-foo', service_name: 'iii' },
      ),
    ).toBe(false)
  })

  it('does NOT match when the child is not a call', () => {
    expect(
      isEngineRoutingPair(
        { name: 'handle_invocation fn-foo', service_name: 'iii' },
        { name: 'process fn-foo', service_name: 'iii' },
      ),
    ).toBe(false)
  })

  it('still matches when service_name is overridden on either side', () => {
    // Predicate works on the verb prefixes only; service_name overrides
    // (`OTEL_SERVICE_NAME`, custom `otel.service_name`) must not break it.
    expect(
      isEngineRoutingPair(
        { name: 'handle_invocation fn-foo', service_name: 'iii-engine' },
        { name: 'call fn-foo', service_name: 'iii-engine' },
      ),
    ).toBe(true)
    expect(
      isEngineRoutingPair(
        { name: 'handle_invocation fn-foo', service_name: undefined },
        { name: 'call fn-foo', service_name: undefined },
      ),
    ).toBe(true)
  })
})
