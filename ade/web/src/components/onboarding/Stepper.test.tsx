import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { Stepper, type StepperStep } from './Stepper'

const STEPS: StepperStep[] = [
  { id: 'welcome', title: 'Welcome' },
  { id: 'models', title: 'Models' },
  { id: 'browser', title: 'Browser', optional: true },
  { id: 'explore', title: 'Explore iii' },
]

const noop = () => undefined

describe('Stepper', () => {
  it('marks the current step and says which ones are done', () => {
    const html = renderToStaticMarkup(
      <Stepper
        steps={STEPS}
        current="browser"
        reachable={new Set(['welcome', 'models'])}
        onSelect={noop}
      />,
    )
    expect(html.match(/aria-current="step"/g)).toHaveLength(1)
    expect(html).toMatch(/Browser<[^>]*>optional<\/span><[^>]*>, current step/)
    expect(html).toMatch(/Welcome<[^>]*>, completed/)
    expect(html).toMatch(/Models<[^>]*>, completed/)
    expect(html).not.toMatch(/Explore iii<[^>]*>, completed/)
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
        current="explore"
        reachable={new Set()}
        onSelect={noop}
      />,
    )
    expect(html).not.toContain('<button')
  })

  // QA 2026-11-16 F7: on the last step every dot, the current one included,
  // showed a check. The current step is current, not done.
  it('marks the last step current, not done, while it is on screen', () => {
    const html = renderToStaticMarkup(
      <Stepper
        steps={STEPS}
        current="explore"
        reachable={new Set()}
        onSelect={noop}
      />,
    )
    expect(html).toMatch(
      /aria-current="step"[\s\S]*Explore iii<[^>]*>, current step/,
    )
    expect(html).not.toMatch(/Explore iii<[^>]*>, completed/)
    expect(html.match(/, completed/g)).toHaveLength(3)
  })

  // QA 2026-11-16 F7: the current step's inline title came and went with
  // the step, so every dot moved along the row; on a phone it cut "Judge"
  // to "J…". Every inline title folds alike, and a caption names the step.
  it('folds every horizontal title alike and names the current step under the dots', () => {
    for (const current of ['welcome', 'explore'] as const) {
      const html = renderToStaticMarkup(
        <Stepper
          steps={STEPS}
          current={current}
          reachable={new Set()}
          onSelect={noop}
        />,
      )
      expect(
        html.match(/sr-only @md:not-sr-only @md:flex/g) ?? [],
      ).toHaveLength(STEPS.length)
    }
    const html = renderToStaticMarkup(
      <Stepper
        steps={STEPS}
        current="browser"
        reachable={new Set()}
        onSelect={noop}
      />,
    )
    expect(html).toMatch(
      /data-stepper-caption[^>]*>Step 3 of 4 · <span[^>]*>Browser<\/span>/,
    )
  })

  it('keeps every horizontal step named for screen readers when labels fold', () => {
    const html = renderToStaticMarkup(
      <Stepper
        steps={STEPS}
        current="browser"
        reachable={new Set(['welcome', 'models'])}
        onSelect={noop}
      />,
    )
    // Folded below @md visually only: never display:none.
    expect(html).not.toMatch(/class="[^"]*\bhidden\b[^"]*@md:flex/)
    expect(html).toContain('sr-only @md:not-sr-only @md:flex')
    expect(html).toMatch(/Welcome<[^>]*>, completed/)
  })
})
