import type { LucideIcon } from 'lucide-react'
import {
  Activity,
  ArrowRight,
  Boxes,
  Puzzle,
  ScanSearch,
  Zap,
} from 'lucide-react'
import { PILLARS, type PillarId } from '@/lib/onboarding/catalog'
import { Button } from './controls'
import { Rows, StepLayout } from './parts'

export const PILLAR_ICONS: Record<PillarId, LucideIcon> = {
  extensible: Puzzle,
  discoverable: ScanSearch,
  observable: Activity,
  composable: Boxes,
  reactive: Zap,
}

export function WelcomeStep({
  onStart,
  onSkip,
}: {
  onStart: () => void
  onSkip: () => void
}) {
  return (
    <StepLayout
      centered
      footer={
        <>
          <Button variant="ghost" className="-ml-3" onClick={onSkip}>
            Skip setup
          </Button>
          <Button onClick={onStart}>
            Get started <ArrowRight aria-hidden />
          </Button>
        </>
      }
    >
      <header className="flex flex-col gap-1.5 -mt-2">
        <h2 className="text-balance text-lg font-semibold leading-7 tracking-[-0.01em] text-ink">
          Welcome to the{' '}
          <abbr
            title="Agentic Development Environment"
            className="no-underline"
          >
            ADE
          </abbr>
        </h2>
        <p className="max-w-[56ch] text-pretty text-sm leading-5 text-neutral-600 dark:text-neutral-400">
          Your agentic development environment. Connect a model and give your
          agent access to everything your backend can do.
        </p>
      </header>
      <Rows as="ul">
        {PILLARS.map((pillar) => {
          const Icon = PILLAR_ICONS[pillar.id]
          return (
            <li
              key={pillar.id}
              className="flex items-start gap-3 px-3.5 py-2.5"
            >
              <Icon
                className="mt-0.5 size-4 shrink-0 text-neutral-500 dark:text-neutral-400"
                strokeWidth={1.75}
                aria-hidden
              />
              <span className="flex min-w-0 flex-col gap-px">
                <span className="text-[13px] font-medium leading-5 text-ink">
                  {pillar.title}
                </span>
                <span className="text-pretty text-[13px] leading-5 text-neutral-600 dark:text-neutral-400">
                  {pillar.line}
                </span>
              </span>
            </li>
          )
        })}
      </Rows>
    </StepLayout>
  )
}
