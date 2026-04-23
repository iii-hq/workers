import { describe, expect, it } from 'vitest'
import type { CoverageReport } from '../src/context.js'
import { buildUserPrompt, SYSTEM_PROMPT } from '../src/prompt.js'

describe('proof prompt builder', () => {
  it('exports a non-trivial system prompt that documents the workflow', () => {
    expect(SYSTEM_PROMPT.length).toBeGreaterThan(200)
    expect(SYSTEM_PROMPT).toMatch(/browser_snapshot/)
    expect(SYSTEM_PROMPT).toMatch(/RUN_COMPLETED/)
  })

  it('includes the base url, file list, and diff fence in the user prompt', () => {
    const prompt = buildUserPrompt('--- a/foo.ts\n+++ b/foo.ts', ['foo.ts'], 'http://localhost:3000')

    expect(prompt).toContain('## Base URL\nhttp://localhost:3000')
    expect(prompt).toContain('## Changed Files (1)\n- foo.ts')
    expect(prompt).toContain('## Diff\n```diff\n--- a/foo.ts\n+++ b/foo.ts\n```')
    expect(prompt).not.toContain('## Instruction')
    expect(prompt).not.toContain('## Recent Commits')
    expect(prompt).not.toContain('## Test Coverage')
  })

  it('prepends an instruction section when one is supplied', () => {
    const prompt = buildUserPrompt('diff', [], 'http://x', 'verify the new login flow')
    expect(prompt.startsWith('## Instruction\nverify the new login flow')).toBe(true)
  })

  it('renders short commit hashes when commits are provided', () => {
    const prompt = buildUserPrompt('diff', [], 'http://x', undefined, [
      { hash: 'abcdef0123456789', subject: 'add login' },
    ])
    expect(prompt).toContain('## Recent Commits\n- abcdef0 add login')
  })

  it('summarizes coverage when entries are provided', () => {
    const coverage: CoverageReport = {
      percent: 50,
      coveredCount: 1,
      totalCount: 2,
      entries: [
        { path: 'src/a.ts', covered: true, testFiles: ['tests/a.test.ts'] },
        { path: 'src/b.ts', covered: false, testFiles: [] },
      ],
    }
    const prompt = buildUserPrompt('diff', [], 'http://x', undefined, undefined, coverage)
    expect(prompt).toContain('## Test Coverage (50% — 1/2 files)')
    expect(prompt).toContain('[covered] src/a.ts (tested by: tests/a.test.ts)')
    expect(prompt).toContain('[no test] src/b.ts')
  })

  it('omits the coverage section when totalCount is zero', () => {
    const coverage: CoverageReport = { percent: 0, coveredCount: 0, totalCount: 0, entries: [] }
    const prompt = buildUserPrompt('diff', [], 'http://x', undefined, undefined, coverage)
    expect(prompt).not.toContain('## Test Coverage')
  })
})
