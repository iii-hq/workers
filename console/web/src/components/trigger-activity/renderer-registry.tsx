/**
 * Ordered registry for worker-owned trigger source renderers.
 *
 * Renderers customize only the source section of a host-owned trigger
 * activity. Injected registrations run in registration order; a matching
 * renderer that returns `null` falls through, and the host renders its generic
 * source fallback when every renderer falls through.
 */

import { useMemo } from 'react'
import { ExtErrorChip, ScopedExtension } from '@/lib/ui-loader'
import {
  type RegisteredTriggerActivityRenderer,
  useExtTriggerActivityRenderers,
} from '@/lib/ui-slots'
import type {
  TriggerActivityMessage,
  TriggerActivityRenderer,
} from '@/types/injectable-ui'

let boundaryGeneration = 0

/** Fence an injected renderer before any host component dispatches to it. */
function fenceInjected(
  entry: RegisteredTriggerActivityRenderer,
): TriggerActivityRenderer {
  const { renderer, scope, path } = entry
  const boundaryKey = `${path}:${renderer.id}:${++boundaryGeneration}`
  return {
    id: renderer.id,
    isMatch(triggerType) {
      try {
        return renderer.isMatch(triggerType)
      } catch {
        return false
      }
    },
    tryRender(activity) {
      let node: React.ReactNode | null = null
      try {
        node = renderer.tryRender(activity)
      } catch (error) {
        console.error(`[iii-ui] trigger renderer ${renderer.id} threw`, error)
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
    },
  }
}

/** Registration-ordered, fenced renderers for a store snapshot. */
export function triggerActivityRenderers(
  injected: readonly RegisteredTriggerActivityRenderer[],
): readonly TriggerActivityRenderer[] {
  return injected.map(fenceInjected)
}

/** Live registration-ordered renderers, recomputed only when the slot changes. */
export function useTriggerActivityRenderers(): readonly TriggerActivityRenderer[] {
  const injected = useExtTriggerActivityRenderers()
  return useMemo(() => triggerActivityRenderers(injected), [injected])
}

export interface RenderedTriggerActivity {
  renderer: TriggerActivityRenderer
  node: React.ReactNode
}

/** Resolve the first matching, non-null source section. */
export function firstRenderedTriggerActivity(
  renderers: readonly TriggerActivityRenderer[],
  activity: TriggerActivityMessage,
): RenderedTriggerActivity | null {
  for (const renderer of renderers) {
    if (!renderer.isMatch(activity.triggerType)) continue
    const node = renderer.tryRender(activity)
    if (node != null) return { renderer, node }
  }
  return null
}
