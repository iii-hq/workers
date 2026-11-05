/**
 * Live workspace updates over this worker's own `shell::on-changed`: a
 * system-level directory watch (FSEvents/inotify) the worker runs for its
 * observers, so EVERY change streams — agent calls, `shell::exec` side
 * effects, and edits made outside the engine entirely. The page observes
 * one watch per pane on the browsed root, through a function named after
 * the pane so two panes of one tab never hear each other's folder (the
 * `iii::` prefix keeps per-event invocations span-suppressed; the observer
 * is disposed with the pane, and made again when the root changes). No
 * polling, and no dependency on any other worker.
 */

import { useEffect, useRef } from 'react'
import type { Host } from '@iii-dev/console-ui'
import { publishWorktree } from './git-watch'

const EVENTS_FN = 'iii::shell-ui::changed'

/** The slice of `shell::on-changed`'s event this page acts on. */
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
    // The branch chips on this folder read their dirty mark again from
    // these events, with no watch of their own.
    const chips = publishWorktree(host, root)
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
        if (event.ignored !== true) chips.note()
        handlerRef.current(event)
      },
    )
    const offObserver = host.iii.observe(
      {
        function_id: 'shell::on-changed',
        // The workspace view counts ignored churn (build output, engine
        // data) and shows it, so it opts in; every other observer gets only
        // the changes that matter by default.
        args: { path: root, include_ignored: true },
      },
      `${functionId}::${host.iii.browserId}`,
    )
    return () => {
      offObserver()
      offHandler()
      chips.off()
    }
  }, [host, root, scope])
}
