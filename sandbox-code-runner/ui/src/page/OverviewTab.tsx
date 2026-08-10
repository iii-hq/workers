/**
 * The overview tab: identity list (full id, state, image + catalog kind,
 * age, slot meter, and the rows the daemon does not report yet — labeled
 * honestly rather than omitted), the action row (jump to console/files,
 * copy the create command, stop with an inline confirm), and the limits
 * note.
 */

import { Badge, Button, type Host } from '@iii-dev/console-ui'
import { useEffect, useRef, useState } from 'react'
import { formatTriggerCommand } from './exec'
import { formatAgeSecs, sandboxState } from './format'
import type { CatalogImage, SandboxSummary } from './store'
import { EXEC_SLOTS } from './store'
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
  catalogImage,
  onGoConsole,
  onGoFiles,
  onStopped,
}: {
  host: Host
  sandbox: SandboxSummary
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
      .catch((err: unknown) =>
        setStopError(err instanceof Error ? err.message : String(err)),
      )
      .finally(() => setStopping(false))
  }

  const createCommand = formatTriggerCommand('sandbox::create', {
    image: sandbox.image,
    ...(sandbox.name ? { name: sandbox.name } : {}),
  })

  const state = sandboxState(sandbox)

  return (
    <div className="cr-page-overview">
      <dl className="cr-page-identity">
        <dt>sandbox id</dt>
        <dd className="cr-page-identity-id">
          <code>{sandbox.sandbox_id}</code>
          <CopyButton text={sandbox.sandbox_id} title="copy the sandbox id" />
        </dd>

        <dt>state</dt>
        <dd>
          <span className={`cr-page-state-pill ${state}`}>{state}</span>
        </dd>

        <dt>image</dt>
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

        <dt>age</dt>
        <dd>{formatAgeSecs(sandbox.age_secs)}</dd>

        <dt>exec slots</dt>
        <dd>
          <SlotsMeter free={sandbox.exec_slots_free} />
        </dd>

        {/* sandbox::list does not carry these yet — say so instead of
            guessing (an omitted row reads as "no network", which is a claim). */}
        <dt>network</dt>
        <dd className="cr-page-faint">not reported by sandbox::list</dd>

        <dt>idle timeout</dt>
        <dd className="cr-page-faint">not reported by sandbox::list</dd>
      </dl>

      <div className="cr-page-actions">
        <Button variant="ghost" size="sm" onClick={onGoConsole} disabled={sandbox.stopped}>
          exec →
        </Button>
        <Button variant="ghost" size="sm" onClick={onGoFiles} disabled={sandbox.stopped}>
          files →
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
        <div className="cr-page-inline-error" role="alert">
          stop failed: {stopError}
        </div>
      ) : null}

      <p className="cr-page-limit-note">
        limits — {EXEC_SLOTS} concurrent execs per sandbox · inline output and
        file reads cap at 1 MiB · idle sandboxes are reaped by the daemon's
        idle timer.
      </p>
    </div>
  )
}
