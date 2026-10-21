import { ArrowRight, Check, LoaderCircle, TriangleAlert } from 'lucide-react'
import { Wordmark } from '@/components/ui/Wordmark'
import type { JudgeOption } from '@/lib/onboarding/catalog'
import type { KitInstall } from '@/lib/onboarding/kits'
import { servesUsableModels } from '@/lib/onboarding/plan'
import { Button } from './controls'
import { ProviderMark } from './ProviderMark'
import { Disclosure, Rows, StepLayout } from './parts'
import type { OnboardingController } from './use-onboarding'

/**
 * What setup connected. Next moves on to Explore iii, where the operator
 * picks what to do first.
 */
export function ReadyStep({
  onboarding,
  judges,
  onNext,
  kits = [],
  onRetryKit,
  onReviewKit,
}: {
  onboarding: OnboardingController
  /** The strategies Judge answers with, default first. */
  judges: readonly JudgeOption[]
  /** On to Explore iii. */
  onNext: () => void
  /** The template's kits and how their background install is going. */
  kits?: readonly KitInstall[]
  /** Install a kit that failed again. */
  onRetryKit?: (kit: string) => void
  /** Close the wizard and open Directory → Kits on the plan to review. */
  onReviewKit?: (install: KitInstall) => void
}) {
  const { snapshot, activity } = onboarding
  const connected = (snapshot.providers ?? []).filter(servesUsableModels)
  const totalModels = connected.reduce(
    (sum, provider) => sum + provider.modelCount,
    0,
  )
  const added = [
    ...new Set(
      activity
        .filter((entry) => entry.status === 'done')
        .flatMap((entry) => entry.workers ?? []),
    ),
  ]
  // Chromium this setup downloaded for the browser worker.
  const chromium = activity.some(
    (entry) => entry.group === 'browser' && entry.status === 'done',
  )
  const lines: { title: string; detail?: string; providerId?: string }[] = [
    ...connected.map((provider) => ({
      title: `${provider.title} connected`,
      providerId: provider.id,
      detail: `${provider.modelCount} ${provider.modelCount === 1 ? 'model' : 'models'}`,
    })),
    ...(chromium
      ? [
          {
            title: 'Chromium is ready for agents',
            detail: 'they open and check the pages they build',
          },
        ]
      : []),
    ...(judges.length > 0
      ? [
          {
            title: `Judge answers with ${judges[0].title}`,
            detail:
              judges.length > 1
                ? `${judges
                    .slice(1)
                    .map((option) => option.title)
                    .join(' and ')} also available`
                : 'function search, argument repair, browser decisions',
          },
        ]
      : []),
  ]

  return (
    <StepLayout
      centered
      footer={
        <Button className="ml-auto" onClick={() => onNext()}>
          Next
          <ArrowRight aria-hidden />
        </Button>
      }
    >
      <div className="flex flex-col items-center gap-3 text-center">
        <Wordmark appearance="assemble" className="size-9" />
        <div className="flex flex-col items-center gap-1">
          <h2 className="font-sans text-lg font-semibold leading-7 tracking-[-0.01em] text-ink">
            {connected.length > 0 ? 'Your harness is ready' : 'You’re all set'}
          </h2>
          <p className="max-w-[48ch] text-pretty font-sans text-sm leading-5 text-neutral-600 dark:text-neutral-400">
            {connected.length > 0 ? (
              <>
                <span className="font-medium tabular-nums text-ink">
                  {totalModels}
                </span>{' '}
                {totalModels === 1 ? 'model' : 'models'} from{' '}
                <span className="font-medium tabular-nums text-ink">
                  {connected.length}
                </span>{' '}
                {connected.length === 1 ? 'provider' : 'providers'} are
                connected.
              </>
            ) : (
              'Connect a model any time from the model picker in the composer.'
            )}
          </p>
        </div>
      </div>

      {lines.length > 0 ? (
        <Rows as="ul">
          {lines.map((line) => (
            <li
              key={line.title}
              className="flex items-start gap-3 px-3.5 py-2.5"
            >
              <span className="mt-0.5 flex size-4 shrink-0 items-center justify-center">
                {line.providerId ? (
                  <ProviderMark
                    id={line.providerId}
                    label={line.title}
                    className="size-4 text-ink"
                  />
                ) : (
                  <Check
                    className="size-4 text-ok"
                    strokeWidth={2.25}
                    aria-hidden
                  />
                )}
              </span>
              <span className="flex min-w-0 flex-col gap-px">
                <span className="font-sans text-[13px] font-medium leading-5 text-ink">
                  {line.title}
                </span>
                {line.detail ? (
                  <span className="break-words font-sans text-[13px] leading-5 text-neutral-600 dark:text-neutral-400">
                    {line.detail}
                  </span>
                ) : null}
              </span>
            </li>
          ))}
        </Rows>
      ) : null}

      {kits.length > 0 ? (
        <Rows as="ul">
          {kits.map((install) => (
            <KitRow
              key={install.kit}
              install={install}
              onRetry={onRetryKit}
              onReview={onReviewKit}
            />
          ))}
        </Rows>
      ) : null}

      {added.length > 0 ? (
        <Disclosure label="Workers added" summary={`${added.length} installed`}>
          <ul className="grid gap-x-4 gap-y-1 rounded-lg border border-neutral-200 bg-neutral-50 px-3.5 py-2.5 font-mono text-xs leading-5 dark:border-neutral-800 dark:bg-neutral-900 @md:grid-cols-2">
            {added.map((worker) => (
              <li key={worker} className="flex min-w-0 items-center gap-2">
                <span
                  aria-hidden
                  className="select-none text-neutral-500 dark:text-neutral-400"
                >
                  +
                </span>
                <span className="truncate text-ink">{worker}</span>
              </li>
            ))}
          </ul>
        </Disclosure>
      ) : null}
    </StepLayout>
  )
}

const KIT_TEXT: Record<
  KitInstall['state'],
  { title: (kit: string) => string; detail: string }
> = {
  installing: {
    title: (kit) => `Installing ${kit}`,
    detail: 'in the background; you can keep going',
  },
  installed: {
    title: (kit) => `${kit} installed`,
    detail: 'its agents are in the agent picker',
  },
  review: {
    title: (kit) => `${kit} needs a review`,
    detail: 'its install plan waits in Directory → Kits',
  },
  failed: {
    title: (kit) => `${kit} did not install`,
    detail: 'try again, or install it from Directory → Kits',
  },
}

/** One template kit: its background install, with the way forward. */
function KitRow({
  install,
  onRetry,
  onReview,
}: {
  install: KitInstall
  onRetry?: (kit: string) => void
  onReview?: (install: KitInstall) => void
}) {
  const text = KIT_TEXT[install.state]
  return (
    <li className="flex items-start gap-3 px-3.5 py-2.5">
      <span className="mt-0.5 flex size-4 shrink-0 items-center justify-center">
        {install.state === 'installing' ? (
          <LoaderCircle
            className="size-4 animate-spin text-neutral-500 motion-reduce:animate-none"
            aria-hidden
          />
        ) : install.state === 'installed' ? (
          <Check className="size-4 text-ok" strokeWidth={2.25} aria-hidden />
        ) : (
          <TriangleAlert className="size-4 text-warn" aria-hidden />
        )}
      </span>
      <span className="flex min-w-0 flex-1 flex-col gap-px">
        <span className="font-sans text-[13px] font-medium leading-5 text-ink">
          {text.title(install.kit)}
        </span>
        <span className="break-words font-sans text-[13px] leading-5 text-neutral-600 dark:text-neutral-400">
          {install.message ?? text.detail}
        </span>
      </span>
      {install.state === 'review' && onReview ? (
        <Button variant="ghost" onClick={() => onReview(install)}>
          Review
        </Button>
      ) : install.state === 'failed' && onRetry ? (
        <Button variant="ghost" onClick={() => onRetry(install.kit)}>
          Try again
        </Button>
      ) : null}
    </li>
  )
}
