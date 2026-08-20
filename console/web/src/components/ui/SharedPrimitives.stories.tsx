import type { Meta, StoryObj } from '@storybook/react-vite'
import { Box, Cpu } from 'lucide-react'
import { Chip } from './Chip'
import { List, ListItem } from './List'
import {
  Card,
  CardBody,
  CardHeader,
  Panel,
  PanelBody,
  PanelHeader,
} from './Surface'

const meta = {
  title: 'UI/Shared recipes',
  parameters: { layout: 'padded' },
} satisfies Meta

export default meta
type Story = StoryObj

export const NeutralSelection: Story = {
  render: () => (
    <div className="grid max-w-3xl gap-4 sm:grid-cols-2">
      <Panel>
        <PanelHeader>Workers</PanelHeader>
        <PanelBody>
          <List>
            <ListItem
              leading={<Cpu className="size-4" />}
              label="llm-router"
              description="7 triggers · healthy"
              trailing={<Chip>internal</Chip>}
            />
            <ListItem
              selected
              leading={<Cpu className="size-4" />}
              label="memory"
              description="3 functions · selected"
              trailing={<Chip selected>active</Chip>}
            />
          </List>
        </PanelBody>
      </Panel>
      <Card selected interactive tabIndex={0}>
        <CardHeader>
          <Box className="size-4" aria-hidden /> Selected Card
        </CardHeader>
        <CardBody className="font-mono text-[12px] text-ink-faint">
          neutral wash, edge and ink in both themes; no blue/orange selection
          rail, border or title.
        </CardBody>
      </Card>
    </div>
  ),
}
