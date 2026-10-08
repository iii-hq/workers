import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { FindRelevantCard } from '../FindRelevantCard'
import {
  firstLocation,
  namedLeads,
  openFileRequest,
  previewLines,
  sharedFolder,
  summarizeFindRelevant,
} from '../find-relevant'

const input = { query: 'how does the hub pick a provider?', path: 'judge' }
const file = (path: string, score: number, extra: Record<string, unknown> = {}) => ({
  path,
  score,
  roles: ['implementation', 'caller', 'test'],
  excerpts: [],
  leads: [],
  call_leads: [],
  source_omitted: false,
  ...extra,
})
const output = {
  status: 'complete',
  reason: null,
  files: [
    file('/r/judge/src/register.rs', 0.97, {
      priority: 0.96,
      excerpts: [{ line_from: 35, line_to: 52, text: 'fn resolve_provider() {\n    request\n}\n' }],
      leads: [{ name: 'resolve_provider', line_from: 35, line_to: 52, score: 0.9 }],
    }),
    file('/r/judge/tests/register.rs', 0.9),
    file('/r/judge/src/main.rs', 0.8),
    file('/r/judge/README.md', 0.7),
    file('/r/judge/src/config.rs', 0.6),
    file('/r/judge/src/ui.rs', 0.55),
  ],
  agents_md: [],
  issues: {},
  stats: { judge_calls: 44, questions: 700, input_tokens: 1, cache_hits: 0, elapsed_ms: 5861 },
}

describe('summarizeFindRelevant', () => {
  it('ranks rows relative to their shared folder and keeps the absolute path to open', () => {
    const summary = summarizeFindRelevant(input, output)!
    expect(summary.status).toBe('complete')
    expect(summary.scope).toBe('judge')
    expect(summary.rows[0]).toMatchObject({
      path: '/r/judge/src/register.rs',
      dir: 'src/',
      name: 'register.rs',
      rank: 0.96,
    })
    expect(summary.rows[3]).toMatchObject({ dir: '', name: 'README.md', rank: 0.7 })
    expect(firstLocation(summary.rows[0])).toEqual({ lineFrom: 35, lineTo: 52 })
    expect(firstLocation(summary.rows[1])).toBeNull()
  })

  it('renders the request alone in flight and refuses shapes that are not its own', () => {
    expect(summarizeFindRelevant(input, undefined)).toMatchObject({ status: null, rows: [] })
    expect(summarizeFindRelevant({ path: '.' }, undefined)).toBeNull()
    expect(summarizeFindRelevant(input, { results: [] })).toBeNull()
  })

  it('offers named leads, best first, and drops units named after their syntax kind', () => {
    const summary = summarizeFindRelevant(input, {
      ...output,
      files: [
        file('/r/a.rs', 0.9, {
          leads: [
            { name: 'use_declaration', line_from: 1, line_to: 1, score: 0.9 },
            { name: 'tests.use_declaration', line_from: 90, line_to: 90, score: 0.9 },
            { name: 'call', line_from: 40, line_to: 50, score: 0.4 },
            { name: 'resolve_provider', line_from: 10, line_to: 20, score: 0.8 },
          ],
        }),
      ],
    })!
    expect(namedLeads(summary.rows[0]).map((lead) => lead.name)).toEqual(['resolve_provider', 'call'])
  })

  it('finds the folder every path shares', () => {
    expect(sharedFolder(['/a/b/c.rs', '/a/b/d/e.rs'])).toBe('/a/b/')
    expect(sharedFolder(['/a/x.rs', '/b/y.rs'])).toBe('')
    expect(sharedFolder([])).toBe('')
  })

  it('numbers preview lines from the excerpt start and opens files at a range', () => {
    const lines = previewLines({ lineFrom: 35, lineTo: 52, text: 'a\nb\nc\n' }, 2)
    expect(lines).toEqual([
      { number: 35, text: 'a' },
      { number: 36, text: 'b' },
    ])
    expect(openFileRequest('/r/a.rs', 35, 52)).toEqual({
      pageId: 'ide',
      context: { type: 'file', path: '/r/a.rs', line: 35, endLine: 52 },
    })
    expect(openFileRequest('/r/a.rs')).toEqual({ pageId: 'ide', context: { type: 'file', path: '/r/a.rs' } })
  })
})

describe('FindRelevantCard', () => {
  it('shows the ranking, the best excerpt and the overflow accordion', () => {
    const summary = summarizeFindRelevant(input, output)!
    const html = renderToStaticMarkup(<FindRelevantCard summary={summary} running={false} onOpen={() => {}} />)
    expect(html).toContain('Found 6 relevant files')
    expect(html).toContain('44 judge calls · 5.9 s')
    expect(html).toContain('lines 35–52')
    expect(html).toContain('resolve_provider')
    expect(html).toContain('relevance 96%')
    expect(html).toContain('Show 1 more file')
    expect(html).toContain('inert=""')
  })

  it('says what went wrong for partial and unavailable asks', () => {
    const partial = summarizeFindRelevant(input, { ...output, status: 'incomplete', issues: { deadline: 2 } })!
    expect(renderToStaticMarkup(<FindRelevantCard summary={partial} running={false} />)).toContain(
      'Partial result (deadline ×2)',
    )
    const oversized = summarizeFindRelevant(input, {
      ...output,
      status: 'incomplete',
      reason: 'request_size',
      issues: { request_size: 1, source_inspection_limit: 1 },
    })!
    expect(renderToStaticMarkup(<FindRelevantCard summary={oversized} running={false} />)).toContain(
      'Partial result (oversized requests, files too large to inspect)',
    )
    const budget = summarizeFindRelevant(input, {
      ...output,
      status: 'incomplete',
      reason: 'token_budget',
      issues: { resource_limit: 1, token_budget: 1 },
    })!
    expect(renderToStaticMarkup(<FindRelevantCard summary={budget} running={false} />)).toContain(
      'Stopped at the judge token budget (size limit, judge token budget spent)',
    )
    const unavailable = summarizeFindRelevant(input, {
      ...output,
      status: 'unavailable',
      reason: 'judge window too small',
      files: [],
    })!
    const html = renderToStaticMarkup(<FindRelevantCard summary={unavailable} running={false} />)
    expect(html).toContain('Judge unavailable')
    expect(html).toContain('No judge answered (judge window too small)')
  })

  it("ends each note with the worker's hint and labels every issue kind", () => {
    const hint = 'Coverage is partial (judge_call_timeout): verify with coder::search. Narrow path.'
    const timedOut = summarizeFindRelevant(input, {
      ...output,
      status: 'incomplete',
      reason: 'judge_call_timeout',
      hint,
      issues: { judge_call_timeout: 2, invalid_response: 1 },
    })!
    const html = renderToStaticMarkup(<FindRelevantCard summary={timedOut} running={false} />)
    expect(html).toContain(`Partial result (judge calls timed out ×2, failed judge evaluations): `)
    expect(html).toContain(hint)
    expect(html).not.toContain('Narrow the folder for full coverage')
    // A reason with no issue counted still names the gap.
    const stopped = summarizeFindRelevant(input, { ...output, status: 'incomplete', reason: 'judge failed' })!
    expect(renderToStaticMarkup(<FindRelevantCard summary={stopped} running={false} />)).toContain(
      'Partial result (judge failed)',
    )
    const loading = summarizeFindRelevant(input, {
      ...output,
      status: 'unavailable',
      reason: 'judge model loading; retry shortly',
      hint: 'The judge is still loading its model: retry the ask in a minute.',
      files: [],
    })!
    const unavailable = renderToStaticMarkup(<FindRelevantCard summary={loading} running={false} />)
    expect(unavailable).toContain('retry the ask in a minute')
    expect(unavailable).not.toContain('still works')
    const empty = summarizeFindRelevant(input, { ...output, files: [], hint: 'Nothing under path looked relevant.' })!
    expect(renderToStaticMarkup(<FindRelevantCard summary={empty} running={false} />)).toContain(
      'Nothing under path looked relevant.',
    )
  })

  it('mounts at most twenty overflow rows and counts the rest', () => {
    const many = { ...output, files: Array.from({ length: 40 }, (_, i) => file(`/r/judge/src/f${i}.rs`, 1 - i / 100)) }
    const html = renderToStaticMarkup(
      <FindRelevantCard summary={summarizeFindRelevant(input, many)!} running={false} />,
    )
    expect(html).toContain('Found 40 relevant files')
    expect(html).toContain('Show 20 more files')
    expect(html).toContain('f24.rs')
    expect(html).not.toContain('f25.rs')
    expect(html).toContain('+15 lower-ranked files')
  })

  it('is a status region with the question while the judge works', () => {
    const summary = summarizeFindRelevant(input, undefined)!
    const html = renderToStaticMarkup(<FindRelevantCard summary={summary} running />)
    expect(html).toContain('data-state="running"')
    expect(html).toContain('Asking the judge')
    expect(html).toContain('how does the hub pick a provider?')
    expect(html).toContain('shui-relevant-progress')
  })
})
