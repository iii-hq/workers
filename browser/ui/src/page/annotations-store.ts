/**
 * Saved annotation sets: an `AnnotationSet` persisted in the `state` worker
 * (scope `annotations`), so a set outlives its session — anyone on the same
 * engine can list it from the palette, reopen it over its stored picture,
 * and send or download it later.
 */

import type { ExtensionIii } from '@iii-dev/console-ui'
import { z } from 'zod'
import type { Annotation, AnnotationSet } from './annotations'

export const ANNOTATION_SETS_SCOPE = 'annotations'

const annotationSchema: z.ZodType<Annotation> = z.object({
  id: z.string(),
  x: z.number(),
  y: z.number(),
  note: z.string(),
  kind: z.enum(['pin', 'rect', 'arrow']).optional(),
  x2: z.number().optional(),
  y2: z.number().optional(),
  color: z.string().optional(),
  label: z.string().optional(),
})

const savedSetSchema = z.object({
  subject: z.string(),
  imageUrl: z.string(),
  width: z.number(),
  height: z.number(),
  annotations: z.array(annotationSchema),
  capturedAt: z.number(),
})

export interface SavedSetSummary {
  key: string
  subject: string
  count: number
  capturedAt: number
}

export function savedSetKey(set: AnnotationSet): string {
  return `set-${set.capturedAt}`
}

export async function saveAnnotationSet(
  iii: ExtensionIii,
  set: AnnotationSet,
): Promise<string> {
  const key = savedSetKey(set)
  await iii.trigger('state::set', {
    scope: ANNOTATION_SETS_SCOPE,
    key,
    value: {
      subject: set.subject,
      imageUrl: set.imageUrl,
      width: set.width,
      height: set.height,
      annotations: set.annotations,
      capturedAt: set.capturedAt,
    },
  })
  return key
}

export async function readAnnotationSet(
  iii: ExtensionIii,
  key: string,
): Promise<AnnotationSet | null> {
  const res = await iii.trigger<unknown>('state::get', {
    scope: ANNOTATION_SETS_SCOPE,
    key,
  })
  const parsed = savedSetSchema.safeParse(res)
  return parsed.success ? parsed.data : null
}

/** Newest first. Reads each set to summarize it; the sets stay small (one
 * frame plus marks) and local, so the per-key reads are fine here. */
export async function listAnnotationSets(
  iii: ExtensionIii,
): Promise<SavedSetSummary[]> {
  const keysRes = await iii.trigger<unknown>('state::list_keys', {
    scope: ANNOTATION_SETS_SCOPE,
  })
  const keys = z.object({ keys: z.array(z.string()) }).safeParse(keysRes)
  if (!keys.success) return []
  const summaries: SavedSetSummary[] = []
  for (const key of keys.data.keys) {
    if (!key.startsWith('set-')) continue
    const set = await readAnnotationSet(iii, key)
    if (set) {
      summaries.push({
        key,
        subject: set.subject,
        count: set.annotations.length,
        capturedAt: set.capturedAt,
      })
    }
  }
  summaries.sort((a, b) => b.capturedAt - a.capturedAt)
  return summaries
}

export async function deleteAnnotationSet(
  iii: ExtensionIii,
  key: string,
): Promise<void> {
  await iii.trigger('state::delete', {
    scope: ANNOTATION_SETS_SCOPE,
    key,
  })
}
