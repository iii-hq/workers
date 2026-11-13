import { describe, expect, it } from 'vitest'
import {
  inheritedTags,
  isEngineRoutingSpan,
  resolveSpanLabel,
  type TagCarrier,
  tagRootKind,
} from './spanLabel'

const DISPLAY = 'iii.tag.display_name'
const KIND = 'iii.tag.kind'

describe('resolveSpanLabel', () => {
  it('verb-strips the span name when no override is set', () => {
    expect(resolveSpanLabel({ name: 'execute session::append' })).toBe(
      'session::append',
    )
  })

  it('applies a display-name override on the scope span (nothing inherited)', () => {
    expect(
      resolveSpanLabel({
        name: 'harness::turn step',
        attributes: { [DISPLAY]: 'Sub-agent · list workers' },
      }),
    ).toBe('Sub-agent · list workers')
  })

  it('applies the override when the ancestry carries a DIFFERENT name', () => {
    expect(
      resolveSpanLabel(
        {
          name: 'harness::turn step',
          attributes: { [DISPLAY]: 'Sub-agent · list workers' },
        },
        'queue(default) harness::turn',
      ),
    ).toBe('Sub-agent · list workers')
  })

  it('suppresses the override on baggage echoes (same name inherited)', () => {
    // The regression this guards: baggage copies `iii.tag.display_name`
    // onto EVERY span started inside a sub-agent turn, so its session
    // writes, LLM calls, and tool spans all repeated the sub-agent title
    // (trace f6292958dfe97afbd87e323d4f4541b6 — 69 of 129 spans).
    expect(
      resolveSpanLabel(
        {
          name: 'execute session::update-message',
          attributes: {
            [DISPLAY]: 'Sub-agent · list workers',
            [KIND]: 'harness.subagent',
          },
        },
        'Sub-agent · list workers',
      ),
    ).toBe('session::update-message')
  })

  it('ignores non-string overrides', () => {
    expect(
      resolveSpanLabel({
        name: 'execute worker::list',
        attributes: { [DISPLAY]: 42 },
      }),
    ).toBe('worker::list')
  })
})

describe('tagRootKind', () => {
  it('returns the kind on a scope root (nothing or a different kind inherited)', () => {
    expect(tagRootKind({ [KIND]: 'harness.subagent' }, undefined)).toBe(
      'harness.subagent',
    )
    expect(tagRootKind({ [KIND]: 'harness.subagent' }, 'harness.turn')).toBe(
      'harness.subagent',
    )
  })

  it('returns null on echoes and untagged spans', () => {
    expect(
      tagRootKind({ [KIND]: 'harness.subagent' }, 'harness.subagent'),
    ).toBeNull()
    expect(tagRootKind({}, undefined)).toBeNull()
    expect(tagRootKind(undefined, undefined)).toBeNull()
  })
})

describe('isEngineRoutingSpan', () => {
  it('classifies engine call/handle_invocation wrappers as routing', () => {
    expect(
      isEngineRoutingSpan({
        name: 'call session::update-message',
        attributes: { function_id: 'session::update-message' },
      }),
    ).toBe(true)
    expect(
      isEngineRoutingSpan({
        name: 'handle_invocation session::update-message',
        attributes: { function_id: 'session::update-message' },
      }),
    ).toBe(true)
  })

  it('does NOT classify built-in call spans as routing', () => {
    // A `call <fn>` span with `iii.function.kind: internal` is an engine
    // built-in executing in-process (`configuration::list`, `state::get`):
    // no worker `execute` span exists behind it, so the call span is the
    // invocation's only record — skipping it as a wrapper would erase the
    // call (and its failure) from the view.
    expect(
      isEngineRoutingSpan({
        name: 'call configuration::list',
        attributes: {
          function_id: 'configuration::list',
          'iii.function.kind': 'internal',
        },
      }),
    ).toBe(false)
  })

  it('leaves worker client spans (no function_id attr) alone', () => {
    expect(isEngineRoutingSpan({ name: 'call llm::generate' })).toBe(false)
  })
})

describe('inheritedTags', () => {
  function chain(
    spans: Record<string, { parent?: string; kind?: string; display?: string }>,
  ): (id: string) => TagCarrier | undefined {
    return (id) => {
      const s = spans[id]
      if (!s) return undefined
      const attributes: Record<string, unknown> = {}
      if (s.kind !== undefined) attributes[KIND] = s.kind
      if (s.display !== undefined) attributes[DISPLAY] = s.display
      return { attributes, parent_span_id: s.parent }
    }
  }

  it('takes each tag from the nearest ancestor that carries it', () => {
    const lookup = chain({
      step: { kind: 'harness.subagent', display: 'Sub-agent · list workers' },
      gap: { parent: 'step' }, // e.g. an older-SDK worker span with no tags
      leaf: { parent: 'gap' },
    })
    // The gap span must not hide the scope from its descendants: this is
    // how `router::models::get` under a tag-less `context::assemble` still
    // reads as an echo of the sub-agent scope.
    expect(inheritedTags('gap', lookup)).toEqual({
      kind: 'harness.subagent',
      displayName: 'Sub-agent · list workers',
    })
    expect(inheritedTags('step', lookup)).toEqual({
      kind: 'harness.subagent',
      displayName: 'Sub-agent · list workers',
    })
  })

  it('resolves kind and display independently across different ancestors', () => {
    const lookup = chain({
      outer: { display: 'queue(default) harness::turn', kind: 'queue.process' },
      mid: { parent: 'outer', kind: 'harness.turn' },
      inner: { parent: 'mid' },
    })
    expect(inheritedTags('inner', lookup)).toEqual({
      kind: 'harness.turn',
      displayName: 'queue(default) harness::turn',
    })
  })

  it('returns empty for roots, unknown parents, and cycles', () => {
    expect(inheritedTags(undefined, chain({}))).toEqual({})
    expect(inheritedTags('missing', chain({}))).toEqual({})
    const cyclic = chain({ a: { parent: 'b' }, b: { parent: 'a' } })
    expect(inheritedTags('a', cyclic)).toEqual({})
  })
})
