import type { Meta, StoryObj } from '@storybook/react-vite'
import { scenarioStory } from './scenario-story'

const meta = {
  title: 'Playground/Timing',
  parameters: { layout: 'fullscreen' },
} satisfies Meta

export default meta
type Story = StoryObj

export const SlowTokens: Story = scenarioStory('slow-tokens')
export const FastTokens: Story = scenarioStory('fast-tokens')
