import { describe, expect, it } from 'vitest'
import { getAnthropicTools, TOOLS, toolNameToFunctionId } from '../src/tools.js'

describe('proof tool registry', () => {
  it('exposes a non-empty, unique-named tool catalog', () => {
    expect(TOOLS.length).toBeGreaterThan(0)

    const names = TOOLS.map((t) => t.name)
    expect(new Set(names).size).toBe(names.length)

    for (const tool of TOOLS) {
      expect(tool.name).toMatch(/^browser_/)
      expect(tool.function_id).toMatch(/^proof::browser::/)
      expect(tool.description.length).toBeGreaterThan(0)
      expect(tool.input_schema).toMatchObject({ type: 'object' })
    }
  })

  it('maps each tool name to its function id', () => {
    expect(toolNameToFunctionId('browser_navigate')).toBe('proof::browser::navigate')
    expect(toolNameToFunctionId('browser_snapshot')).toBe('proof::browser::snapshot')
  })

  it('throws on unknown tool names instead of silently returning undefined', () => {
    expect(() => toolNameToFunctionId('not_a_real_tool')).toThrow(/Unknown tool/)
  })

  it('shapes tools for the Anthropic API by stripping the iii function_id', () => {
    const anthropic = getAnthropicTools()
    expect(anthropic).toHaveLength(TOOLS.length)
    for (const t of anthropic) {
      expect(t).toHaveProperty('name')
      expect(t).toHaveProperty('description')
      expect(t).toHaveProperty('input_schema')
      expect(t).not.toHaveProperty('function_id')
    }
  })
})
