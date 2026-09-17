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

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  type Host,
  type SessionChipProps,
  type SessionChipRegistration,
  StatusDot,
} from '@iii-dev/console-ui'
import { Box } from 'lucide-react'
import { useEffect, useState } from 'react'
import { jumpToSandbox } from '../lib/selection'
import { formatAgeSecs, truncateId } from './format'
import { parseSandboxList, type SandboxSummary } from './store'

const CHIP_EVENTS_FN = 'iii::sandbox-code-runner-ui::chip-events'
const CHIP_EVENT_TRIGGER_TYPE = 'sandbox-code-runner::event'

export function createSandboxSessionChip(host: Host): SessionChipRegistration {
  function SandboxChip(_props: SessionChipProps) {
    const [sandboxes, setSandboxes] = useState<SandboxSummary[] | null>(null)

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

    if (sandboxes === null) return null
    const running = sandboxes.filter((s) => !s.stopped)

    return (
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <button
            type="button"
            className="cr-page-chip-btn"
            title={`${running.length} sandbox${running.length === 1 ? '' : 'es'} running`}
          >
            <Box size={16} aria-hidden /> {running.length}
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" aria-label="sandbox fleet">
          {sandboxes.length === 0 ? (
            <DropdownMenuItem disabled>no sandboxes</DropdownMenuItem>
          ) : (
            sandboxes.map((sandbox) => (
              <DropdownMenuItem
                key={sandbox.sandbox_id}
                className="cr-page-chip-row"
                onSelect={() => jumpToSandbox(sandbox.sandbox_id)}
              >
                <StatusDot tone={sandbox.stopped ? 'ink' : 'ok'} />
                <span className="cr-page-chip-id">
                  {sandbox.name || truncateId(sandbox.sandbox_id)}
                </span>
                <span className="cr-page-chip-meta">
                  {sandbox.image} · {formatAgeSecs(sandbox.age_secs)}
                </span>
              </DropdownMenuItem>
            ))
          )}
        </DropdownMenuContent>
      </DropdownMenu>
    )
  }
  return { id: 'sandbox-fleet', render: SandboxChip }
}
