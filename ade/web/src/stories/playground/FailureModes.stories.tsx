import type { Meta, StoryObj } from '@storybook/react-vite'
import { scenarioStory } from './scenario-story'

const meta = {
  title: 'Playground/Failure Modes',
  parameters: { layout: 'fullscreen' },
} satisfies Meta

export default meta
type Story = StoryObj

export const AbortMidThought: Story = scenarioStory('abort-mid-thought')
export const ErrorOnFcall: Story = scenarioStory('error-on-fcall')
