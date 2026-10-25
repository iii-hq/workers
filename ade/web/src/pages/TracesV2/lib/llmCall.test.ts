import { describe, expect, it } from 'vitest'
import {
  formatCost,
  formatTokenCount,
  isLlmCall,
  llmCallFromSpan,
  parsePayload,
  repairTruncatedJson,
  truncatedBytesOf,
} from './llmCall'

const CHAT_REQUEST = {
  _caller_worker_id: 'w1',
  model: 'claude-sonnet-5-5',
  provider: 'anthropic',
  system_prompt: 'You are an iii agent.',
  max_output_tokens: 128000,
  messages: [
    { role: 'user', content: [{ type: 'text', text: 'hey' }], timestamp: 1 },
    {
      role: 'assistant',
      content: [{ type: 'text', text: 'Hey! What can I help with?' }],
      model: 'claude-sonnet-5-5',
      stop_reason: 'end',
      usage: { input: 1289, output: 14, cache_read: 0, cache_write: 24533 },
    },
    {
      role: 'assistant',
      content: [
        {
          type: 'tool_use',
          id: 't1',
          name: 'agent_trigger',
          input: { function: 'state::get' },
        },
      ],
    },
    {
      role: 'user',
      content: [{ type: 'tool_result', tool_use_id: 't1', content: 'ok' }],
    },
  ],
  tools: [
    {
      name: 'agent_trigger',
      description: 'Trigger any function',
      parameters: { type: 'object' },
    },
  ],
}

const CHAT_RESPONSE = {
  model: 'claude-sonnet-5-5',
  ok: true,
  provider: 'anthropic',
  stop_reason: 'function_call',
  usage: {
    cache_read: 0,
    cache_write: 25802,
    cost_usd: 0.1030595,
    input: 235,
    output: 133,
  },
}

describe('repairTruncatedJson', () => {
  const whole = JSON.stringify(CHAT_REQUEST)

  it('returns a document that already parses untouched', () => {
    expect(repairTruncatedJson(whole)).toEqual(CHAT_REQUEST)
  })

  it('closes a cut inside a string value and strips the engine marker', () => {
    const cut = `${whole.slice(0, whole.indexOf('Hey! What') + 6)}…[truncated 84686 bytes]`
    const repaired = repairTruncatedJson(cut) as {
      model: string
      messages: Array<{ content: Array<{ text?: string }> }>
    }
    expect(repaired).not.toBeNull()
    expect(repaired.model).toBe('claude-sonnet-5-5')
    expect(repaired.messages).toHaveLength(2)
    expect(repaired.messages[1].content[0].text).toBe('Hey! W')
  })

  it('handles a cut right after a key, a colon, a comma, or mid-literal', () => {
    const cutAt = (needle: string) =>
      whole.slice(0, whole.indexOf(needle) + needle.length)
    expect(repairTruncatedJson(cutAt('"provider":'))).toMatchObject({
      _caller_worker_id: 'w1',
      model: 'claude-sonnet-5-5',
      provider: null,
    })
    expect(
      repairTruncatedJson(cutAt('"model":"claude-sonnet-5-5",')),
    ).toMatchObject({
      model: 'claude-sonnet-5-5',
    })
    expect(repairTruncatedJson(cutAt('"max_output_tokens":128'))).toMatchObject(
      {
        _caller_worker_id: 'w1',
      },
    )
    expect(repairTruncatedJson(cutAt('"system_pro'))).toMatchObject({
      model: 'claude-sonnet-5-5',
    })
    expect(
      repairTruncatedJson(
        cutAt(
          '"content":[{"type":"text","text":"hey"}],"timestamp":1},{"role":"assistant","content":[{"type":"text","text":"Hey! What can I help with?"}],"model":"claude-sonnet-5-5","stop_reason":"end","usage":{"input":1289,"output":14,"cache_read":0,"cache_write":24533}},{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"agent_trigger","input":{"function":"state::get"}}]},{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"ok"}]}],"tools":[{"name":"agent_trigger","description":"Trigger any function","parameters":{"type":"object"}}]}',
        ),
      ),
    ).toEqual(CHAT_REQUEST)
  })

  it('drops a trailing lone backslash before closing a string', () => {
    const cut = '{"text":"line one\\nline two\\'
    expect(repairTruncatedJson(cut)).toEqual({ text: 'line one\nline two' })
  })

  it('gives up on text that was never JSON', () => {
    expect(repairTruncatedJson('plain words…[truncated 5 bytes]')).toBeNull()
  })
})

describe('parsePayload / truncatedBytesOf', () => {
  it('passes objects through, parses strings, repairs cut strings', () => {
    expect(parsePayload(CHAT_RESPONSE)).toBe(CHAT_RESPONSE)
    expect(parsePayload(JSON.stringify(CHAT_RESPONSE))).toEqual(CHAT_RESPONSE)
    const cut = `${JSON.stringify(CHAT_RESPONSE).slice(0, 40)}…[truncated 10 bytes]`
    expect(parsePayload(cut)).toMatchObject({ model: 'claude-sonnet-5-5' })
    expect(parsePayload('not json')).toBe('not json')
    expect(truncatedBytesOf(cut)).toBe(10)
    expect(truncatedBytesOf(JSON.stringify(CHAT_RESPONSE))).toBeNull()
    expect(truncatedBytesOf(CHAT_RESPONSE)).toBeNull()
  })
})

describe('llmCallFromSpan', () => {
  it('reads the router chat shape: system, messages, tools, params, usage', () => {
    const call = llmCallFromSpan(
      {
        'gen_ai.request.model': 'claude-sonnet-5-5',
        'gen_ai.system': 'anthropic',
      },
      JSON.stringify(CHAT_REQUEST),
      CHAT_RESPONSE,
    )
    expect(call.model).toBe('claude-sonnet-5-5')
    expect(call.provider).toBe('anthropic')
    expect(call.stopReason).toBe('function_call')
    expect(call.systemPrompt).toBe('You are an iii agent.')
    expect(call.messages.map((m) => m.role)).toEqual([
      'user',
      'assistant',
      'assistant',
      'user',
    ])
    expect(call.messages[1].usage?.input).toBe(1289)
    expect(call.messages[2].blocks[0]).toMatchObject({
      type: 'tool_use',
      name: 'agent_trigger',
    })
    expect(call.messages[3].blocks[0]).toMatchObject({
      type: 'tool_result',
      toolUseId: 't1',
    })
    expect(call.tools).toEqual([
      {
        name: 'agent_trigger',
        description: 'Trigger any function',
        parameters: { type: 'object' },
      },
    ])
    expect(call.params).toEqual({ max_output_tokens: 128000 })
    expect(call.usage).toEqual({
      input: 235,
      output: 133,
      cacheRead: 0,
      cacheWrite: 25802,
      costUsd: 0.1030595,
    })
    expect(call.responseFields).toEqual({})
    expect(call.requestTruncatedBytes).toBeNull()
  })

  it('falls back to gen_ai attributes and OpenAI-style usage keys', () => {
    const call = llmCallFromSpan(
      {
        'gen_ai.system': 'openai',
        'gen_ai.request.model': 'gpt-5',
        'gen_ai.response.finish_reason': 'stop',
        'gen_ai.usage.input_tokens': '1240',
        'gen_ai.usage.output_tokens': 342,
      },
      undefined,
      { choices: [], usage: { prompt_tokens: 10, completion_tokens: 5 } },
    )
    expect(call.model).toBe('gpt-5')
    expect(call.provider).toBe('openai')
    expect(call.stopReason).toBe('stop')
    expect(call.usage).toEqual({
      input: 10,
      output: 5,
      cacheRead: undefined,
      cacheWrite: undefined,
      costUsd: undefined,
    })
    expect(call.responseFields).toEqual({ choices: [] })
    expect(call.messages).toEqual([])
  })

  it('reports the bytes the engine dropped from a truncated request', () => {
    const cut = `${JSON.stringify(CHAT_REQUEST).slice(0, 120)}…[truncated 84686 bytes]`
    const call = llmCallFromSpan(undefined, cut, undefined)
    expect(call.requestTruncatedBytes).toBe(84686)
    expect(call.model).toBe('claude-sonnet-5-5')
  })
})

describe('isLlmCall', () => {
  it('matches router/provider function ids, gen_ai attributes, or a messages+model payload', () => {
    expect(isLlmCall('router::chat', undefined, undefined)).toBe(true)
    expect(isLlmCall('provider::anthropic::stream', undefined, undefined)).toBe(
      true,
    )
    expect(isLlmCall('router::models::list', undefined, undefined)).toBe(false)
    expect(isLlmCall(null, { 'gen_ai.request.model': 'x' }, undefined)).toBe(
      true,
    )
    expect(isLlmCall('judge::evaluate', {}, { messages: [], model: 'm' })).toBe(
      true,
    )
    expect(isLlmCall('judge::evaluate', {}, { evaluations: [] })).toBe(false)
  })
})

describe('formatters', () => {
  it('compacts token counts and prices', () => {
    expect(formatTokenCount(641)).toBe('641')
    expect(formatTokenCount(1289)).toBe('1,289')
    expect(formatTokenCount(25802)).toBe('25.8k')
    expect(formatTokenCount(1_000_000)).toBe('1M')
    expect(formatCost(0.1030595)).toBe('$0.103')
    expect(formatCost(0.0012)).toBe('$0.0012')
    expect(formatCost(12.5)).toBe('$12.50')
  })
})
