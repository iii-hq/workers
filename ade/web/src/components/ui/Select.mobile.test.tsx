// @vitest-environment jsdom

import { act, type ComponentProps } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { BottomSheet, BottomSheetContent } from './BottomSheet'
import { Select } from './Select'

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

const options = [
  { value: 'one', label: 'Option one' },
  { value: 'two', label: 'Option two' },
  { value: 'disabled', label: 'Disabled option', disabled: true },
]

const mounted: Array<() => void> = []

beforeEach(() => {
  vi.stubGlobal('matchMedia', (query: string) => ({
    matches:
      query === '(max-width: 767px)' ||
      query === '(prefers-reduced-motion: reduce)',
    addEventListener: () => {},
    removeEventListener: () => {},
  }))
})

afterEach(() => {
  for (const unmount of mounted.splice(0)) unmount()
  vi.unstubAllGlobals()
})

async function mountSelect(
  props: Partial<ComponentProps<typeof Select>> = {},
  embedded = false,
) {
  const container = document.createElement('div')
  document.body.appendChild(container)
  const root = createRoot(container)
  const onChange = vi.fn()
  const select = (
    <Select
      aria-label="Example"
      sheetDescription="Choose an option."
      value="one"
      options={options}
      onChange={onChange}
      {...props}
    />
  )
  await act(async () => {
    root.render(
      embedded ? (
        <BottomSheet open>
          <BottomSheetContent
            heading="Settings"
            description="Edit your settings."
          >
            {select}
          </BottomSheetContent>
        </BottomSheet>
      ) : (
        select
      ),
    )
  })
  mounted.push(() => {
    act(() => root.unmount())
    container.remove()
  })
  const trigger = document.querySelector<HTMLButtonElement>(
    'button[aria-label="Example"]',
  )!
  await act(async () => {
    trigger.focus()
    trigger.click()
  })
  expect(trigger.getAttribute('aria-expanded')).toBe('true')
  return { trigger, onChange }
}

function radio(value: string) {
  const input = Array.from(
    document.querySelectorAll<HTMLInputElement>('input[type="radio"]'),
  ).find((input) => input.value === value)
  expect(input).toBeDefined()
  return input!
}

async function click(element: HTMLElement) {
  await act(async () => element.click())
}

describe('Select mobile option activation', () => {
  it.each(['radio', 'label'])(
    'closes when the current %s is activated without reporting a value change',
    async (target) => {
      const { trigger, onChange } = await mountSelect()
      const current = radio('one')
      expect(current.checked).toBe(true)

      await click(target === 'radio' ? current : current.closest('label')!)

      expect(trigger.getAttribute('aria-expanded')).toBe('false')
      expect(document.querySelector('[role="dialog"]')).toBeNull()
      // Radix restores focus after its focus scope has unmounted.
      await act(async () => {
        await new Promise<void>((resolve) => window.setTimeout(resolve, 0))
      })
      expect(document.activeElement).toBe(trigger)
      expect(onChange).not.toHaveBeenCalled()
    },
  )

  it.each(['radio', 'label'])(
    'closes when another %s is activated and reports the new value once',
    async (target) => {
      const { trigger, onChange } = await mountSelect()
      const next = radio('two')

      await click(target === 'radio' ? next : next.closest('label')!)

      expect(trigger.getAttribute('aria-expanded')).toBe('false')
      expect(document.querySelector('[role="dialog"]')).toBeNull()
      expect(onChange).toHaveBeenCalledExactlyOnceWith('two')
    },
  )

  it.each(['radio', 'label'])(
    'keeps the picker open when a disabled %s is activated',
    async (target) => {
      const { trigger, onChange } = await mountSelect()
      const disabled = radio('disabled')
      expect(disabled.disabled).toBe(true)

      await click(target === 'radio' ? disabled : disabled.closest('label')!)

      expect(trigger.getAttribute('aria-expanded')).toBe('true')
      expect(onChange).not.toHaveBeenCalled()
    },
  )

  it('does not dismiss when the current option is disabled', async () => {
    const { trigger, onChange } = await mountSelect({
      options: [{ value: 'one', label: 'Option one', disabled: true }],
    })

    await click(radio('one').closest('label')!)

    expect(trigger.getAttribute('aria-expanded')).toBe('true')
    expect(onChange).not.toHaveBeenCalled()
  })

  it('preserves the clear option callback', async () => {
    const onClear = vi.fn()
    const { trigger, onChange } = await mountSelect({ allowEmpty: true, onClear })
    const empty = document.querySelector<HTMLInputElement>(
      'input[type="radio"]',
    )!

    await click(empty.closest('label')!)

    expect(trigger.getAttribute('aria-expanded')).toBe('false')
    expect(onClear).toHaveBeenCalledOnce()
    expect(onChange).not.toHaveBeenCalled()
  })

  it('returns from an embedded picker to its sheet and restores trigger focus', async () => {
    const { trigger, onChange } = await mountSelect({}, true)
    expect(document.querySelectorAll('[role="dialog"]')).toHaveLength(1)
    expect(
      document.querySelector('button[aria-label="Back from Example"]'),
    ).not.toBeNull()

    await click(radio('one').closest('label')!)
    await act(async () => {
      await new Promise<void>((resolve) => {
        window.requestAnimationFrame(() => resolve())
      })
    })

    expect(trigger.getAttribute('aria-expanded')).toBe('false')
    expect(document.querySelectorAll('[role="dialog"]')).toHaveLength(1)
    expect(trigger.closest('[aria-hidden]')?.getAttribute('aria-hidden')).toBe(
      'false',
    )
    expect(document.activeElement).toBe(trigger)
    expect(onChange).not.toHaveBeenCalled()
  })
})
