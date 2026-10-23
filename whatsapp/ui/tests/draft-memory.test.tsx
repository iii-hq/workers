// @vitest-environment jsdom
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { DraftProvider, useDraftMemory, useDraftState, usePageState } from '../src/drafts'

vi.mock('@iii-dev/console-ui', () => ({ Button: () => null, Dialog: () => null, DialogContent: () => null, DialogDescription: () => null, DialogTitle: () => null }))

;(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true
const cleanups: Array<() => void> = []

function Probe({ account }: { account: string }) {
  const [draft, setDraft, clearDraft] = useDraftState(`${account}:draft`, '', (value) => !!value)
  const [result, setResult] = usePageState<{ id: string; incoming: Set<string> } | null>(`${account}:test`, null)
  const memory = useDraftMemory()
  return <>
    <output data-draft>{draft}</output><output data-result>{result?.id ?? 'No test'}{result?.incoming.has('prior') ? ' · Baseline kept' : ''}</output>
    <output data-dirty>{memory?.values.size ?? 0}</output>
    <button onClick={() => setDraft('Unsent message')}>Write draft</button>
    <button onClick={clearDraft}>Discard draft</button>
    <button onClick={() => setResult({ id: 'accepted-id', incoming: new Set(['prior']) })}>Record test</button>
    <button onClick={() => memory?.clearPrefix(`${account}:`)}>Remove account</button>
  </>
}

async function render() {
  let account = 'native:session:1', visible = true
  const container = document.createElement('div'); document.body.append(container)
  const root = createRoot(container)
  const show = () => root.render(<DraftProvider>{visible && <Probe key={account} account={account} />}</DraftProvider>)
  await act(async () => show())
  cleanups.push(() => { act(() => root.unmount()); container.remove() })
  return {
    container,
    async click(label: string) {
      const button = Array.from(container.querySelectorAll('button')).find((item) => item.textContent === label)!
      await act(async () => button.click())
    },
    async account(next: string) { account = next; await act(async () => show()) },
    async reopenTool() { visible = false; await act(async () => show()); visible = true; await act(async () => show()) },
    async reopenPage() { await act(async () => root.render(null)); await act(async () => show()) },
  }
}
afterEach(() => { for (const cleanup of cleanups.splice(0)) cleanup() })

describe('Page-only draft and test memory', () => {
  it('restores scoped test evidence including its Set without marking it as an unsent draft', async () => {
    const view = await render()
    await view.click('Record test')
    expect(view.container.querySelector('[data-dirty]')?.textContent).toBe('0')
    await view.account('native:session:2')
    expect(view.container.querySelector('[data-result]')?.textContent).toBe('No test')
    await view.account('native:session:1')
    expect(view.container.querySelector('[data-result]')?.textContent).toBe('accepted-id · Baseline kept')
    await view.click('Write draft')
    await view.click('Discard draft')
    expect(view.container.querySelector('[data-draft]')?.textContent).toBe('')
    expect(view.container.querySelector('[data-result]')?.textContent).toBe('accepted-id · Baseline kept')
    expect(view.container.querySelector('[data-dirty]')?.textContent).toBe('0')
  })

  it('purges both records and drafts for a removed account and drops all memory when the page closes', async () => {
    const view = await render()
    await view.click('Write draft'); await view.click('Record test')
    await view.click('Remove account'); await view.reopenTool()
    expect(view.container.querySelector('[data-draft]')?.textContent).toBe('')
    expect(view.container.querySelector('[data-result]')?.textContent).toBe('No test')
    await view.click('Record test'); await view.reopenPage()
    expect(view.container.querySelector('[data-result]')?.textContent).toBe('No test')
    expect(view.container.querySelector('[data-dirty]')?.textContent).toBe('0')
  })
})
