import { ArrowRight } from 'lucide-react'
import { Button } from './controls'
import { StepLayout } from './parts'
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
        <h2 className="text-balance text-xl font-semibold leading-7 tracking-[-0.01em] text-ink">
          Welcome to the{' '}
          <abbr
            title="Agentic Development Environment"
            className="no-underline"
          >
            ADE
          </abbr>
        </h2>
        <p className="max-w-[56ch] text-pretty text-sm leading-5 text-muted-foreground">
          Your agentic development environment, built on iii. Everything you add
          is composable, observable, discoverable, extensible, and reactive.
          Connect a model to give your agent access to all of it.
        </p>
      </header>
    </StepLayout>
  )
}
