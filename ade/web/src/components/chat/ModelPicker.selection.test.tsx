// @vitest-environment jsdom
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { describe, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import { ModelPickerPanel } from './ModelPicker'

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
