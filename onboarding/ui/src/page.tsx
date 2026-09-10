import {
  Badge,
  Button,
  CollapsibleCard,
  CollapsibleCardContent,
  CollapsibleCardTrigger,
  EmptyState,
  type Host,
  PageBody,
  PageHeader,
  PageMain,
  type PageRenderProps,
  PageShell,
  Skeleton,
} from '@iii-dev/console-ui'
import { useCallback, useEffect, useState } from 'react'
import { disposeSpotlight, hideSpotlight, showSpotlight } from './spotlight'

interface Step {
  id: string
  title: string
  body: string
  anchor?: string
}

interface Tour {
  id: string
  title: string
  description: string
  steps: Step[]
}

interface TourProgress {
  step_index: number
  reached_index: number
  status: 'started' | 'completed' | 'dismissed'
}

interface ProgressResponse {
  tours: Record<string, TourProgress | undefined>
  next_tour_id: string | null
}

export function TourPage({ host }: { host: Host } & PageRenderProps) {
  const [tour, setTour] = useState<Tour | null>(null)
  const [step, setStep] = useState(0)
  const [reached, setReached] = useState(0)
  const [open, setOpen] = useState<string | null>(null)
  const [done, setDone] = useState(false)
  const [error, setError] = useState<string | null>(null)

  // Load the tour the operator has not finished, resumed at the step they
  // reached: progress lives in the engine, so another tab or another day
  // picks up in the same place.
  useEffect(() => {
    let live = true
    void (async () => {
      try {
        const [list, progress] = await Promise.all([
          host.iii.trigger<{ tours: { id: string }[] }>('onboarding::tours::list'),
          host.iii.trigger<ProgressResponse>('onboarding::progress::get', {}),
        ])
        const id = progress.next_tour_id ?? list.tours[0]?.id
        if (!id) return
        const { tour: loaded } = await host.iii.trigger<{ tour: Tour }>('onboarding::tours::get', { id })
        if (!live) return
        const stored = progress.tours[id]
        setTour(loaded)
        setStep(stored?.step_index ?? 0)
        setReached(stored?.reached_index ?? 0)
        setDone(stored?.status === 'completed')
        setOpen(loaded.steps[stored?.step_index ?? 0]?.id ?? null)
      } catch (cause) {
        if (live) setError(cause instanceof Error ? cause.message : String(cause))
      }
    })()
    return () => {
      live = false
    }
  }, [host])

  // The box follows the step, and leaves with the page.
  useEffect(() => {
    if (!tour || done) {
      hideSpotlight()
      return
    }
    showSpotlight(tour.steps[step]?.anchor ?? null)
  }, [tour, step, done])
  useEffect(() => disposeSpotlight, [])

  const go = useCallback(
    (index: number, status: 'started' | 'completed' = 'started') => {
      if (!tour) return
      const clamped = Math.max(0, Math.min(index, tour.steps.length - 1))
      setStep(clamped)
      setReached((furthest) => Math.max(furthest, clamped))
      setOpen(tour.steps[clamped]?.id ?? null)
      setDone(status === 'completed')
      void host.iii
        .trigger('onboarding::progress::set', {
          tour_id: tour.id,
          step_index: clamped,
          status,
        })
        .catch((cause: unknown) => setError(cause instanceof Error ? cause.message : String(cause)))
    },
    [host, tour],
  )

  if (error)
    return (
      <Frame>
        <EmptyState title="Could not load the tour" description={error} />
      </Frame>
    )
  if (!tour)
    return (
      <Frame>
        <Skeleton />
      </Frame>
    )

  const last = step >= tour.steps.length - 1

  return (
    <Frame
      title={tour.title}
      description={tour.description}
      actions={
        <Badge variant={done ? 'ok' : 'default'}>
          {done ? 'complete' : `step ${step + 1} of ${tour.steps.length}`}
        </Badge>
      }
    >
      <div className="onboarding-steps">
        {tour.steps.map((item, index) => {
          // A step opens once it has been reached — reading ahead would give
          // away a box the operator has not been shown yet.
          const locked = index > reached
          return (
            <CollapsibleCard
              key={item.id}
              className="onboarding-step"
              data-current={index === step ? 'true' : undefined}
              open={open === item.id}
              disabled={locked}
              onOpenChange={(next) => setOpen(next ? item.id : null)}
            >
              <CollapsibleCardTrigger>
                <span className="onboarding-step-index">{index + 1}</span>
                <span className="onboarding-step-title">{item.title}</span>
                {locked ? <span className="onboarding-step-lock">not reached</span> : null}
              </CollapsibleCardTrigger>
              <CollapsibleCardContent>
                <p className="onboarding-step-body">{item.body}</p>
                {index !== step && !locked ? (
                  <Button variant="ghost" size="sm" onClick={() => go(index)}>
                    Show me this one
                  </Button>
                ) : null}
              </CollapsibleCardContent>
            </CollapsibleCard>
          )
        })}
      </div>

      <div className="onboarding-controls">
        <Button variant="ghost" disabled={step === 0} onClick={() => go(step - 1)}>
          Back
        </Button>
        {last ? (
          <Button onClick={() => go(step, 'completed')} disabled={done}>
            {done ? 'Finished' : 'Finish'}
          </Button>
        ) : (
          <Button onClick={() => go(step + 1)}>Next</Button>
        )}
      </div>
    </Frame>
  )
}

function Frame({
  title = 'Tour',
  description,
  actions,
  children,
}: {
  title?: string
  description?: string
  actions?: React.ReactNode
  children: React.ReactNode
}) {
  return (
    <PageShell>
      <PageMain>
        <PageHeader title={title} description={description} actions={actions} />
        <PageBody>{children}</PageBody>
      </PageMain>
    </PageShell>
  )
}
