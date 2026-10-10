import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { Stepper, type StepperStep } from './Stepper'

const STEPS: StepperStep[] = [
  { id: 'welcome', title: 'Welcome' },
  { id: 'models', title: 'Models' },
  { id: 'judge', title: 'Judge', optional: true },
  { id: 'ready', title: 'Ready' },
]

const noop = () => undefined

describe('Stepper', () => {
  it('marks the current step and says which ones are done', () => {
    const html = renderToStaticMarkup(
      <Stepper
        steps={STEPS}
        current="judge"
        reachable={new Set(['welcome', 'models'])}
        onSelect={noop}
      />,
    )
    expect(html.match(/aria-current="step"/g)).toHaveLength(1)
    expect(html).toMatch(/Judge<[^>]*>optional<\/span><[^>]*>, current step/)
    expect(html).toMatch(/Welcome<[^>]*>, completed/)
    expect(html).toMatch(/Models<[^>]*>, completed/)
    expect(html).not.toMatch(/Ready<[^>]*>, completed/)
  })

  it('only makes a button of a step the person can go back to', () => {
    const html = renderToStaticMarkup(
      <Stepper
        steps={STEPS}
        current="models"
        reachable={new Set(['welcome'])}
        onSelect={noop}
      />,
    )
    // One button: Welcome. The current and upcoming steps are plain text.
    expect(html.match(/<button/g)).toHaveLength(1)
    expect(html).not.toContain('disabled')
  })

  it('offers no way back once setup is recorded', () => {
    const html = renderToStaticMarkup(
      <Stepper
        steps={STEPS}
        current="ready"
        reachable={new Set()}
        onSelect={noop}
      />,
    )
    expect(html).not.toContain('<button')
    // Arriving at the last step completes it: it is current and done.
    expect(html).toMatch(/aria-current="step"[\s\S]*Ready<[^>]*>, completed/)
    expect(html.match(/, completed/g)).toHaveLength(4)
  })
})
