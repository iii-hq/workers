import type { Meta, StoryObj } from '@storybook/react-vite'
import { scenarioStory } from './scenario-story'

const meta = {
  title: 'Playground/Agent',
  parameters: { layout: 'fullscreen' },
} satisfies Meta

export default meta
type Story = StoryObj

export const MultiFunctionAgent: Story = scenarioStory('multi-function-agent')
export const PendingApproval: Story = scenarioStory('pending-approval')
