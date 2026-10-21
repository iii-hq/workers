import type { Meta, StoryObj } from '@storybook/react-vite'
import { Button } from '@/components/shadcn/button'
import {
  Popover,
  PopoverContent,
  PopoverDescription,
  PopoverHeader,
  PopoverTitle,
  PopoverTrigger,
} from '@/components/shadcn/popover'

/**
 * The shadcn/ui primitives (Base UI flavour) the redesign reaches for,
 * painted by the console's own tokens: ink, surfaces and the focus stroke
 * follow `data-theme` with no second palette. New sections build on these;
 * existing Radix-based `components/ui` stay until their section moves.
 */
function Catalogue() {
  return (
    <div className="flex flex-col gap-6 p-6">
      <section className="flex flex-wrap items-center gap-2">
        <Button>Default</Button>
        <Button variant="secondary">Secondary</Button>
        <Button variant="outline">Outline</Button>
        <Button variant="ghost">Ghost</Button>
        <Button variant="destructive">Destructive</Button>
        <Button variant="link">Link</Button>
        <Button size="sm">Small</Button>
        <Button size="xs">Extra small</Button>
      </section>
      <section>
        <Popover>
          <PopoverTrigger
            render={<Button variant="outline">Open a popover</Button>}
          />
          <PopoverContent align="start">
            <PopoverHeader>
              <PopoverTitle>Popover</PopoverTitle>
              <PopoverDescription>
                A Base UI popover on the console's surfaces.
              </PopoverDescription>
            </PopoverHeader>
          </PopoverContent>
        </Popover>
      </section>
    </div>
  )
}

const meta = {
  title: 'Design/shadcn',
  component: Catalogue,
  parameters: { layout: 'fullscreen' },
} satisfies Meta<typeof Catalogue>

export default meta
type Story = StoryObj<typeof meta>

export const Primitives: Story = {}
