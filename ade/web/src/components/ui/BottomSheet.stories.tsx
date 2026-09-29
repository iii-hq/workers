import type { Meta, StoryObj } from '@storybook/react-vite'
import { useState } from 'react'
import { ExtensionScopeProvider } from '@/lib/ui-scope'
import {
  BottomSheet,
  BottomSheetContent,
  BottomSheetTrigger,
} from './BottomSheet'
import { Button } from './Button'

const meta = {
  title: 'UI/BottomSheet',
  component: BottomSheet,
  parameters: { layout: 'centered', viewport: { defaultViewport: 'mobile1' } },
} satisfies Meta<typeof BottomSheet>

export default meta
type Story = StoryObj<typeof meta>

/** Mobile-only (`md:hidden`): narrow the viewport to see it. */
export const Basic: Story = {
  render: () => {
    const [open, setOpen] = useState(false)
    return (
      <BottomSheet open={open} onOpenChange={setOpen}>
        <BottomSheetTrigger asChild>
          <Button type="button">Open sheet</Button>
        </BottomSheetTrigger>
        <BottomSheetContent
          heading="Session options"
          description="The portal carries the worker's data-iii-ui scope."
        >
          <div className="px-4 pb-2 font-sans text-[13px] text-ink">
            Sheet body.
          </div>
        </BottomSheetContent>
      </BottomSheet>
    )
  },
}

/** Inside an injected worker's scope the portalled content is wrapped in
    `[data-iii-ui="demo-worker"]`, so the worker's scoped CSS still applies. */
export const ScopedPortal: Story = {
  render: () => (
    <ExtensionScopeProvider scope="demo-worker">
      <BottomSheet defaultOpen>
        <BottomSheetContent heading="Scoped">
          <div className="px-4 pb-2 font-sans text-[13px] text-ink">
            Inspect the DOM: the sheet sits under data-iii-ui="demo-worker".
          </div>
        </BottomSheetContent>
      </BottomSheet>
    </ExtensionScopeProvider>
  ),
}
