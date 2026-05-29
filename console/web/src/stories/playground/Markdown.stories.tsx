import type { Meta, StoryObj } from '@storybook/react-vite'
import { scenarioStory } from './scenario-story'

const meta = {
  title: 'Playground/Markdown',
  parameters: { layout: 'fullscreen' },
} satisfies Meta

export default meta
type Story = StoryObj

export const LongMarkdown: Story = scenarioStory('long-markdown')
export const MarkdownStress: Story = scenarioStory('markdown-stress')
