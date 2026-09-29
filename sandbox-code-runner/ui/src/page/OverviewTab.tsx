/**
 * The overview tab: identity list (full id, state, image + catalog kind,
 * age, slot meter, and the rows the daemon does not report yet — labeled
 * honestly rather than omitted), the action row (jump to console/files,
 * copy the create command, stop with an inline confirm), and the limits
 * note.
 */

import { Badge, Button, type Host, StatusPanel } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { uiClasses } from '@iii-dev/console-ui/ui-classes'
import { ArrowRight } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { formatTriggerCommand } from './exec'
import {
  displayAgeSecs,
  formatAgeSecs,
  formatSecs,
  REAP_WARN_SECS,
  reapCountdownSecs,
  sandboxState,
} from './format'
import type { CatalogImage, SandboxSummary } from './store'
import { deriveReapInSecs, EXEC_SLOTS } from './store'
import { CopyButton } from './widgets'

function SlotsMeter({ free }: { free: number }) {
  const used = Math.min(EXEC_SLOTS, Math.max(0, EXEC_SLOTS - free))
  return (
    <span
      className="cr-page-slots-meter"
      title={`${free} of ${EXEC_SLOTS} exec slots free`}
    >
      {Array.from({ length: EXEC_SLOTS }, (_, i) => (
        <span
          // Fixed-length positional cells; index is the identity.
          // biome-ignore lint/suspicious/noArrayIndexKey: positional cells
          key={i}
          className={`cr-page-slot${i < used ? ' used' : ''}`}
        />
      ))}
      <span className="cr-page-slots-count">
        {free}/{EXEC_SLOTS} free
      </span>
    </span>
  )
}

export function OverviewTab({
  host,
  sandbox,
  snapshotAt,
  clock,
  catalogImage,
  onGoConsole,
  onGoFiles,
  onStopped,
}: {
  host: Host
  sandbox: SandboxSummary
  snapshotAt: number | null
  /** The page's 1s display clock — age and the reap countdown tick on it. */
  clock: number
  /** The catalog row matching this sandbox's image, when the catalog has it. */
  catalogImage: CatalogImage | null
  onGoConsole(): void
  onGoFiles(): void
  onStopped(): void
}) {
  const [confirmStop, setConfirmStop] = useState(false)
  const [stopping, setStopping] = useState(false)
  const [stopError, setStopError] = useState<string | null>(null)
  // A pending confirm belongs to ONE sandbox: the parent keys this tab by
  // sandbox_id, so a selection change remounts with fresh state.

  // Keyboard flow: the trigger button unmounts when the confirmation
  // appears, so focus must move onto the confirm button explicitly.
  const confirmRef = useRef<HTMLButtonElement>(null)
  useEffect(() => {
    if (confirmStop) confirmRef.current?.focus()
  }, [confirmStop])

  const stop = () => {
    setStopping(true)
    setStopError(null)
    host.iii
      .trigger(
        'sandbox::stop',
        { sandbox_id: sandbox.sandbox_id, wait: true },
        { timeoutMs: 60_000 },
      )
      .then(() => {
        setConfirmStop(false)
        onStopped()
      })
      .catch((err: unknown) => setStopError(errorMessage(err)))
      .finally(() => setStopping(false))
  }

  const createCommand = formatTriggerCommand('sandbox::create', {
    image: sandbox.image,
    ...(sandbox.name ? { name: sandbox.name } : {}),
  })

  const state = sandboxState(sandbox)
  const stateVariant = state === 'running' ? 'ok' : state === 'busy' ? 'warn' : 'default'
  const reapBase = sandbox.stopped ? null : deriveReapInSecs(sandbox)
  const reapLeft =
    reapBase === null ? null : reapCountdownSecs(reapBase, snapshotAt, clock)

  return (
    <div className="cr-page-overview">
      <dl className="cr-page-identity">
        <dt className={uiClasses.eyebrow}>sandbox id</dt>
        <dd className="cr-page-identity-id">
          <code>{sandbox.sandbox_id}</code>
          <CopyButton text={sandbox.sandbox_id} title="copy the sandbox id" />
        </dd>

        <dt className={uiClasses.eyebrow}>state</dt>
        <dd>
          <Badge variant={stateVariant}>{state}</Badge>
        </dd>

        <dt className={uiClasses.eyebrow}>image</dt>
        <dd className="cr-page-identity-image">
          <code>{sandbox.image || '—'}</code>
          {catalogImage ? (
            <Badge variant={catalogImage.kind === 'preset' ? 'default' : 'accent'}>
              {catalogImage.kind}
            </Badge>
          ) : null}
          {catalogImage?.oci_ref ? (
            <span className="cr-page-faint" title={catalogImage.oci_ref}>
              {catalogImage.oci_ref}
            </span>
          ) : null}
        </dd>

        <dt className={uiClasses.eyebrow}>age</dt>
        <dd>{formatAgeSecs(displayAgeSecs(sandbox.age_secs, snapshotAt, clock))}</dd>

        <dt className={uiClasses.eyebrow}>exec slots</dt>
        <dd>
          <SlotsMeter free={sandbox.exec_slots_free} />
        </dd>

        {/* sandbox::list does not carry these yet — say so instead of
            guessing (an omitted row reads as "no network", which is a claim). */}
        <dt className={uiClasses.eyebrow}>network</dt>
        <dd className="cr-page-faint">not reported by sandbox::list</dd>

        {reapLeft === null ? (
          <>
            <dt className={uiClasses.eyebrow}>idle timeout</dt>
            <dd className="cr-page-faint">not reported by sandbox::list</dd>
          </>
        ) : (
          <>
            <dt className={uiClasses.eyebrow}>idle deadline</dt>
            <dd className={reapLeft < REAP_WARN_SECS ? 'cr-page-reap-warn' : undefined}>
              {reapLeft <= 0
                ? 'reaping…'
                : `reaped in ${formatSecs(Math.ceil(reapLeft))} unless something execs`}
            </dd>
          </>
        )}
      </dl>

      <div className="cr-page-actions">
        <Button variant="ghost" size="sm" onClick={onGoConsole} disabled={sandbox.stopped}>
          exec <ArrowRight size={16} aria-hidden />
        </Button>
        <Button variant="ghost" size="sm" onClick={onGoFiles} disabled={sandbox.stopped}>
          files <ArrowRight size={16} aria-hidden />
        </Button>
        <CopyButton
          text={createCommand}
          label="copy create command"
          title={createCommand}
        />
        <span className="cr-page-actions-gap" />
        {sandbox.stopped ? null : confirmStop ? (
          <span className="cr-page-stop-confirm">
            <span>stop this sandbox? the filesystem is gone with it.</span>
            <Button variant="ghost" size="sm" onClick={() => setConfirmStop(false)} disabled={stopping}>
              cancel
            </Button>
            <button
              type="button"
              className="cr-page-danger-btn"
              onClick={stop}
              disabled={stopping}
              ref={confirmRef}
            >
              {stopping ? 'stopping…' : 'confirm stop'}
            </button>
          </span>
        ) : (
          <button
            type="button"
            className="cr-page-danger-btn"
            onClick={() => setConfirmStop(true)}
            title="sandbox::stop { wait: true } — destroys the microVM and its filesystem"
          >
            stop
          </button>
        )}
      </div>
      {stopError ? (
        <StatusPanel variant="alert" role="alert" headline="stop failed" detail={stopError} />
      ) : null}

      <p className="cr-page-limit-note">
        limits — {EXEC_SLOTS} concurrent execs per sandbox · inline output and
        file reads cap at 1 MiB · idle sandboxes are reaped by the daemon's
        idle timer.
      </p>
    </div>
  )
}
