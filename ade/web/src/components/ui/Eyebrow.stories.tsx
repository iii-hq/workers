import type { Meta, StoryObj } from '@storybook/react-vite'
import { Eyebrow } from './Eyebrow'

const meta = {
  title: 'UI/Eyebrow',
  component: Eyebrow,
  parameters: { layout: 'padded' },
} satisfies Meta<typeof Eyebrow>

export default meta
type Story = StoryObj<typeof meta>

/** The mono caps label above a section, a pane, or a key/value pair. */
export const Default: Story = {
  args: { children: 'stdout' },
}

export const AsHeading: Story = {
  render: () => (
    <div className="flex w-[24rem] flex-col gap-2">
      <Eyebrow as="h2">functions · 12</Eyebrow>
      <p className="font-sans text-[13px] text-ink">
        Section body under an `h2` eyebrow.
      </p>
      <span className="iii-ui-eyebrow">the recipe class reads the same</span>
    </div>
  ),
}
