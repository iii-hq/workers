/**
 * The `sandbox-fleet` session chip: `⬚ N` where N counts RUNNING (not
 * stopped) sandboxes — one `sandbox::list` read on mount, then pure push
 * from the worker's fleet-watcher events (no polling, house doctrine).
 * Clicking opens a popover listing the fleet; picking a sandbox jumps to
 * #/ext/sandbox with it selected (jumpToSandbox — the page reads the
 * selection on mount).
 *
 * Mirrors the harness context chip's registration shape: a factory that
 * closes over `host` and returns the SessionChipRegistration for
 * `host.chat?.registerSessionChip`. The chip renders nothing while the
 * daemon is absent (`sandbox::list` failing) — a dead `⬚` would just be
 * chrome noise in every chat.
 */

import type {
  Host,
  SessionChipProps,
  SessionChipRegistration,
} from '@iii-dev/console-ui'
import { useEffect, useRef, useState } from 'react'
import { jumpToSandbox } from '../lib/selection'
import { formatAgeSecs, truncateId } from './format'
import { parseSandboxList, type SandboxSummary } from './store'

const CHIP_EVENTS_FN = 'iii::sandbox-code-runner-ui::chip-events'
const CHIP_EVENT_TRIGGER_TYPE = 'sandbox-code-runner::event'

export function createSandboxSessionChip(host: Host): SessionChipRegistration {
  function SandboxChip(_props: SessionChipProps) {
    const [sandboxes, setSandboxes] = useState<SandboxSummary[] | null>(null)
    const [open, setOpen] = useState(false)
    const rootRef = useRef<HTMLSpanElement | null>(null)

    useEffect(() => {
      let cancelled = false
      host.iii
        .trigger('sandbox::list', {})
        .then((value) => {
          if (!cancelled) setSandboxes(parseSandboxList(value))
        })
        .catch(() => {
          if (!cancelled) setSandboxes(null)
        })
      let offHandler: (() => void) | null = null
      let offTrigger: (() => void) | null = null
      try {
        offHandler = host.iii.on(CHIP_EVENTS_FN, (payload: unknown) => {
          if (cancelled) return
          const rec =
            payload && typeof payload === 'object'
              ? (payload as Record<string, unknown>)
              : null
          if (rec?.kind !== 'fleet') return
          if (rec.daemon_absent === true) setSandboxes(null)
          else setSandboxes(parseSandboxList({ sandboxes: rec.sandboxes }))
        })
        offTrigger = host.iii.registerTrigger({
          type: CHIP_EVENT_TRIGGER_TYPE,
          function_id: `${CHIP_EVENTS_FN}::${host.iii.browserId}`,
          config: {},
        })
      } catch {
        /* worker gone mid-session — the mount read stands */
      }
      return () => {
        cancelled = true
        offTrigger?.()
        offHandler?.()
      }
    }, [])

    useEffect(() => {
      if (!open) return
      const onPointerDown = (event: MouseEvent) => {
        const root = rootRef.current
        if (root && !root.contains(event.target as Node)) setOpen(false)
      }
      const onKeyDown = (event: KeyboardEvent) => {
        if (event.key === 'Escape') setOpen(false)
      }
      document.addEventListener('mousedown', onPointerDown)
      document.addEventListener('keydown', onKeyDown)
      return () => {
        document.removeEventListener('mousedown', onPointerDown)
        document.removeEventListener('keydown', onKeyDown)
      }
    }, [open])

    if (sandboxes === null) return null
    const running = sandboxes.filter((s) => !s.stopped)

    return (
      <span className="cr-page-chip-root" ref={rootRef}>
        <button
          type="button"
          className="cr-page-chip-btn"
          onClick={() => setOpen((v) => !v)}
          aria-expanded={open}
          title={`${running.length} sandbox${running.length === 1 ? '' : 'es'} running`}
        >
          ⬚ {running.length}
        </button>
        {open ? (
          <div className="cr-page-chip-pop" role="dialog" aria-label="sandbox fleet">
            {sandboxes.length === 0 ? (
              <div className="cr-page-chip-empty">no sandboxes</div>
            ) : (
              sandboxes.map((sandbox) => (
                <button
                  key={sandbox.sandbox_id}
                  type="button"
                  className="cr-page-chip-row"
                  onClick={() => {
                    setOpen(false)
                    jumpToSandbox(sandbox.sandbox_id)
                  }}
                >
                  <span className={`cr-page-chip-dot${sandbox.stopped ? ' stopped' : ''}`} />
                  <span className="cr-page-chip-id">
                    {sandbox.name || truncateId(sandbox.sandbox_id)}
                  </span>
                  <span className="cr-page-chip-meta">
                    {sandbox.image} · {formatAgeSecs(sandbox.age_secs)}
                  </span>
                </button>
              ))
            )}
          </div>
        ) : null}
      </span>
    )
  }

  return { id: 'sandbox-fleet', render: SandboxChip }
}
