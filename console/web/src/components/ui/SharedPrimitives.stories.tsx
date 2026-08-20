import type { Meta, StoryObj } from '@storybook/react-vite'
import { Box, Cpu } from 'lucide-react'
import { Chip } from './Chip'
import {
  CollapsibleCard,
  CollapsibleCardContent,
  CollapsibleCardTrigger,
} from './CollapsibleCard'
import { List, ListItem } from './List'
import {
  Card,
  CardBody,
  CardHeader,
  CardHighlight,
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
          <CardHighlight className="p-3">
            Borderless highlight for related content nested inside a card.
          </CardHighlight>
        </CardBody>
      </Card>
      <CollapsibleCard defaultOpen className="sm:col-span-2">
        <CollapsibleCardTrigger className="p-3">
          <div className="flex min-w-0 items-center justify-between gap-3">
            <div className="min-w-0 font-sans text-sm font-semibold text-ink">
              Collapsible activity
            </div>
            <Chip>Expandable</Chip>
          </div>
        </CollapsibleCardTrigger>
        <CollapsibleCardContent>
          <div className="border-t border-edge p-3 font-sans text-sm text-ink-faint">
            This content remains mounted while the shared auto-height transition
            opens and closes the card.
          </div>
        </CollapsibleCardContent>
      </CollapsibleCard>
    </div>
  ),
}
