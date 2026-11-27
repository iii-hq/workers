// The one place on the check-and-deploy screen that concerns values: the names the compose file reads that iii cloud
// does not fill in by itself, as counts (never a list of twenty names), and the person's three ways out: set them in
// the review editor, skip the optional ones for now, or carry on. It locks nothing: the worker's check decides that.

import { Button, Skeleton } from '@iii-dev/console-ui'
import { useState } from 'react'
import { valuesPlan } from './findings'
import { plainMessage } from './model'
import { Mono, SectionCard } from './parts'
import type { ValuesRead } from './ValuesEditor'

const NO_WRITE = {
  api_key:
    "An API key can't write secrets and variables. Set them in iii cloud's console, or sign in with the browser.",
  role: "Changing production's secrets and variables takes Owner or Admin.",
} as const

export function ValuesCard({
  read,
  onSet,
  onSkip,
  onRunWithout,
  onAll,
  runFailed,
}: {
  read: ValuesRead
  /** The check's local run failed: "deploy without them" is not a thing to say then. */
  runFailed: boolean
  /** "Set values": the review editor over these names (required first, then asked, then optional). */
  onSet: (names: string[]) => void
  /** `values::skip` for these names; resolves when it went through. */
  onSkip: (names: string[], skip: boolean) => Promise<void>
  /** "Run without them here": skip these names, then check (the check makes the title and the lock true again). */
  onRunWithout: (names: string[]) => Promise<void>
  /** "All names": the Secrets and variables screen. */
  onAll: () => void
}) {
  const [busy, setBusy] = useState(false)
  const v = read.data
  const plan = valuesPlan(v)
  // Nothing was read and nothing is being read (an older worker): the card has nothing to say.
  if (!read.loading && !read.error && (!v || !plan.items.length)) return null
  const file = v?.manifest ?? 'the compose file'
  const toSet = [...plan.required, ...plan.asked, ...plan.local, ...plan.optional]
  // What "Skip for now" may skip: never a required name; a name the run here lacks runs with an empty value.
  const skippable = plan.optional
  const noWrite = v && v.save !== 'ok' ? NO_WRITE[v.save] : null
  const skip = async (names: string[], on: boolean) => {
    setBusy(true)
    await onSkip(names, on)
    setBusy(false)
  }
  const unreadable = v?.cloud === 'unreadable'
  return (
    <SectionCard
      title="Secrets and variables"
      count={v && !unreadable ? toSet.length : undefined}
      end={
        <button type="button" className="iii-cloud-linkbtn iii-cloud-small" onClick={onAll}>
          All names
        </button>
      }
      flush
    >
      <div className="iii-cloud-cs-vals">
        {read.loading && !v ? (
          <Skeleton className="iii-cloud-cs-skel" />
        ) : read.error || unreadable ? (
          <p className="iii-cloud-cs-vline">
            <span>
              {read.error
                ? `The names could not be read: ${plainMessage(read.error)}`
                : "iii cloud's variables could not be read, so what is set is not known."}
            </span>
            <button type="button" className="iii-cloud-linkbtn iii-cloud-small" onClick={read.refetch}>
              Try again
            </button>
          </p>
        ) : (
          <>
            {plan.required.length ? (
              <div className="iii-cloud-cs-vfact">
                <p className="iii-cloud-cs-vline" data-tone="alert">
                  {`${plan.required.length} required by ${file} and not set`}
                </p>
                <p className="iii-cloud-cs-vnames">
                  {plan.required.map((i) => (
                    <Mono key={i.name}>{i.name}</Mono>
                  ))}
                </p>
                <p className="iii-cloud-cs-sub">
                  iii cloud does not deploy without them: set them, or leave them empty on purpose.
                </p>
              </div>
            ) : null}
            {plan.asked.length ? (
              <div className="iii-cloud-cs-vfact">
                <p className="iii-cloud-cs-vline">{`${plan.asked.length} your agent asked for`}</p>
                {plan.asked.map((i) => (
                  <p key={i.name} className="iii-cloud-cs-sub">
                    <Mono>{i.name}</Mono>
                    {i.asked?.description ? `: ${i.asked.description}` : ''}
                  </p>
                ))}
              </div>
            ) : null}
            {plan.local.length ? (
              <div className="iii-cloud-cs-vfact">
                <p className="iii-cloud-cs-vline" data-tone="alert">
                  {`${plan.local.length} set in iii cloud but missing on this machine`}
                </p>
                <p className="iii-cloud-cs-vnames">
                  {plan.local.map((i) => (
                    <Mono key={i.name}>{i.name}</Mono>
                  ))}
                </p>
                <p className="iii-cloud-cs-sub">
                  The run here needs them in an env file of your folder, or skip them to run with an empty value.
                </p>
                <div className="iii-cloud-cs-acts">
                  <Button
                    size="sm"
                    variant="secondary"
                    disabled={busy}
                    onClick={() => {
                      setBusy(true)
                      void onRunWithout(plan.local.map((i) => i.name)).finally(() => setBusy(false))
                    }}
                  >
                    Run without them here
                  </Button>
                </div>
              </div>
            ) : null}
            {plan.optional.length ? (
              <p className="iii-cloud-cs-vline">{`${plan.optional.length} optional, not set`}</p>
            ) : null}
            {plan.skipped.length ? (
              <p className="iii-cloud-cs-vline">
                <span>{`${plan.skipped.length} skipped for now`}</span>
                <button
                  type="button"
                  className="iii-cloud-linkbtn iii-cloud-small"
                  disabled={busy}
                  onClick={() =>
                    void skip(
                      plan.skipped.map((i) => i.name),
                      false,
                    )
                  }
                >
                  Undo
                </button>
              </p>
            ) : null}
            {plan.set.length ? <p className="iii-cloud-cs-vline">{`${plan.set.length} set`}</p> : null}
            {plan.offered.length ? (
              <p className="iii-cloud-cs-sub">{`${plan.offered.length} can come from your env files`}</p>
            ) : null}
            {!toSet.length && !plan.skipped.length ? (
              <p className="iii-cloud-cs-sub">{`Every name ${file} reads is set.`}</p>
            ) : null}
            {runFailed && toSet.length ? (
              <p className="iii-cloud-cs-sub">
                The local run failed. If the application needs one of the names above, set it: the run's last lines say
                which.
              </p>
            ) : !plan.required.length && !plan.asked.length && plan.optional.length && !plan.local.length ? (
              <p className="iii-cloud-cs-sub">Optional ones can wait: you can deploy without them.</p>
            ) : null}
            {noWrite ? <p className="iii-cloud-cs-sub">{noWrite}</p> : null}
            {toSet.length ? (
              <div className="iii-cloud-cs-acts">
                <Button
                  size="sm"
                  variant="secondary"
                  disabled={!!noWrite || busy}
                  onClick={() => onSet(toSet.map((i) => i.name))}
                >
                  Set values
                </Button>
                {skippable.length ? (
                  <Button
                    size="sm"
                    variant="secondary"
                    // A skip only writes the worker's own store: an API key or a role may do it.
                    disabled={busy}
                    onClick={() =>
                      void skip(
                        skippable.map((i) => i.name),
                        true,
                      )
                    }
                  >
                    Skip for now
                  </Button>
                ) : null}
              </div>
            ) : null}
          </>
        )}
      </div>
    </SectionCard>
  )
}
