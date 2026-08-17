import type { Meta, StoryObj } from '@storybook/react-vite'
import { useEffect, useState } from 'react'
import { registerExtSessionChip } from '@/lib/ui-slots'
import type { SessionChipProps } from '@/types/injectable-ui'
import { PlaygroundHarness } from './harness'
import { findScenario } from './scenarios'

/**
 * The `chat` extension slot: what an injected session chip looks like in
 * the chat header's right cluster. The demo chip carries id `context`, so
 * it also demonstrates the supersede rule — the built-in estimate-based
 * ContextUsage meter hides while the chip is registered.
 */

function DemoContextChip({ contextWindow }: SessionChipProps) {
  const window = contextWindow ?? 200_000
  const used = Math.round(window * 0.62)
  const pct = Math.round((used / window) * 100)
  return (
    <span
      className="flex items-center gap-1.5 font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint"
      title={`${used.toLocaleString()} / ${window.toLocaleString()} tokens (${pct}%)`}
    >
      <span>ctx</span>
      <span className="relative h-[6px] w-14 overflow-hidden bg-surface-active">
        <span
          className="absolute inset-y-0 left-0 bg-accent"
          style={{ width: `${pct}%` }}
        />
      </span>
      <span className="tabular-nums text-ink">{pct}%</span>
    </span>
  )
}

function WithDemoChip({ children }: { children: React.ReactElement }) {
  const [registered, setRegistered] = useState(false)
  useEffect(() => {
    const off = registerExtSessionChip({
      id: 'context',
      scope: 'storybook',
      path: 'storybook/demo.js',
      render: DemoContextChip,
    })
    setRegistered(true)
    return off
  }, [])
  return registered ? children : null
}

const meta = {
  title: 'Playground/SessionChips',
  parameters: { layout: 'fullscreen' },
} satisfies Meta

export default meta
type Story = StoryObj

export const InjectedContextChip: Story = {
  name: 'injected context chip in the header',
  render: () => {
    const scenario = findScenario('multi-function-agent')
    if (!scenario) throw new Error('missing playground scenario')
    return (
      <WithDemoChip>
        <PlaygroundHarness
          backend={scenario.backend}
          label={scenario.label}
          preferredMode={scenario.preferredMode}
        />
      </WithDemoChip>
    )
  },
}
