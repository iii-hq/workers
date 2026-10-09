import type { ReactElement } from 'react'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import type { FindRelevantResponse } from '../coder'
import { coderFindRelevant, coderSearch } from '../coder'
import { askFolder, askNotice, askRefusal, SearchTab } from '../SearchTab'
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
  stats: { judge_calls: 3, cache_hits: 0 },
  ...out,
})

describe('askFolder', () => {
  it('takes one folder inside the root, as a name or a dir/**, and nothing else', () => {
    expect(askFolder('')).toBe('')
    expect(askFolder('./')).toBe('')
    expect(askFolder('.')).toBe('')
    expect(askFolder(' judge/src/** ')).toBe('judge/src')
    expect(askFolder('/judge/**')).toBe('judge')
    for (const plain of ['src', 'src/', './src', '/src/']) expect(askFolder(plain)).toBe('src')
    expect(askFolder('.github')).toBe('.github')
    // never above or beside the root
    expect(askFolder('../other/**')).toBeNull()
    expect(askFolder('..')).toBeNull()
    expect(askFolder('src/../../x')).toBeNull()
    expect(askFolder('src/./lib')).toBeNull()
    expect(askFolder('**')).toBeNull()
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

  it('re-attaches Enter on the same question to an ask whose answer was dropped', async () => {
    const tab = render()
    tab.toggleAsk()
    tab.type('how are judge slots limited?')
    tab.enter()
    // Leaving and re-entering ask mode drops the answer, not the worker's run.
    tab.toggleAsk()
    tab.toggleAsk()
    expect(tab.page()).not.toContain('asking the judge')
    tab.enter()
    expect(coderFindRelevant).toHaveBeenCalledTimes(1)
    expect(tab.page()).not.toContain('An earlier ask is still running')
    expect(tab.page()).toContain('asking the judge…')

    asks[0](answer({}))
    await settle()
    expect(tab.page()).toContain('1 result in 1 file')
    tab.enter()
    expect(coderFindRelevant).toHaveBeenCalledTimes(2)
  })

  it('waits for a dropped ask before a different question, then clears the wait note', async () => {
    const tab = render()
    tab.toggleAsk()
    tab.type('how are judge slots limited?')
    tab.enter()
    tab.type('')
    tab.type('where are secrets redacted?')
    tab.enter()
    expect(coderFindRelevant).toHaveBeenCalledTimes(1)
    expect(tab.page()).toContain('An earlier ask is still running')

    asks[0](answer({}))
    await settle()
    expect(tab.page()).not.toContain('An earlier ask is still running')
    expect(tab.page()).not.toContain('1 result')
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
      root: '/repo',
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
    const folder = (value: string) => tab.find((p) => p.placeholder === 'e.g. src').onChange({ target: { value } })
    folder('lib/**')
    expect(tab.page()).toContain('Results are for an earlier folder or exclusions — press Enter to ask again.')
    folder('')
    expect(tab.page()).not.toContain('Results ')
    // not one folder: it asks the root, like an empty field
    folder('*.ts')
    expect(tab.page()).not.toContain('Results ')
    folder('')
    tab.find((p) => p.placeholder === 'e.g. *.test.ts, dist/**').onChange({ target: { value: 'gen/**' } })
    expect(tab.page()).toContain('Results are for an earlier folder or exclusions')
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
    expect(tab.page()).toContain(
      'Partial results (judge token budget spent) — the answer may be in files not listed. Narrow the folder.',
    )
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

    // nothing eligible: the judge never saw the folder
    tab.type('where is the cache evicted?')
    tab.enter()
    asks[3](answer({ files: [], stats: { judge_calls: 0, cache_hits: 0 } }))
    await settle()
    expect(tab.find((p) => p.title === 'No results').description).toBe(
      'Nothing in this folder could be judged: it is empty, or every file is hidden, ignored or excluded. Check the folder and the exclusions.',
    )
  })

  it('tells how to fix a judge that cannot work, not to wait', () => {
    expect(askNotice('unavailable', 'missing_key')).toBe(
      'Judge unavailable (no API key is set for the judge provider). Set the provider key in the judge settings. Text search still works.',
    )
    expect(askNotice('unavailable', 'not registered')).toContain('Start the judge worker or pick another judge.')
    expect(askNotice('unavailable', 'invalid_request')).toBe(
      'Judge unavailable (the judge rejected the request). Text search still works.',
    )
    expect(askNotice('unavailable', 'http')).toContain('Ask again later.')
  })

  it('compares a question with surrounding spaces as asked', async () => {
    const tab = render()
    tab.toggleAsk()
    tab.type(' how are judge slots limited? ')
    tab.enter()
    asks[0](answer({}))
    await settle()
    tab.find((p) => p['aria-label'] === 'Toggle search details').onClick()
    tab.find((p) => p.placeholder === 'e.g. src').onChange({ target: { value: 'lib/**' } })
    expect(tab.page()).toContain('Results are for an earlier folder or exclusions — press Enter to ask again.')
  })

  it('never lets an older text search overwrite a newer one across a re-attach', async () => {
    const searches: Array<{ query: string; land: (out: unknown) => void }> = []
    const pending: typeof coderSearch = (_host, args) =>
      new Promise((land) => searches.push({ query: args.query, land })) as never
    vi.mocked(coderSearch).mockImplementationOnce(pending).mockImplementationOnce(pending)
    const hit = (path: string) => ({
      content_matches: [{ path, line: 1, column: 1, text: 'x' }],
      path_matches: [],
      truncated: false,
    })
    const debounce = () => new Promise((resolve) => setTimeout(resolve, 260))
    const shown = () =>
      (
        (tab.find((p) => typeof p.renderRow === 'function').rows as unknown as Array<{
          type: string
          group?: { rel: string }
        }>) ?? []
      )
        .filter((row) => row.type === 'file')
        .map((row) => row.group?.rel)
    const tab = render()
    tab.toggleAsk()
    tab.type('needle')
    tab.enter()
    tab.toggleAsk()
    await debounce()
    // the same ask again: Enter re-attaches while text search A runs
    tab.toggleAsk()
    tab.enter()
    tab.type('other')
    tab.toggleAsk()
    await debounce()
    expect(searches.map((search) => search.query)).toEqual(['needle', 'other'])
    searches[1].land(hit('/repo/other.rs'))
    await settle()
    searches[0].land(hit('/repo/needle.rs'))
    await settle()
    expect(shown()).toEqual(['other.rs'])
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
      'Judge unavailable (the judge did not list its models in time). Ask again in a minute. Text search still works.',
    )
    expect(tab.page()).not.toContain('coder::search')
  })

  it('says a next step that fits why the answer is partial', () => {
    expect(askNotice('incomplete', 'changed')).toBe(
      'Partial results (files changed meanwhile) — the answer may be in files not listed. Ask again once files stop changing. Check with text search.',
    )
    expect(askNotice('incomplete', 'judge_call_timeout')).toContain('(judge calls timed out)')
    expect(askNotice('incomplete', 'invalid_response')).toContain('Ask again later.')
    expect(askNotice('incomplete', 'unreadable')).toBe(
      'Partial results (unreadable files or folders) — the answer may be in files not listed. Check with text search.',
    )
  })

  it("words the worker's refusals for this view, not for an agent", async () => {
    const refusal =
      'handler error: {"code":"C210","message":"find-relevant sends file text to the judge, so it only searches a project folder (the session folder, a Git work tree, a granted folder or a jailed worker\'s root, see coder::info), and /repo is none; use coder::search"}'
    expect(askRefusal(refusal)).toBe('Ask only searches a project folder — use text search here.')
    expect(askRefusal('path is gitignored or inside an ignored folder, which find-relevant never searches')).toBe(
      'Ask never searches folders Git ignores — use text search here.',
    )
    expect(askRefusal('transport timeout')).toBe('transport timeout')
    expect(
      askRefusal(
        'handler error: {"code":"C211","message":"/repo/scr: not found or not accessible. Verify the path with coder::list-folder or coder::tree. Folders beside it, closest name first: /repo/src, /repo/scripts."}',
        '/repo/',
      ),
    ).toBe('No such folder in this workspace — did you mean src, scripts?')
    expect(askRefusal('/repo/x: not found or not accessible. Verify the path with coder::tree.')).toBe(
      'No such folder in this workspace — check the folder to ask about.',
    )
    expect(askRefusal('query is 4100 bytes; at most 4000: retry with the question alone')).toContain(
      'The question is too long to ask',
    )
    expect(
      askRefusal('this session is scoped to /repo; /etc is inside an allowed root but outside the session directory'),
    ).toBe('That folder leads outside this workspace — ask about one inside it.')

    vi.mocked(coderFindRelevant).mockImplementationOnce(() => Promise.reject(new Error(refusal)))
    const tab = render()
    tab.toggleAsk()
    tab.type('how are judge slots limited?')
    tab.enter()
    await settle()
    expect(tab.page()).toContain('Ask only searches a project folder — use text search here.')
    expect(tab.page()).not.toContain('coder::')
  })

  it('says the results were dismissed, not that the judge found nothing', async () => {
    const tab = render()
    tab.toggleAsk()
    tab.type('how are judge slots limited?')
    tab.enter()
    asks[0](answer({}))
    await settle()
    const list = tab.find((p) => typeof p.renderRow === 'function')
    const rows = list.rows as unknown as Array<{ type: string }>
    const index = rows.findIndex((row) => row.type === 'file')
    const dismiss = elements(list.renderRow(rows[index], index)).find((element) =>
      String(element.props?.['aria-label']).startsWith('Dismiss'),
    )
    ;(dismiss?.props?.onClick as (event: unknown) => void)({ stopPropagation() {} })
    expect(tab.find((p) => p.title === 'No results').description).toBe(
      'All results dismissed — search again to restore them.',
    )
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
