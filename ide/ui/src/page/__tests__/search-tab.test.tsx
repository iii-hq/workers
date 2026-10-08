import type { ReactElement } from 'react'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import type { FindRelevantResponse } from '../coder'
import { coderFindRelevant } from '../coder'
import { askFolder, SearchTab } from '../SearchTab'
import { mount } from './bare-hooks'

vi.mock('react', async (original) => ({
  ...(await original<typeof import('react')>()),
  ...(await import('./bare-hooks')).hooks,
}))

// The real components are supplied by the Console import map, not Node.
vi.mock('@iii-dev/console-ui', () => {
  const Pass = () => null
  return { EmptyState: Pass, IconButton: Pass, SearchField: Pass }
})
vi.mock('@iii-dev/console-ui/format', () => ({ errorMessage: String }))
vi.mock('../file-type-icon', () => ({ FileTypeIcon: () => null }))
vi.mock('../VirtualList', () => ({ VirtualList: () => null }))

// Each ask's answer, settled by the test.
const asks = vi.hoisted(() => [] as Array<(out: FindRelevantResponse) => void>)
vi.mock('../coder', () => ({
  coderSearch: vi.fn(() => new Promise(() => {})),
  coderFindRelevant: vi.fn(() => new Promise((resolve) => asks.push(resolve))),
}))

beforeAll(() => {
  vi.stubGlobal('window', {
    setTimeout,
    clearTimeout,
    setInterval,
    clearInterval,
    requestAnimationFrame: () => 0,
  })
})
afterEach(() => {
  vi.clearAllMocks()
  asks.length = 0
})

type Props = Parameters<typeof SearchTab>[0]
type Element = { type?: unknown; props?: Record<string, unknown> }

/** Every element under `node`, including those passed as props (`actions`). */
function elements(node: unknown, out: Element[] = []): Element[] {
  if (Array.isArray(node)) {
    for (const child of node) elements(child, out)
  } else if (node !== null && typeof node === 'object' && 'props' in node) {
    const element = node as Element
    out.push(element)
    for (const value of Object.values(element.props ?? {})) elements(value, out)
  }
  return out
}

/** The text a subtree renders, nested elements included. */
function text(node: unknown): string {
  if (typeof node === 'string' || typeof node === 'number') return String(node)
  if (Array.isArray(node)) return node.map(text).join('')
  if (node !== null && typeof node === 'object' && 'props' in node) return text((node as Element).props?.children)
  return ''
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 0))

function render(request: Props['request'] = null) {
  const props: Props = {
    hidden: false,
    host: {} as Props['host'],
    root: '/repo',
    request,
    onOpenMatch: () => {},
    onPreviewFile: () => {},
    onPinFile: () => {},
    onRevealFolder: () => {},
  }
  const view = mount(
    (SearchTab as unknown as { type: (props: Props) => ReactElement<{ hidden: boolean }> }).type,
    props,
  )
  const all = () => elements(view.result)
  const find = (test: (props: Record<string, unknown>) => boolean) => {
    const hit = all().find((element) => test(element.props ?? {}))
    if (!hit) throw new Error('no such element')
    return hit.props as Record<string, (...args: unknown[]) => void> & Record<string, unknown>
  }
  const field = () => find((p) => p['aria-label'] === 'Search query')
  return {
    view,
    find,
    update: (next: Partial<Props>) => view.rerender({ ...props, ...next }),
    page: () => text(view.result),
    type: (value: string) => field().onChange(value),
    enter: () => field().onKeyDown({ key: 'Enter', nativeEvent: { isComposing: false }, preventDefault() {} }),
    toggleAsk: () => find((p) => p.label === 'Ask the judge').onToggle(),
    refresh: () => find((p) => p.label === 'Refresh'),
  }
}

const answer = (out: Partial<FindRelevantResponse>): FindRelevantResponse => ({
  status: 'complete',
  files: [
    {
      path: '/repo/judge/src/slots.rs',
      score: 0.9,
      roles: [],
      excerpts: [{ line_from: 3, line_to: 4, text: 'fn acquire() {}' }],
      leads: [],
      source_omitted: false,
    },
  ],
  ...out,
})

describe('askFolder', () => {
  it('takes the folder of a single dir/** and nothing else', () => {
    expect(askFolder('')).toBe('')
    expect(askFolder(' judge/src/** ')).toBe('judge/src')
    expect(askFolder('/judge/**')).toBe('judge')
    expect(askFolder('*.ts')).toBeNull()
    expect(askFolder('a/**, b/**')).toBeNull()
    expect(askFolder('src/*.rs')).toBeNull()
    // bracketed and grouped folder names are literal folders
    expect(askFolder('app/[slug]/**')).toBe('app/[slug]')
    expect(askFolder('app/(group)/[id]/**')).toBe('app/(group)/[id]')
    expect(askFolder('{a,b}/**')).toBeNull()
  })
})

describe('the Search tab in ask mode', () => {
  it('runs one ask at a time and keeps Refresh off while it runs', async () => {
    const tab = render()
    tab.toggleAsk()
    tab.type('how are judge slots limited?')
    tab.enter()
    tab.enter()
    expect(coderFindRelevant).toHaveBeenCalledTimes(1)
    expect(tab.refresh().disabled).toBe(true)
    expect(tab.page()).not.toContain('earlier ask')

    asks[0](answer({}))
    await settle()
    expect(tab.refresh().disabled).toBe(false)
    tab.enter()
    expect(coderFindRelevant).toHaveBeenCalledTimes(2)
  })

  it('says a new question waits while its own ask runs', async () => {
    const tab = render()
    tab.toggleAsk()
    tab.type('how are judge slots limited?')
    tab.enter()
    tab.type('where are secrets redacted?')
    tab.enter()
    expect(coderFindRelevant).toHaveBeenCalledTimes(1)
    expect(tab.page()).toContain('An earlier ask is still running')

    asks[0](answer({}))
    await settle()
    expect(tab.page()).not.toContain('An earlier ask is still running')
    expect(tab.page()).toContain('Results for “how are judge slots limited?”')
  })

  it('waits for an ask whose answer was dropped, then clears the wait note', async () => {
    const tab = render()
    tab.toggleAsk()
    tab.type('how are judge slots limited?')
    tab.enter()
    // Leaving and re-entering ask mode drops the answer, not the worker's run.
    tab.toggleAsk()
    tab.toggleAsk()
    tab.enter()
    expect(coderFindRelevant).toHaveBeenCalledTimes(1)
    expect(tab.page()).toContain('An earlier ask is still running')

    asks[0](answer({}))
    await settle()
    expect(tab.page()).not.toContain('An earlier ask is still running')
    tab.enter()
    expect(coderFindRelevant).toHaveBeenCalledTimes(2)
  })

  it('asks about the "Find in folder" folder with the exclude globs and hides the Git toggle', () => {
    const tab = render({ seq: 1, includeGlob: 'judge/src/**' })
    tab.toggleAsk()
    tab.find((p) => p.placeholder === 'e.g. *.test.ts, dist/**').onChange({ target: { value: 'gen/**, *.md' } })
    tab.type('how are judge slots limited?')
    tab.enter()
    expect(coderFindRelevant).toHaveBeenCalledWith(expect.anything(), {
      query: 'how are judge slots limited?',
      path: '/repo/judge/src',
      excludeGlobs: ['**/gen/**', '**/*.md'],
      timeoutMs: 240_000,
    })
    expect(tab.page()).toContain('folder to ask about')
    expect(tab.page()).not.toContain('skip files ignored by Git')
  })

  it('says when the include field is not one folder', () => {
    const tab = render({ seq: 1, includeGlob: '*.rs' })
    tab.toggleAsk()
    tab.type('how are judge slots limited?')
    tab.enter()
    expect(coderFindRelevant).toHaveBeenCalledWith(expect.anything(), expect.objectContaining({ path: '/repo' }))
    expect(tab.page()).toContain('An ask takes one folder')
  })

  it('marks the answer stale once the question, folder or exclusions change', async () => {
    const tab = render()
    tab.toggleAsk()
    tab.type('how are judge slots limited?')
    tab.enter()
    asks[0](answer({}))
    await settle()
    expect(tab.page()).not.toContain('Results for')

    tab.find((p) => p['aria-label'] === 'Toggle search details').onClick()
    tab.find((p) => p.placeholder === 'e.g. src/**').onChange({ target: { value: 'lib/**' } })
    expect(tab.page()).toContain('Results for “how are judge slots limited?” — press Enter to ask again.')
    tab.find((p) => p.placeholder === 'e.g. src/**').onChange({ target: { value: '' } })
    expect(tab.page()).not.toContain('Results for')
    tab.find((p) => p.placeholder === 'e.g. *.test.ts, dist/**').onChange({ target: { value: 'gen/**' } })
    expect(tab.page()).toContain('Results for')
    tab.find((p) => p.placeholder === 'e.g. *.test.ts, dist/**').onChange({ target: { value: '' } })
    tab.type('where are secrets redacted?')
    expect(tab.page()).toContain('Results for “how are judge slots limited?” — press Enter to ask again.')
    expect(coderFindRelevant).toHaveBeenCalledTimes(1)
  })

  it('words partial and empty answers for this view, once each, not with the agent hint', async () => {
    const hint =
      'Coverage is partial (token_budget): the answer may be in files not listed, so verify with coder::search before relying on this list. Narrow path for fuller coverage.'
    const tab = render()
    tab.toggleAsk()
    tab.type('how are judge slots limited?')
    tab.enter()
    asks[0](answer({ status: 'incomplete', reason: 'token_budget', hint }))
    await settle()
    expect(tab.page()).toContain('Partial results (token_budget) — the answer may be in files not listed.')
    expect(tab.page()).not.toContain('coder::search')

    // no rows: the empty state carries the notice, no banner repeats it
    tab.enter()
    tab.type('where are secrets redacted?')
    tab.enter()
    asks[1](answer({ status: 'incomplete', reason: 'deadline', hint, files: [] }))
    await settle()
    expect(tab.page()).not.toContain('Partial results')
    expect(tab.find((p) => p.title === 'No results').description).toContain('Partial results (deadline)')

    tab.type('where is the cache keyed?')
    tab.enter()
    asks[2](
      answer({
        files: [],
        hint: 'Nothing under path looked relevant to the judge: widen path, or use coder::search for exact names.',
      }),
    )
    await settle()
    expect(tab.find((p) => p.title === 'No results').description).toBe(
      'The judge found nothing relevant — widen the folder or use text search.',
    )
  })

  it('reports an unavailable judge with its reason in its own words', async () => {
    const tab = render()
    tab.toggleAsk()
    tab.type('how are judge slots limited?')
    tab.enter()
    asks[0](
      answer({
        status: 'unavailable',
        reason: 'listing_timeout',
        hint: 'The judge did not list its models in time (a local judge may still be loading its model): retry the ask in a minute, or use coder::search now.',
        files: [],
      }),
    )
    await settle()
    expect(tab.page()).toContain(
      'Judge unavailable (the judge did not list its models in time) — use text search, or ask again later.',
    )
    expect(tab.page()).not.toContain('coder::search')
  })
})

describe('the Search tab while hidden', () => {
  it('keeps its ask and lands the answer', async () => {
    const tab = render()
    tab.toggleAsk()
    tab.type('how are judge slots limited?')
    tab.enter()
    tab.update({ hidden: true })
    expect(tab.view.result.props.hidden).toBe(true)
    asks[0](answer({}))
    await settle()
    tab.update({ hidden: false })
    expect(tab.view.result.props.hidden).toBe(false)
    expect(tab.page()).toContain('1 result in 1 file')
  })

  it('takes no pane focus while hidden', () => {
    const tab = render()
    const input = { setAttribute: vi.fn(), toggleAttribute: vi.fn(), focus() {}, select() {} }
    ;(tab.find((p) => p['aria-label'] === 'Search query').ref as unknown as { current: unknown }).current = input
    tab.update({ hidden: true })
    expect(input.toggleAttribute).toHaveBeenLastCalledWith('data-autofocus', false)
    tab.update({ hidden: false })
    expect(input.toggleAttribute).toHaveBeenLastCalledWith('data-autofocus', true)
  })
})
