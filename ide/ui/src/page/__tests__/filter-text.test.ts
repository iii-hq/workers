import { afterEach, describe, expect, it, vi } from 'vitest'
import { FilterText } from '../GitCommitList'
import { mount } from './bare-hooks'

vi.mock('react', async (original) => ({
  ...(await original<typeof import('react')>()),
  ...(await import('./bare-hooks')).hooks,
}))
// The real components are supplied by the Console import map, not Node.
vi.mock('@iii-dev/console-ui', () => ({ SearchField: () => null }))

describe("the Log's text filter", () => {
  afterEach(() => {
    vi.useRealTimers()
  })

  it('applies what was typed once typing settles, or as the field goes', () => {
    vi.useFakeTimers()
    const applied: string[] = []
    const onText = (text: string) => {
      applied.push(text)
    }
    const field = mount(FilterText, { text: '', onText })
    field.result.props.onChange('fix')
    expect(field.result.props.value).toBe('fix')
    vi.advanceTimersByTime(300)
    expect(applied).toEqual(['fix'])

    // More typed, then a row tapped at once in a narrow pane: it goes.
    field.rerender({ text: 'fix', onText })
    field.result.props.onChange('fix bug')
    field.unmount()
    expect(applied).toEqual(['fix', 'fix bug'])
  })

  it('drops what was waiting when the filter is cleared', () => {
    vi.useFakeTimers()
    const applied: string[] = []
    const onText = (text: string) => {
      applied.push(text)
    }
    const field = mount(FilterText, { text: 'nothing', onText })
    field.result.props.onChange('nothing here')
    // "Clear filters", with the typing not yet settled.
    field.rerender({ text: '', onText })
    expect(field.result.props.value).toBe('')
    vi.advanceTimersByTime(1_000)
    field.unmount()
    expect(applied).toEqual([])
  })

  it('drops what was waiting on Clear filters even when the text was already empty', () => {
    vi.useFakeTimers()
    const applied: string[] = []
    const onText = (text: string) => {
      applied.push(text)
    }
    // Only an author filter was set: the text is '' before and after.
    const field = mount(FilterText, { text: '', clears: 0, onText })
    field.result.props.onChange('typed')
    field.rerender({ text: '', clears: 1, onText })
    expect(field.result.props.value).toBe('')
    vi.advanceTimersByTime(1_000)
    field.unmount()
    expect(applied).toEqual([])
  })
})
