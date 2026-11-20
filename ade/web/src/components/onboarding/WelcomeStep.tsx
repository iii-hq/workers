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
        <p className="max-w-[56ch] text-pretty text-sm leading-5 text-muted-foreground">
          iii is built on the idea that software should be easy to compose,
          observe, discover, extend, and react to. In iii anything you build is
          composed of one or more Functions. This means that there is no
          division between what is a plugin, extension, service, or API.
        </p>
        <p className="max-w-[56ch] text-pretty text-sm leading-5 text-muted-foreground">
          The Function primitive makes development clear, predictable, and
          better fit for agentic development workflows than traditional system
          architectures.
        </p>
        <p className="max-w-[56ch] text-pretty text-sm leading-5 text-muted-foreground">
          The best way to see what we mean is to add your AI provider on the
          next screen and start building either your own project or from one of
          our many examples.
        </p>
        <p className="max-w-[56ch] text-pretty text-sm leading-5 text-muted-foreground">
          And after that to check out our docs.
        </p>
      </header>
    </StepLayout>
  )
}
