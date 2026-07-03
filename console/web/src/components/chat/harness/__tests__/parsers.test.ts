import { describe, expect, it } from 'vitest'
import { parseSandboxErrorDisplay } from '@/components/chat/sandbox/parsers'
import {
  HARNESS_FUNCTION_IDS,
  isHarnessFunction,
  safeParseRequest,
  spawnRequestSchema,
  spawnResponseSchema,
  taskText,
  unwrapEnvelope,
} from '../parsers'

/** entry-mapper success shape: { content, details }. */
function resultEnvelope(text: string, details: unknown) {
  return { content: [{ type: 'text', text }], details }
}

/** entry-mapper error shape (functionResultOutput, is_error branch). */
function errorEnvelope(code: string, message: string) {
  return {
    error: {
      kind: 'function_error',
      message,
      details: { error: code, message },
      content: [{ type: 'text', text: message }],
    },
  }
}

describe('isHarnessFunction', () => {
  it('matches every id in the explicit allowlist', () => {
    for (const id of HARNESS_FUNCTION_IDS) {
      expect(isHarnessFunction(id)).toBe(true)
    }
  })

  it('rejects unrelated ids', () => {
    expect(isHarnessFunction('harness::send')).toBe(false)
    expect(isHarnessFunction('harness::')).toBe(false)
    expect(isHarnessFunction('submit_results')).toBe(false)
  })
})

describe('spawnRequestSchema', () => {
  it('accepts a minimal request', () => {
    const r = safeParseRequest(spawnRequestSchema, { task: 'do the thing' })
    expect(r?.task).toBe('do the thing')
  })

  it('accepts a fully-populated request', () => {
    const r = safeParseRequest(spawnRequestSchema, {
      task: 'audit the CI runs',
      model: 'claude-sonnet-4-6',
      provider: 'anthropic',
      session_id: 's_123',
      parent_session_id: 's_parent',
      spawned_by_subscription_id: 'sub_1',
      reactive_depth: 2,
      options: {
        system_prompt: 'be terse',
        system_prompt_strategy: 'enrich',
        mode: 'agent',
        max_turns: 8,
        thinking_level: 'low',
        output: { type: 'json', schema: { type: 'object' } },
        functions: {
          allow: ['web::fetch'],
          deny: ['sandbox::fs::rm'],
          expose: 'agent_trigger',
        },
        max_children: 2,
        pending_timeout_ms: 300_000,
      },
    })
    expect(r?.options?.mode).toBe('agent')
    expect(r?.options?.output?.type).toBe('json')
    expect(r?.options?.functions?.allow).toEqual(['web::fetch'])
  })

  it('accepts a task in AgentMessage form', () => {
    const r = safeParseRequest(spawnRequestSchema, {
      task: { role: 'user', content: [{ type: 'text', text: 'hi' }] },
    })
    expect(r).not.toBeNull()
    expect(taskText(r?.task)).toBe('hi')
  })

  it('tolerates a clipped approval excerpt', () => {
    // a gated call's preview input can be a redacted arguments_excerpt
    expect(safeParseRequest(spawnRequestSchema, {})).not.toBeNull()
    expect(safeParseRequest(spawnRequestSchema, undefined)).not.toBeNull()
  })

  it('keeps unknown additive wire fields from breaking the parse', () => {
    const r = safeParseRequest(spawnRequestSchema, {
      task: 'x',
      some_future_field: true,
    })
    expect(r?.task).toBe('x')
  })

  it('rejects a non-object payload', () => {
    expect(safeParseRequest(spawnRequestSchema, 'not a request')).toBeNull()
  })
})

describe('spawnResponseSchema', () => {
  it('parses the direct-call acknowledgement', () => {
    const parsed = spawnResponseSchema.safeParse({
      child_session_id: 's_1',
      child_turn_id: 't_1',
    })
    expect(parsed.success).toBe(true)
  })

  it('rejects the free-form child result', () => {
    expect(spawnResponseSchema.safeParse('a markdown report').success).toBe(
      false,
    )
    expect(spawnResponseSchema.safeParse({ status: 'ok' }).success).toBe(false)
  })
})

describe('unwrapEnvelope on spawn outputs', () => {
  it('yields the raw string for a text-contract result', () => {
    expect(unwrapEnvelope(resultEnvelope('final text', 'final text'))).toBe(
      'final text',
    )
  })

  it('yields the structured value for a json-contract result', () => {
    const details = { status: 'ok', failing: 2 }
    expect(
      unwrapEnvelope(resultEnvelope(JSON.stringify(details), details)),
    ).toEqual(details)
  })

  it('leaves a bare direct-call response untouched', () => {
    const direct = { child_session_id: 's_1', child_turn_id: 't_1' }
    expect(unwrapEnvelope(direct)).toBe(direct)
  })
})

describe('taskText', () => {
  it('passes a string task through', () => {
    expect(taskText('summarize the repo')).toBe('summarize the repo')
  })

  it('joins text blocks of an AgentMessage task', () => {
    expect(
      taskText({
        role: 'user',
        content: [
          { type: 'text', text: 'line one' },
          { type: 'image', mime: 'image/png', data: '…' },
          { type: 'text', text: 'line two' },
        ],
      }),
    ).toBe('line one\nline two')
  })

  it('returns null for missing or empty tasks', () => {
    expect(taskText(undefined)).toBeNull()
    expect(taskText('')).toBeNull()
    expect(taskText({ role: 'user', content: [] })).toBeNull()
  })
})

describe('spawn error dispatch', () => {
  it('routes guard errors to the invocation error display', () => {
    // locks in the dispatcher's error-before-success ordering
    const display = parseSandboxErrorDisplay(
      errorEnvelope(
        'harness/spawn_depth_exceeded',
        'harness/spawn_depth_exceeded: child depth 3 exceeds max_depth 2',
      ),
    )
    expect(display?.variant).toBe('invocation')
    if (display?.variant === 'invocation') {
      expect(display.error.message).toContain('harness/spawn_depth_exceeded')
    }
  })

  it('does not flag a successful result envelope as an error', () => {
    expect(
      parseSandboxErrorDisplay(resultEnvelope('all done', 'all done')),
    ).toBeNull()
  })
})
