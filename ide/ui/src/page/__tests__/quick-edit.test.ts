import { describe, expect, it, vi } from 'vitest'
import type { CodeEditorQuickEditRequest, Host } from '@iii-dev/console-ui'
import {
  buildQuickEditPrompt,
  cleanQuickEditAnswer,
  resolveQuickEditModel,
  runQuickEdit,
  splitModel,
} from '../quick-edit'

const request = (overrides: Partial<CodeEditorQuickEditRequest> = {}): CodeEditorQuickEditRequest => ({
  instruction: 'rename x to y',
  scope: 'selection',
  selection: { startLine: 2, startColumn: 1, endLine: 2, endColumn: 6, text: 'let x' },
  value: 'a\nlet x = 1\nb',
  language: 'typescript',
  signal: new AbortController().signal,
  ...overrides,
})

const hostWith = (trigger: (id: string, payload?: Record<string, unknown>) => unknown, composerModel?: string | null) =>
  ({
    iii: { trigger: vi.fn(async (id: string, payload?: Record<string, unknown>) => trigger(id, payload)) },
    chat: { composerModel: () => composerModel ?? null },
  }) as unknown as Host

describe('buildQuickEditPrompt', () => {
  it('marks exactly the selected text inside the file', () => {
    const prompt = buildQuickEditPrompt(request(), 'src/a.ts')
    expect(prompt).toContain('Instruction: rename x to y')
    expect(prompt).toContain('File: src/a.ts (typescript)')
    expect(prompt).toContain('a\n<<<EDIT_START>>>let x<<<EDIT_END>>> = 1\nb')
  })

  it('marks the whole file for the file scope', () => {
    expect(buildQuickEditPrompt(request({ scope: 'file' }), 'a.ts')).toContain('<<<EDIT_START>>>\na\nlet x = 1\nb\n<<<EDIT_END>>>')
  })

  it('elides lines far from the selection', () => {
    const value = Array.from({ length: 400 }, (_, i) => `line ${i + 1}`).join('\n')
    const prompt = buildQuickEditPrompt(
      request({ value, selection: { startLine: 200, startColumn: 1, endLine: 200, endColumn: 9, text: 'line 200' } }),
      'a.ts',
    )
    expect(prompt).toContain('… (79 lines above)')
    expect(prompt).toContain('… (80 lines below)')
    expect(prompt).not.toContain('line 79\n')
  })
})

describe('cleanQuickEditAnswer', () => {
  it('unwraps a fence and drops echoed markers', () => {
    expect(cleanQuickEditAnswer('```ts\nlet y\n```', 'let x')).toBe('let y')
    expect(cleanQuickEditAnswer('<<<EDIT_START>>>let y<<<EDIT_END>>>', 'let x')).toBe('let y')
  })

  it("matches the original's trailing newline", () => {
    expect(cleanQuickEditAnswer('a', 'x\n')).toBe('a\n')
    expect(cleanQuickEditAnswer('a\n\n', 'x')).toBe('a')
  })

  it('leaves fences that are part of the code', () => {
    const md = 'text\n```\ncode\n```\nmore'
    expect(cleanQuickEditAnswer(md, md)).toBe(md)
  })
})

describe('models', () => {
  it('splits provider::model', () => {
    expect(splitModel('anthropic::claude-sonnet-5-5')).toEqual({ provider: 'anthropic', model: 'claude-sonnet-5-5' })
    expect(splitModel('gpt-x')).toEqual({ provider: null, model: 'gpt-x' })
  })

  it("prefers the chat composer's model, else the first configured provider", async () => {
    expect(await resolveQuickEditModel(hostWith(() => ({}), 'openai::gpt'), null)).toBe('openai::gpt')
    const host = hostWith(() => ({
      providers: [
        { id: 'copilot', configured: false, available: true, default_model: 'c' },
        { id: 'anthropic', configured: true, available: true, default_model: 'claude' },
      ],
    }))
    expect(await resolveQuickEditModel(host, null)).toBe('anthropic::claude')
    await expect(resolveQuickEditModel(hostWith(() => ({ providers: [] })), null)).rejects.toThrow(/No model/)
  })
})

describe('runQuickEdit', () => {
  it('sends provider and model apart and returns the cleaned text', async () => {
    let sent: Record<string, unknown> | undefined
    const host = hostWith((_, payload) => {
      sent = payload
      return { message: { content: [{ type: 'text', text: '```\nlet y\n```' }] } }
    })
    expect(await runQuickEdit(host, request(), 'a.ts', 'anthropic::claude')).toBe('let y')
    expect(sent).toMatchObject({ provider: 'anthropic', model: 'claude', system_prompt: expect.any(String) })
  })

  it('surfaces a router error turn', async () => {
    const host = hostWith(() => ({ message: { stop_reason: 'error', error_message: 'rate limited' } }))
    await expect(runQuickEdit(host, request(), 'a.ts', 'm')).rejects.toThrow('rate limited')
  })

  it('stops waiting when aborted', async () => {
    const abort = new AbortController()
    const host = hostWith(() => new Promise(() => {}))
    const pending = runQuickEdit(host, request({ signal: abort.signal }), 'a.ts', 'm')
    abort.abort()
    await expect(pending).rejects.toMatchObject({ name: 'AbortError' })
  })
})
