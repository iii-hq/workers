// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import type { ExamplePrompt } from '@/lib/onboarding/prompts'
import { ExampleList, ExploreStep, type TourState } from './ExploreStep'

const PROMPTS: ExamplePrompt[] = [
  'Build a link shortener',
  'Build an expense tracker',
].map((title) => ({
  title,
  description: `${title}, described`,
  agent: 'ade-worker-builder',
  prompt: `${title}.`,
  models: [{ provider: 'claude-code', model: 'claude-sonnet-5-5' }],
}))

let container: HTMLDivElement
let root: Root

beforeEach(() => {
  ;(
    globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
  ).IS_REACT_ACT_ENVIRONMENT = true
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
})

afterEach(() => {
  act(() => root.unmount())
  container.remove()
})

const button = (label: RegExp) => {
  const found = [...container.querySelectorAll('button')].find((entry) =>
    label.test(entry.textContent ?? ''),
  )
  if (!found) throw new Error(`no button ${label}`)
  return found
}

/** The pick group moves the focus two animation frames on. */
const frames = () =>
  act(() => new Promise<void>((resolve) => setTimeout(resolve, 50)))

describe('ExploreStep focus', () => {
  it('puts the focus back on the picked row when Back leaves the examples', async () => {
    const noop = () => undefined
    act(() =>
      root.render(
        <ExploreStep
          connected
          prompts={PROMPTS}
          onPrompt={noop}
          tour={{ kind: 'idle' }}
          onTour={noop}
          onOwn={noop}
          onFinish={noop}
          onBack={noop}
        />,
      ),
    )
    await frames()
    act(() => button(/Build an example project/).click())
    act(() => button(/Choose a project/).click())
    await frames()
    act(() => button(/^Back$/).click())
    await frames()
    expect(document.activeElement).toBe(button(/Build an example project/))
  })

  it('leaves the focus where it is when the prompts load late', async () => {
    const noop = () => undefined
    const list = (prompts: readonly ExamplePrompt[] | null) => (
      <>
        <button type="button">Back</button>
        <ExampleList prompts={prompts} picked={null} onPick={noop} />
      </>
    )
    act(() => root.render(list(null)))
    await frames()
    act(() => button(/^Back$/).focus())
    act(() => root.render(list(PROMPTS)))
    await frames()
    expect(document.activeElement).toBe(button(/^Back$/))
  })

  it('ticks Explore on my own on arrival and leaves the focus to the primary button', async () => {
    const noop = () => undefined
    act(() =>
      root.render(
        <ExploreStep
          connected
          prompts={PROMPTS}
          onPrompt={noop}
          tour={{ kind: 'idle' }}
          onTour={noop}
          onOwn={noop}
          onFinish={noop}
          onBack={noop}
        />,
      ),
    )
    await frames()
    const own = button(/Explore on my own/)
    expect(own.getAttribute('aria-checked')).toBe('true')
    expect(button(/Take the learn-iii tour/).getAttribute('aria-checked')).toBe(
      'false',
    )
    // The ticked row does not pull the focus in: setup's own move to the
    // footer button stands, so Enter closes setup into a chat.
    expect(document.activeElement).not.toBe(own)
    expect(
      (document.activeElement as HTMLElement).getAttribute('role'),
    ).not.toBe('radiogroup')
  })

  it('moves the pick from the ticked row with the arrow keys', async () => {
    const noop = () => undefined
    act(() =>
      root.render(
        <ExploreStep
          connected
          prompts={PROMPTS}
          onPrompt={noop}
          tour={{ kind: 'idle' }}
          onTour={noop}
          onOwn={noop}
          onFinish={noop}
          onBack={noop}
        />,
      ),
    )
    await frames()
    const own = button(/Explore on my own/)
    act(() => own.focus())
    act(() => {
      own.dispatchEvent(
        new KeyboardEvent('keydown', { key: 'ArrowUp', bubbles: true }),
      )
    })
    const tour = button(/Take the learn-iii tour/)
    expect(document.activeElement).toBe(tour)
    expect(tour.getAttribute('aria-checked')).toBe('true')
  })

  // QA 2026-11-16 F5: with nothing picked the button read Finish beside a
  // forward arrow and closed setup without a word. It names what it does,
  // and a pick renames it.
  it('names what the footer button does, before and after a pick', async () => {
    const closed: string[] = []
    act(() =>
      root.render(
        <ExploreStep
          connected
          prompts={PROMPTS}
          onPrompt={() => undefined}
          tour={{ kind: 'idle' }}
          onTour={() => closed.push('tour')}
          onOwn={() => closed.push('own')}
          onFinish={() => closed.push('finish')}
          onBack={() => undefined}
        />,
      ),
    )
    await frames()
    const footer = () =>
      [...container.querySelectorAll('footer button')].at(-1) as HTMLElement
    // Explore on my own starts out ticked, so the button closes setup into
    // a chat; picking the tour renames it.
    expect(footer().textContent).toBe('Explore on my own')
    act(() => button(/Take the learn-iii tour/).click())
    expect(footer().textContent).toBe('Start the tour')
    act(() => button(/Explore on my own/).click())
    expect(footer().textContent).toBe('Explore on my own')
    act(() => footer().click())
    expect(closed).toEqual(['own'])
  })

  it('keeps the tour ticked while it gets ready', async () => {
    const noop = () => undefined
    const render = (tour: TourState) =>
      act(() =>
        root.render(
          <ExploreStep
            connected
            prompts={PROMPTS}
            onPrompt={noop}
            tour={tour}
            onTour={noop}
            onOwn={noop}
            onFinish={noop}
            onBack={noop}
          />,
        ),
      )
    render({ kind: 'idle' })
    await frames()
    act(() => button(/Take the learn-iii tour/).click())
    render({ kind: 'preparing', adding: 'onboarding' })
    const tour = button(/Take the learn-iii tour/)
    expect(tour.getAttribute('aria-checked')).toBe('true')
    expect(tour.disabled).toBe(true)
  })
})
