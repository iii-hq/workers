/**
 * Ordered registry for worker-owned observer source renderers, matched by
 * the observed function id.
 *
 * Renderers can customize the source section, the complete expanded Terminal
 * view, and the compact timeline display. Injected registrations run in
 * registration order per slot; `null` falls through to the next renderer and
 * then to the host fallback.
 */

import { useMemo } from 'react'
import { ExtErrorChip, ScopedExtension } from '@/lib/ui-loader'
import {
  type RegisteredObserverActivityRenderer,
  useExtObserverActivityRenderers,
} from '@/lib/ui-slots'
import type {
  ObserverActivityMessage,
  ObserverActivityRenderer,
} from '@/types/injectable-ui'

let boundaryGeneration = 0

/** A broken redactor must never fall back to the value it was meant to hide. */
export const OBSERVER_RAW_REDACTION_FAILED =
  '[redaction failed — value withheld]'

/** Fence an injected renderer before any host component dispatches to it. */
function fenceInjected(
  entry: RegisteredObserverActivityRenderer,
): ObserverActivityRenderer {
  const { renderer, scope, path } = entry
  const boundaryKey = `${path}:${renderer.id}:${++boundaryGeneration}`
  const matches = (functionId: string): boolean => {
    try {
      return renderer.isMatch(functionId)
    } catch {
      return false
    }
  }
  const wrap = (
    render: (activity: ObserverActivityMessage) => React.ReactNode | null,
  ) => {
    return (activity: ObserverActivityMessage): React.ReactNode | null => {
      if (!matches(activity.functionId)) return null
      let node: React.ReactNode | null = null
      try {
        node = render(activity)
      } catch (error) {
        console.error(`[iii-ui] observer renderer ${renderer.id} threw`, error)
        return (
          <ExtErrorChip
            path={path}
            error={error instanceof Error ? error : new Error(String(error))}
          />
        )
      }
      if (node == null) return null
      return (
        <ScopedExtension key={boundaryKey} scope={scope} path={path}>
          {node}
        </ScopedExtension>
      )
    }
  }
  return {
    id: renderer.id,
    isMatch: matches,
    tryRender: wrap((activity) => renderer.tryRender(activity)),
    tryRenderDetails: renderer.tryRenderDetails
      ? wrap((activity) => renderer.tryRenderDetails?.(activity) ?? null)
      : undefined,
    tryRenderDisplay: renderer.tryRenderDisplay
      ? wrap((activity) => renderer.tryRenderDisplay?.(activity) ?? null)
      : undefined,
    redactRaw: renderer.redactRaw
      ? (value: unknown) => {
          try {
            return renderer.redactRaw?.(value)
          } catch (error) {
            console.error(
              `[iii-ui] observer redactRaw of ${renderer.id} threw`,
              error,
            )
            return OBSERVER_RAW_REDACTION_FAILED
          }
        }
      : undefined,
  }
}

/** Registration-ordered, fenced renderers for a store snapshot. */
export function observerActivityRenderers(
  injected: readonly RegisteredObserverActivityRenderer[],
): readonly ObserverActivityRenderer[] {
  return injected.map(fenceInjected)
}

/** Live registration-ordered renderers, recomputed only when the slot changes. */
export function useObserverActivityRenderers(): readonly ObserverActivityRenderer[] {
  const injected = useExtObserverActivityRenderers()
  return useMemo(() => observerActivityRenderers(injected), [injected])
}

export interface RenderedObserverActivity {
  renderer: ObserverActivityRenderer
  node: React.ReactNode
}

/** Resolve the first matching, non-null source section. */
export function firstRenderedObserverActivity(
  renderers: readonly ObserverActivityRenderer[],
  activity: ObserverActivityMessage,
): RenderedObserverActivity | null {
  for (const renderer of renderers) {
    if (!renderer.isMatch(activity.functionId)) continue
    const node = renderer.tryRender(activity)
    if (node != null) return { renderer, node }
  }
  return null
}

/** Resolve an optional renderer slot with the same ordering as `tryRender`. */
export function firstRenderedObserverActivitySlot(
  renderers: readonly ObserverActivityRenderer[],
  activity: ObserverActivityMessage,
  pick: (renderer: ObserverActivityRenderer) => React.ReactNode | null,
): RenderedObserverActivity | null {
  for (const renderer of renderers) {
    if (!renderer.isMatch(activity.functionId)) continue
    const node = pick(renderer)
    if (node != null) return { renderer, node }
  }
  return null
}

/** Redactor declared by the first renderer that claims this observed function. */
export function observerActivityRawRedactor(
  renderers: readonly ObserverActivityRenderer[],
  functionId: string,
): ((value: unknown) => unknown) | undefined {
  for (const renderer of renderers) {
    if (renderer.redactRaw && renderer.isMatch(functionId)) {
      return (value) => renderer.redactRaw?.(value)
    }
  }
  return undefined
}
