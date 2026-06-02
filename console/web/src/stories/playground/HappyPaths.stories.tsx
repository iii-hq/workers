import type { Meta, StoryObj } from '@storybook/react-vite'
import { scenarioStory } from './scenario-story'

// No `component` field: the harness wraps ChatView and takes a required
// `backend`, which would force every story to supply `args`. These stories
// drive everything through `render`, so a loose Meta keeps typing clean.
const meta = {
  title: 'Playground/Happy Paths',
  parameters: { layout: 'fullscreen' },
} satisfies Meta

export default meta
type Story = StoryObj

export const HappyPlan: Story = scenarioStory('happy-plan')
export const HappyAsk: Story = scenarioStory('happy-ask')
export const HappyAgent: Story = scenarioStory('happy-agent')
export const CoderMutate: Story = scenarioStory('coder-mutate')
export const CoderUpdate: Story = scenarioStory('coder-update')
