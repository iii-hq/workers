/**
 * The navigation rail: the FLEET list (one row per sandbox — status dot,
 * id, image, age, free exec slots), the RUNTIMES strip (code-runner
 * runtimes with their guest-registered functions: expand, trigger, tear
 * down), and the CATALOG (daemon images with their preset/custom badge).
 *
 * Status reads: running (accent), busy (warn, pulsing — execs in flight),
 * stopped (ink). Empty fleet and daemon-absent each get their own empty
 * state; the rail never renders a bare nothing.
 */

import { Badge, EmptyState, type Host, StatusDot } from '@iii-dev/console-ui'
import { useState } from 'react'
import {
  displayAgeSecs,
  formatAgeSecs,
  REAP_WARN_SECS,
  reapCountdownSecs,
  type SandboxState,
  sandboxState,
} from './format'
import { BoxIcon, ChevronIcon } from './icons'
import { InvokeDialog } from './InvokeDialog'
import type {
  CatalogImage,
  FleetTombstone,
  RuntimeInfo,
  SandboxSummary,
} from './store'
import { deriveReapInSecs, EXEC_SLOTS } from './store'

const STATUS_DOT: Record<
  SandboxState,
  { tone: 'accent' | 'warn' | 'ink'; pulse: boolean }
> = {
  running: { tone: 'accent', pulse: false },
  busy: { tone: 'warn', pulse: true },
  stopped: { tone: 'ink', pulse: false },
}

function SandboxRow({
  sandbox,
  selected,
  snapshotAt,
  clock,
  onSelect,
}: {
  sandbox: SandboxSummary
  selected: boolean
  snapshotAt: number | null
  clock: number
  onSelect(id: string): void
}) {
  const state = sandboxState(sandbox)
  const dot = STATUS_DOT[state]
  const reapBase = sandbox.stopped ? null : deriveReapInSecs(sandbox)
  const reapSoon =
    reapBase !== null &&
    reapCountdownSecs(reapBase, snapshotAt, clock) < REAP_WARN_SECS
  return (
    <button
      type="button"
      className={`cr-page-fleet-row${selected ? ' selected' : ''}${reapSoon ? ' reap-soon' : ''}`}
      onClick={() => onSelect(sandbox.sandbox_id)}
      title={`${sandbox.sandbox_id} · ${state}${reapSoon ? ' · idle reap imminent' : ''}`}
      aria-current={selected ? 'true' : undefined}
    >
      <StatusDot tone={dot.tone} pulse={dot.pulse} />
      <span className="cr-page-fleet-id">{sandbox.name || sandbox.sandbox_id}</span>
      <span className="cr-page-fleet-meta">
        <span className="cr-page-fleet-image" title={sandbox.image}>
          {sandbox.image}
        </span>
        <span className="cr-page-fleet-age">
          {formatAgeSecs(displayAgeSecs(sandbox.age_secs, snapshotAt, clock))}
        </span>
        <span
          className={`cr-page-fleet-slots${sandbox.exec_slots_free === 0 ? ' full' : ''}`}
          title={`${sandbox.exec_slots_free} of ${EXEC_SLOTS} exec slots free`}
        >
          {sandbox.exec_slots_free}/{EXEC_SLOTS}
        </span>
      </span>
    </button>
  )
}

function RuntimeRow({
  runtime,
  onInvoke,
  onTeardown,
  busy,
}: {
  runtime: RuntimeInfo
  onInvoke(functionId: string): void
  onTeardown(runtimeId: string): void
  busy: boolean
}) {
  const [expanded, setExpanded] = useState(false)
  const [confirming, setConfirming] = useState(false)
  const count = runtime.registered_functions.length
  return (
    <div className="cr-page-rt">
      <div className="cr-page-rt-head">
        <button
          type="button"
          className="cr-page-rt-toggle"
          onClick={() => setExpanded((v) => !v)}
          aria-expanded={expanded}
          disabled={count === 0}
          title={count === 0 ? 'no functions registered' : 'show functions'}
        >
          <ChevronIcon className={expanded ? 'cr-page-chev open' : 'cr-page-chev'} aria-hidden />
          <span className="cr-page-rt-lang">{runtime.lang}</span>
          <span className="cr-page-rt-count">
            {count} fn{count === 1 ? '' : 's'}
          </span>
          {runtime.vm_gone ? (
            <span
              className="cr-page-rt-gone"
              title="the backing VM left sandbox::list (idle-reaped or stopped); teardown cleans this record up"
            >
              vm gone
            </span>
          ) : null}
        </button>
        {confirming ? (
          <span className="cr-page-stop-confirm">
            <button
              type="button"
              className="cr-page-linkish"
              onClick={() => setConfirming(false)}
              disabled={busy}
            >
              cancel
            </button>
            <button
              type="button"
              className="cr-page-danger-link"
              onClick={() => {
                setConfirming(false)
                onTeardown(runtime.runtime_id)
              }}
              disabled={busy}
              title="sandbox-code-runner::teardown — stops the runtime and unregisters its functions"
            >
              confirm
            </button>
          </span>
        ) : (
          <button
            type="button"
            className="cr-page-danger-link"
            onClick={() => setConfirming(true)}
            disabled={busy}
            title="sandbox-code-runner::teardown — stops the runtime and unregisters its functions"
          >
            teardown
          </button>
        )}
      </div>
      {expanded && count > 0 ? (
        <ul className="cr-page-rt-fns">
          {runtime.registered_functions.map((fn) => (
            <li key={fn} className="cr-page-rt-fn">
              <span className="cr-page-rt-fn-id" title={fn}>
                {fn}
              </span>
              <button
                type="button"
                className="cr-page-linkish"
                onClick={() => onInvoke(fn)}
                title={`trigger ${fn} over the bus with a JSON payload`}
              >
                trigger
              </button>
            </li>
          ))}
        </ul>
      ) : null}
    </div>
  )
}

export function FleetRail({
  host,
  sandboxes,
  runtimes,
  tombstones = [],
  images,
  snapshotAt,
  clock,
  selected,
  daemonAbsent,
  onSelect,
  onRefresh,
}: {
  host: Host
  sandboxes: SandboxSummary[]
  runtimes: RuntimeInfo[]
  tombstones?: FleetTombstone[]
  images: CatalogImage[]
  snapshotAt: number | null
  /** The page's 1s display clock — ages tick against it between events. */
  clock: number
  selected: string | null
  daemonAbsent: boolean
  onSelect(id: string): void
  onRefresh(): void
}) {
  const [invokeFn, setInvokeFn] = useState<string | null>(null)
  const [teardownBusy, setTeardownBusy] = useState(false)
  const [teardownError, setTeardownError] = useState<string | null>(null)

  const teardown = (runtimeId: string) => {
    setTeardownBusy(true)
    setTeardownError(null)
    host.iii
      .trigger('sandbox-code-runner::teardown', { runtime_id: runtimeId })
      .then(() => onRefresh())
      .catch((err: unknown) =>
        setTeardownError(err instanceof Error ? err.message : String(err)),
      )
      .finally(() => setTeardownBusy(false))
  }

  return (
    <div className="cr-page-rail">
      <div className="cr-page-rail-label">fleet</div>
      {daemonAbsent ? (
        <div className="cr-page-rail-empty">
          <EmptyState
            icon={BoxIcon}
            title="no sandbox daemon"
            description="sandbox::list is not on this engine — add it with `iii worker add iii-sandbox`, then sandboxes appear here."
          />
        </div>
      ) : sandboxes.length === 0 && tombstones.length === 0 ? (
        // The workspace already renders the full create empty-state; the
        // rail keeps a one-liner so the two never read as an echo.
        <div className="cr-page-rail-none">none running</div>
      ) : (
        <div className="cr-page-fleet">
          {sandboxes.map((sandbox) => (
            <SandboxRow
              key={sandbox.sandbox_id}
              sandbox={sandbox}
              selected={sandbox.sandbox_id === selected}
              snapshotAt={snapshotAt}
              clock={clock}
              onSelect={onSelect}
            />
          ))}
          {tombstones.map((t) => (
            <div
              key={t.sandbox.sandbox_id}
              className="cr-page-fleet-row ghost"
              title={`${t.sandbox.sandbox_id} — left the fleet (idle-reaped or stopped)`}
            >
              <StatusDot tone="ink" />
              <span className="cr-page-fleet-id">
                {t.sandbox.name || t.sandbox.sandbox_id}
              </span>
              <span className="cr-page-fleet-meta">
                <span className="cr-page-fleet-gone">gone — reaped or stopped</span>
              </span>
            </div>
          ))}
        </div>
      )}

      {runtimes.length > 0 ? (
        <>
          <div className="cr-page-rail-label">runtimes</div>
          <div className="cr-page-rts">
            {runtimes.map((runtime) => (
              <RuntimeRow
                key={runtime.runtime_id}
                runtime={runtime}
                onInvoke={setInvokeFn}
                onTeardown={teardown}
                busy={teardownBusy}
              />
            ))}
            {teardownError ? (
              <div className="cr-page-inline-error" role="alert">
                teardown failed: {teardownError}
              </div>
            ) : null}
          </div>
        </>
      ) : null}

      {images.length > 0 ? (
        <>
          <div className="cr-page-rail-label">catalog</div>
          <ul className="cr-page-catalog">
            {images.map((image) => (
              <li key={image.name} className="cr-page-catalog-row" title={image.oci_ref}>
                <span className="cr-page-catalog-name">{image.name}</span>
                <Badge variant={image.kind === 'preset' ? 'default' : 'accent'}>
                  {image.kind}
                </Badge>
              </li>
            ))}
          </ul>
        </>
      ) : null}

      <InvokeDialog host={host} functionId={invokeFn} onClose={() => setInvokeFn(null)} />
    </div>
  )
}
