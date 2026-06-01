import type { StoryObj } from '@storybook/react-vite'
import { PlaygroundHarness } from './harness'
import { findScenario } from './scenarios'

/**
 * Build a Storybook story from a registered playground scenario. Each story
 * renders the live `PlaygroundHarness` (ChatView + event log) over the
 * scenario's `ChatBackend`, preserving its preferred mode — the closest
 * parity with the old `#/playground` picker, now driven by the sidebar.
 */
export function scenarioStory(id: string): StoryObj {
  const scenario = findScenario(id)
  if (!scenario) throw new Error(`unknown playground scenario: ${id}`)
  return {
    name: scenario.label,
    render: () => (
      <PlaygroundHarness
        backend={scenario.backend}
        label={scenario.label}
        preferredMode={scenario.preferredMode}
      />
    ),
  }
}
