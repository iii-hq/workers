/**
 * Ordered registry for worker-owned trigger source renderers.
 *
 * Renderers can customize the source section, the complete expanded Terminal
 * view, and the compact timeline display. Injected registrations run in
 * registration order per slot; `null` falls through to the next renderer and
 * then to the host fallback.
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

/** A broken redactor must never fall back to the value it was meant to hide. */
export const TRIGGER_RAW_REDACTION_FAILED =
  '[redaction failed — value withheld]'

/** Fence an injected renderer before any host component dispatches to it. */
function fenceInjected(
  entry: RegisteredTriggerActivityRenderer,
): TriggerActivityRenderer {
  const { renderer, scope, path } = entry
  const boundaryKey = `${path}:${renderer.id}:${++boundaryGeneration}`
  const matches = (triggerType: string): boolean => {
    try {
      return renderer.isMatch(triggerType)
    } catch {
      return false
    }
  }
  const wrap = (
    render: (activity: TriggerActivityMessage) => React.ReactNode | null,
  ) => {
    return (activity: TriggerActivityMessage): React.ReactNode | null => {
      if (!matches(activity.triggerType)) return null
      let node: React.ReactNode | null = null
      try {
        node = render(activity)
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
              `[iii-ui] trigger redactRaw of ${renderer.id} threw`,
              error,
            )
            return TRIGGER_RAW_REDACTION_FAILED
          }
        }
      : undefined,
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

/** Resolve an optional renderer slot with the same ordering as `tryRender`. */
export function firstRenderedTriggerActivitySlot(
  renderers: readonly TriggerActivityRenderer[],
  activity: TriggerActivityMessage,
  pick: (renderer: TriggerActivityRenderer) => React.ReactNode | null,
): RenderedTriggerActivity | null {
  for (const renderer of renderers) {
    if (!renderer.isMatch(activity.triggerType)) continue
    const node = pick(renderer)
    if (node != null) return { renderer, node }
  }
  return null
}

/** Redactor declared by the first renderer that claims this trigger type. */
export function triggerActivityRawRedactor(
  renderers: readonly TriggerActivityRenderer[],
  triggerType: string,
): ((value: unknown) => unknown) | undefined {
  for (const renderer of renderers) {
    if (renderer.redactRaw && renderer.isMatch(triggerType)) {
      return (value) => renderer.redactRaw?.(value)
    }
  }
  return undefined
}
