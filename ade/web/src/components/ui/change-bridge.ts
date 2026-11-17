import type * as React from 'react'

/**
 * The first console API hands checkboxes and switches a React change event
 * whose `currentTarget.checked` is the new state. The shadcn (Base UI)
 * controls report `onCheckedChange(checked, details)` from the button they
 * render and keep a hidden native input for forms. This builds the event
 * the old handlers read, around that hidden input, so a worker UI written
 * for `event.currentTarget.checked` keeps working unchanged.
 */
export function changeEventFor(
  input: HTMLInputElement | null,
  checked: boolean,
  native: Event | undefined,
): React.ChangeEvent<HTMLInputElement> {
  if (input) input.checked = checked
  const target = input ?? ({ checked } as unknown as HTMLInputElement)
  return {
    type: 'change',
    target,
    currentTarget: target,
    nativeEvent: native ?? new Event('change'),
    bubbles: true,
    cancelable: false,
    defaultPrevented: false,
    eventPhase: 0,
    isTrusted: false,
    timeStamp: Date.now(),
    isDefaultPrevented: () => false,
    isPropagationStopped: () => false,
    preventDefault: () => undefined,
    stopPropagation: () => undefined,
    persist: () => undefined,
  } as unknown as React.ChangeEvent<HTMLInputElement>
}
