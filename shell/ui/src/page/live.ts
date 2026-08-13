/**
 * Live workspace updates over the editor worker's `editor::changed` push
 * channel. The editor worker observes every filesystem-touching harness
 * call (`shell::*`, `coder::*`) through `harness::hook::post-trigger` and
 * fans out one event per write — path, cause, kind, and the workspace
 * root. This page subscribes with a tab-scoped Message-path binding (the
 * `iii::` prefix keeps per-event invocations span-suppressed; the binding
 * is GC'd with the tab) and refreshes the tree, the git listing, and any
 * open file the agent just wrote. No polling anywhere.
 *
 * Degrades to the old load-once behavior when the editor worker is not
 * installed: the trigger type never fires and nothing subscribes twice.
 */

import { useEffect, useRef } from 'react'
import type { Host } from '@iii-dev/console-ui'

const EVENTS_FN = 'iii::shell-ui::changed'

/** The slice of `editor::changed` this page acts on. */
export interface WorkspaceChangedEvent {
  /** Path relative to `root`. */
  path: string
  /** The function that caused the write, e.g. `coder::update-file`. */
  cause: string
  /** `created`, `modified`, `deleted`, or `unknown`. */
  kind: string
  /** Workspace root the path is relative to. */
  root: string
}

export function useWorkspaceChanges(
  host: Host,
  onEvent: (e: WorkspaceChangedEvent) => void,
) {
  const handlerRef = useRef(onEvent)
  handlerRef.current = onEvent
  useEffect(() => {
    const offHandler = host.iii.on<WorkspaceChangedEvent>(
      EVENTS_FN,
      (event) => {
        if (typeof event?.path !== 'string') return
        handlerRef.current(event)
      },
    )
    const offTrigger = host.iii.registerTrigger({
      type: 'editor::changed',
      function_id: `${EVENTS_FN}::${host.iii.browserId}`,
      config: {},
    })
    return () => {
      offTrigger()
      offHandler()
    }
  }, [host])
}
