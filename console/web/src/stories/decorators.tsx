import type { Decorator } from '@storybook/react-vite'

/**
 * Wrap a story in the `.workers-tab` scope. The env-template Lexical pill
 * styling in index.css is scoped to that class, so schema-form and
 * worker-config stories need this wrapper to render at the right scale —
 * exactly as the live workers configuration surface does.
 */
export const WorkersTabDecorator: Decorator = (Story) => (
  <div className="workers-tab">
    <Story />
  </div>
)

/**
 * Constrain a story to a readable column and pad it, mirroring the gutters
 * the Examples spec sheet used to apply around each section.
 */
export const PaddedDecorator: Decorator = (Story) => (
  <div className="p-6 max-w-[1100px] mx-auto">
    <Story />
  </div>
)
