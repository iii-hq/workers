'use client'

import { Switch as SwitchPrimitive } from '@base-ui/react/switch'
import { cn } from 'cn'

/* A compact toggle. Track and thumb move on one 150ms ease-out; both are
   CSS transitions, so a second press mid-flight reverses from wherever the
   thumb is instead of restarting. */
function Switch({ className, ...props }: SwitchPrimitive.Root.Props) {
  return (
    <SwitchPrimitive.Root
      data-slot="switch"
      className={cn(
        'peer relative inline-flex h-6 w-10 shrink-0 cursor-pointer items-center rounded-full border border-transparent p-0.5 transition-[background-color,box-shadow] duration-150 ease-out outline-none focus-visible:ring-3 focus-visible:ring-ring/50 disabled:cursor-not-allowed disabled:opacity-50 data-checked:bg-neutral-900 data-unchecked:bg-neutral-300 data-unchecked:hover:bg-neutral-400/70 dark:data-checked:bg-neutral-300 dark:data-unchecked:bg-neutral-800 dark:data-unchecked:hover:bg-neutral-700',
        className,
      )}
      {...props}
    >
      <span
        className="pointer-events-none absolute left-1/2 top-1/2 size-[max(100%,2.75rem)] -translate-1/2 pointer-fine:hidden"
        aria-hidden="true"
      />
      <SwitchPrimitive.Thumb
        data-slot="switch-thumb"
        className="pointer-events-none block size-5 rounded-full bg-white shadow-[0_1px_2px_rgba(0,0,0,0.2)] transition-transform duration-150 ease-out data-checked:translate-x-4 data-unchecked:translate-x-0 dark:bg-neutral-400 dark:shadow-none dark:data-checked:bg-neutral-800"
      />
    </SwitchPrimitive.Root>
  )
}

export { Switch }
