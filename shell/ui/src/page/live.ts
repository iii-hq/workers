/**
 * Live workspace updates over this worker's own `shell::changed` trigger
 * type: a system-level directory watch (FSEvents/inotify) the worker runs
 * for each binding, so EVERY change streams — agent calls, `shell::exec`
 * side effects, and edits made outside the engine entirely. The page
 * binds one watch per pane on the browsed root, through a function named
 * after the pane so two panes of one tab never hear each other's folder
 * (the `iii::` prefix keeps per-event invocations span-suppressed; the
 * binding is GC'd with the pane, and re-binds when the root changes). No
 * polling, and no dependency on any other worker.
 */

import { useEffect, useRef } from 'react'
import type { Host } from '@iii-dev/console-ui'

const EVENTS_FN = 'iii::shell-ui::changed'

/** The slice of `shell::changed` this page acts on. */
export interface WorkspaceChangedEvent {
  /** Path relative to `root`. */
  path: string
  /** `created`, `modified`, or `deleted`. */
  kind: string
  /** The watched directory the path is relative to. */
  root: string
  /** True for directories — they refresh the tree but must never open
      as files. Absent on older workers: treat as false. */
  dir?: boolean
  /** True when git ignores the path under `root`. Absent on older
      workers: treat as false. */
  ignored?: boolean
}

export function useWorkspaceChanges(
  host: Host,
  root: string | null,
  onEvent: (e: WorkspaceChangedEvent) => void,
  /** The pane's function-id segment (`paneScopeToken`). */
  scope: string,
) {
  const handlerRef = useRef(onEvent)
  handlerRef.current = onEvent
  useEffect(() => {
    if (!root) return
    const functionId = `${EVENTS_FN}::${scope}`
    const offHandler = host.iii.on<WorkspaceChangedEvent>(
      functionId,
      (event) => {
        if (
          typeof event?.path !== 'string' ||
          typeof event.kind !== 'string' ||
          typeof event.root !== 'string'
        ) {
          return
        }
        handlerRef.current(event)
      },
    )
    const offTrigger = host.iii.registerTrigger({
      type: 'shell::changed',
      function_id: `${functionId}::${host.iii.browserId}`,
      config: { path: root },
    })
    return () => {
      offTrigger()
      offHandler()
    }
  }, [host, root, scope])
}
