import type { LucideIcon } from 'lucide-react'
import { Boxes, Gauge, Puzzle, ScanSearch, Zap } from 'lucide-react'
import { Button } from '@/components/ui/Button'
import { PILLARS, type PillarId } from '@/lib/onboarding/catalog'
import { Section, StepHeader, StepLayout } from './parts'

export const PILLAR_ICONS: Record<PillarId, LucideIcon> = {
  extensible: Puzzle,
  discoverable: ScanSearch,
  optimized: Gauge,
  composable: Boxes,
  reactive: Zap,
}

const SETUP = [
  'Look for Claude Code, Codex and API keys already on this machine',
  'Connect at least one model provider',
  'Optionally, set up Judge for the small decisions agents make',
]

export function WelcomeStep({
  onStart,
  onSkip,
}: {
  onStart: () => void
  onSkip: () => void
}) {
  return (
    <StepLayout
      footer={
        <>
          <Button variant="ghost" onClick={onSkip}>
            Skip setup
          </Button>
          <Button onClick={onStart}>Get started</Button>
        </>
      }
    >
      <StepHeader
        eyebrow="Welcome to the ADE"
        title="Agents that live inside your backend"
        lead="The harness is an agent loop that runs as a worker on the iii engine, next to every function your backend registers. Connect a model and it can use all of them."
      />
      <ul className="grid gap-x-6 gap-y-4 @2xl:grid-cols-2">
        {PILLARS.map((pillar, index) => {
          const Icon = PILLAR_ICONS[pillar.id]
          return (
            <li
              key={pillar.id}
              className="onboarding-rise flex gap-3"
              style={{ animationDelay: `${index * 60}ms` }}
            >
              <span className="mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-md bg-surface text-ink">
                <Icon className="size-4" aria-hidden />
              </span>
              <span className="flex min-w-0 flex-col gap-0.5">
                <span className="font-sans text-[13px] font-semibold text-ink">
                  {pillar.title}
                </span>
                <span className="text-pretty font-sans text-[12px] leading-relaxed text-ink-faint">
                  {pillar.line}
                </span>
              </span>
            </li>
          )
        })}
      </ul>
      <Section title="This setup takes about two minutes">
        <ol className="flex flex-col gap-2 rounded-md bg-surface px-3 py-3">
          {SETUP.map((line, index) => (
            <li
              key={line}
              className="flex gap-3 font-sans text-[13px] text-ink"
            >
              <span className="w-4 shrink-0 text-right font-mono text-[11px] leading-5 tabular-nums text-ink-ghost">
                {index + 1}
              </span>
              <span className="leading-5">{line}</span>
            </li>
          ))}
        </ol>
        <p className="font-sans text-[12px] leading-relaxed text-ink-faint">
          Nothing is installed without your click. Every worker this setup adds,
          and every setting it writes, is shown before it runs and logged as it
          happens.
        </p>
      </Section>
    </StepLayout>
  )
}
