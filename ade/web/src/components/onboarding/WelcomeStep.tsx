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
          Welcome to{' '}
          <abbr
            title="Agentic Development Environment"
            className="no-underline"
          >
            iii's Agentic Development Environment (ADE)
          </abbr>
        </h2>
        <div className="flex max-w-[56ch] flex-col gap-2.5 text-pretty text-sm leading-5 text-muted-foreground">
          <p>
            iii makes software easy to compose, observe, discover, extend, and
            react to.
          </p>
          <p>
            Everything you build is made of one primitive, the Function, so
            there's no line between a plugin, an extension, a service, or an
            API. Your agent fits into your projects as naturally as their own
            code.
          </p>
          <p>
            To see what we mean, connect your AI provider on the next screen,
            then start your own project or one of our examples.
          </p>
        </div>
      </header>
    </StepLayout>
  )
}
