// @vitest-environment jsdom
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { describe, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import { ModelPicker, ModelPickerPanel } from './ModelPicker'
import { PinnedModelMenu } from './PinnedModelMenu'

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

// jsdom does not implement scrolling; selection behavior still uses the real panel.
Element.prototype.scrollIntoView = () => {}

describe('asynchronous model selection', () => {
  it.each([false, true])(
    'only changes reasoning effort when accepted=%s',
    async (accepted) => {
      let settle: (accepted: boolean) => void = () => {
        throw new Error('Selection not requested')
      }
      const onChange = vi.fn(
        () =>
          new Promise<boolean>((resolve) => {
            settle = resolve
          }),
      )
      const onEffort = vi.fn()
      const container = document.body.appendChild(document.createElement('div'))
      const root = createRoot(container)
      try {
        await act(async () =>
          root.render(
            <TooltipProvider>
              <ModelPickerPanel
                value="openai::large"
                options={[
                  {
                    id: 'openai::small',
                    label: 'Small model',
                    supportsThinking: true,
                  },
                ]}
                providers={[]}
                thinkingLevel="high"
                onChange={onChange}
                onThinkingLevelChange={onEffort}
                autoFocusFilter={false}
              />
            </TooltipProvider>,
          ),
        )
        const choice =
          [...container.querySelectorAll('button')].find(
            (button) => button.textContent === 'Small Model',
          ) ??
          [...container.querySelectorAll('button')].find(
            (button) => button.textContent?.toLowerCase() === 'small model',
          )
        if (!choice) throw new Error('Model option missing')
        await act(async () => choice.click())
        expect(onChange).toHaveBeenCalledWith('openai::small', 'default')
        expect(onEffort).not.toHaveBeenCalled()
        await act(async () => settle(accepted))
        if (accepted) expect(onEffort).toHaveBeenCalledWith('default')
        else expect(onEffort).not.toHaveBeenCalled()
      } finally {
        act(() => root.unmount())
        container.remove()
      }
    },
  )
})

function pressKey(target: Element, key: string) {
  act(() => {
    target.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true }))
  })
}

function typeFilter(container: HTMLElement, text: string) {
  const input = container.querySelector<HTMLInputElement>(
    'input[role="combobox"]',
  )
  if (!input) throw new Error('search missing')
  act(() => {
    Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      'value',
    )?.set?.call(input, text)
    input.dispatchEvent(new Event('input', { bubbles: true }))
  })
  return input
}

describe('an automatic choice', () => {
  const options = [
    { id: 'openai::gpt-5', label: 'gpt-5' },
    { id: 'openai::gpt-4.1', label: 'gpt-4.1' },
  ]

  it.each(['panel', 'menu'] as const)(
    'leads the %s list and reports its selection',
    async (surface) => {
      const onSelect = vi.fn()
      const onChange = vi.fn()
      const automatic = { label: 'Automatic · gpt-5', onSelect }
      const container = document.body.appendChild(document.createElement('div'))
      const root = createRoot(container)
      try {
        await act(async () =>
          root.render(
            <TooltipProvider>
              {surface === 'panel' ? (
                <ModelPickerPanel
                  value={null}
                  options={options}
                  providers={[]}
                  thinkingLevel="default"
                  onChange={onChange}
                  onThinkingLevelChange={() => {}}
                  automatic={automatic}
                />
              ) : (
                <PinnedModelMenu
                  value={null}
                  options={options}
                  providers={[]}
                  pinned={[]}
                  thinkingLevel="default"
                  efforts={[]}
                  onSelect={onChange}
                  onThinkingLevelChange={() => {}}
                  onPinnedChange={() => {}}
                  automatic={automatic}
                  autoFocusSearch={false}
                />
              )}
            </TooltipProvider>,
          ),
        )
        const rows = () => [
          ...container.querySelectorAll<HTMLButtonElement>(
            'button[data-model-option]',
          ),
        ]
        const [first] = rows()
        expect(first?.dataset.modelOption).toBe(':automatic')
        expect(first?.getAttribute('aria-pressed')).toBe('true')
        expect(first?.textContent).toContain('Automatic · gpt-5')

        await act(async () => first?.click())
        expect(onSelect).toHaveBeenCalledTimes(1)

        // Unset, the highlight starts on it: Enter picks it from the search.
        const input = typeFilter(container, '')
        pressKey(input, 'Enter')
        expect(onSelect).toHaveBeenCalledTimes(2)

        // The search matches it like any row.
        typeFilter(container, 'automatic')
        expect(rows().map((row) => row.dataset.modelOption)).toEqual([
          ':automatic',
        ])
        typeFilter(container, 'gpt-4')
        expect(rows().map((row) => row.dataset.modelOption)).not.toContain(
          ':automatic',
        )
        // Arrow keys walk off it and back like any row.
        typeFilter(container, 'gpt')
        pressKey(input, 'ArrowDown')
        pressKey(input, 'ArrowUp')
        pressKey(input, 'Enter')
        expect(onSelect).toHaveBeenCalledTimes(3)
        pressKey(input, 'ArrowDown')
        pressKey(input, 'Enter')
        expect(onSelect).toHaveBeenCalledTimes(3)
        expect(onChange.mock.calls[0]?.[0]).toBe('openai::gpt-5')
      } finally {
        act(() => root.unmount())
        container.remove()
      }
    },
  )

  it('the menu does not select it for a stored id the options do not list', async () => {
    const container = document.body.appendChild(document.createElement('div'))
    const root = createRoot(container)
    try {
      await act(async () =>
        root.render(
          <TooltipProvider>
            <ModelPicker
              value="openai::gone"
              options={options}
              providers={[]}
              thinkingLevel="default"
              onChange={() => {}}
              onThinkingLevelChange={() => {}}
              automatic={{ label: 'Automatic · gpt-5', onSelect: () => {} }}
              showRefresh={false}
            />
          </TooltipProvider>,
        ),
      )
      const trigger = container.querySelector<HTMLButtonElement>('button')
      if (!trigger) throw new Error('trigger missing')
      await act(async () => trigger.click())
      const row = document.body.querySelector(
        'button[data-model-option=":automatic"]',
      )
      expect(row?.getAttribute('aria-pressed')).toBe('false')
    } finally {
      act(() => root.unmount())
      container.remove()
    }
  })
})

describe('the dropdown search', () => {
  it('takes typed characters instead of the menu typeahead', async () => {
    const container = document.body.appendChild(document.createElement('div'))
    const root = createRoot(container)
    try {
      await act(async () =>
        root.render(
          <TooltipProvider>
            <ModelPicker
              value="openai::gpt-5"
              options={[
                { id: 'openai::gpt-5', label: 'gpt-5' },
                { id: 'openai::gpt-4.1', label: 'gpt-4.1' },
              ]}
              providers={[]}
              thinkingLevel="default"
              onChange={() => {}}
              onThinkingLevelChange={() => {}}
              showRefresh={false}
            />
          </TooltipProvider>,
        ),
      )
      const trigger = container.querySelector<HTMLButtonElement>('button')
      if (!trigger) throw new Error('trigger missing')
      await act(async () => trigger.click())
      const input = document.body.querySelector<HTMLInputElement>(
        'input[aria-label="filter models"]',
      )
      if (!input) throw new Error('search missing')
      for (const key of ['g', 'p', ' ', 'T']) {
        const event = new KeyboardEvent('keydown', {
          key,
          bubbles: true,
          cancelable: true,
        })
        act(() => {
          input.dispatchEvent(event)
        })
        // A prevented keydown never inserts its character.
        expect(event.defaultPrevented, key).toBe(false)
      }
    } finally {
      act(() => root.unmount())
      container.remove()
    }
  })
})
