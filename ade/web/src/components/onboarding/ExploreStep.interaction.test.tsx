// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import type { ExamplePrompt } from '@/lib/onboarding/prompts'
import { ExploreStep } from './ExploreStep'

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
})
