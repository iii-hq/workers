import type { Meta, StoryObj } from '@storybook/react-vite'
import { EmailPrompt } from './EmailPrompt'

/**
 * The prompt opens from a stored record, so the story plants one that has
 * already earned it before the hook reads storage.
 */
function Harness() {
  localStorage.setItem(
    'iii-email-prompt',
    JSON.stringify({
      status: 'pending',
      activeMs: 60 * 60 * 1000,
      snoozeUntil: 0,
    }),
  )
  return (
    <div className="h-screen">
      <EmailPrompt />
    </div>
  )
}

const meta = {
  title: 'Console/EmailPrompt',
  component: Harness,
  parameters: { layout: 'fullscreen' },
} satisfies Meta<typeof Harness>

export default meta
type Story = StoryObj<typeof meta>

export const Ask: Story = {}
