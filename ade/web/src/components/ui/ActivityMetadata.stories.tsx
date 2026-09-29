import type { Meta, StoryObj } from '@storybook/react-vite'
import { ArrowRight, SquareFunction, TriangleAlert } from 'lucide-react'
import { ActionLine, ActivityMetadata, MetaRow } from './ActivityMetadata'
import { Badge } from './Badge'
import { Chip } from './Chip'

const meta = {
  title: 'UI/ActivityMetadata',
  component: ActivityMetadata,
  parameters: { layout: 'padded' },
} satisfies Meta<typeof ActivityMetadata>

export default meta
type Story = StoryObj<typeof meta>

export const Metadata: Story = {
  args: {
    createdAt: Date.now() - 5 * 60_000,
    identifier: 'call_0192f3a1-4b2c-7d3e-8f90-abcdef123456',
  },
}

/** The strip a function-trigger card opens with: pairs, chips, then the
    action lines — icon in the tone, body in ink. */
export const CardChrome: Story = {
  args: {},
  render: () => (
    <div className="w-[32rem] overflow-hidden rounded-sm border border-edge">
      <MetaRow
        items={[
          { label: 'status', value: '200' },
          { label: 'took', value: '842ms' },
        ]}
      >
        <Badge variant="ok">ok</Badge>
        <Chip>GET</Chip>
      </MetaRow>
      <ActionLine icon={<ArrowRight />} tone="ink">
        https://example.com/api/items?page=2
      </ActionLine>
      <ActionLine icon={<SquareFunction />}>web::fetch</ActionLine>
      <ActionLine icon={<TriangleAlert />} tone="warn">
        parse error: unexpected end of JSON input
      </ActionLine>
    </div>
  ),
}
