/**
 * Saved annotation sets: an `AnnotationSet` persisted in the `state` worker
 * (scope `annotations`), so a set outlives its session — anyone on the same
 * engine can list it from the palette, reopen it over its stored picture,
 * and send or download it later.
 *
 * The full record (picture included) lives under its own key; a single
 * `index` document holds the light summaries, so listing is one read no
 * matter how many sets exist. The index is updated with compare-and-set and
 * a few retries; a lost race falls back to an overwrite of the merged list.
 */

import type { ExtensionIii } from '@iii-dev/console-ui'
import { z } from 'zod'
import type { Annotation, AnnotationSet } from './annotations'

export const ANNOTATION_SETS_SCOPE = 'annotations'
const INDEX_KEY = 'index'
const CAS_RETRIES = 3

/** The stored picture is bounded: a frame larger than this is re-encoded
 * down before it enters shared engine state. */
export const MAX_IMAGE_DATA_URL_LENGTH = 1_500_000
const COMPRESS_MAX_WIDTH = 1280

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

const summarySchema = z.object({
  key: z.string(),
  subject: z.string(),
  count: z.number(),
  capturedAt: z.number(),
})
const indexSchema = z.array(summarySchema)

export type SavedSetSummary = z.infer<typeof summarySchema>

export function savedSetKey(set: AnnotationSet): string {
  // capturedAt orders; the suffix keeps two same-instant saves distinct.
  return `set-${set.capturedAt}-${Math.random().toString(36).slice(2, 6)}`
}

/** Re-encode an oversized picture down to a bounded JPEG. */
async function boundedImageUrl(dataUrl: string): Promise<string> {
  if (dataUrl.length <= MAX_IMAGE_DATA_URL_LENGTH) return dataUrl
  const image = await new Promise<HTMLImageElement>((resolve, reject) => {
    const el = new Image()
    el.onload = () => resolve(el)
    el.onerror = () => reject(new Error('image failed to load'))
    el.src = dataUrl
  })
  const scale = Math.min(1, COMPRESS_MAX_WIDTH / image.naturalWidth)
  const canvas = document.createElement('canvas')
  canvas.width = Math.max(1, Math.round(image.naturalWidth * scale))
  canvas.height = Math.max(1, Math.round(image.naturalHeight * scale))
  const context = canvas.getContext('2d')
  if (!context) return dataUrl
  context.drawImage(image, 0, 0, canvas.width, canvas.height)
  return canvas.toDataURL('image/jpeg', 0.8)
}

/** The stored index, an empty list when absent, or null when the read
 * itself failed - callers must not mistake a failed read for emptiness. */
async function readIndex(iii: ExtensionIii): Promise<SavedSetSummary[] | null> {
  let failed = false
  const res = await iii
    .trigger<unknown>('state::get', { scope: ANNOTATION_SETS_SCOPE, key: INDEX_KEY })
    .catch(() => {
      failed = true
      return null
    })
  if (failed) return null
  const parsed = indexSchema.safeParse(res)
  return parsed.success ? parsed.data : []
}

/** Apply `change` to the index with compare-and-set; a persistently lost
 * race degrades to an overwrite of the freshest merge. */
async function updateIndex(
  iii: ExtensionIii,
  change: (index: SavedSetSummary[]) => SavedSetSummary[],
): Promise<void> {
  for (let attempt = 0; attempt <= CAS_RETRIES; attempt += 1) {
    const current = await readIndex(iii)
    if (current === null) {
      throw new Error('the saved-sets index could not be read; nothing changed')
    }
    const next = change(current)
    if (attempt === CAS_RETRIES) {
      await iii.trigger('state::set', {
        scope: ANNOTATION_SETS_SCOPE,
        key: INDEX_KEY,
        value: next,
      })
      return
    }
    const res = await iii
      .trigger<unknown>('state::compare-and-set', {
        scope: ANNOTATION_SETS_SCOPE,
        key: INDEX_KEY,
        // An empty read means absent or empty; expect-absent covers the
        // first save, and a mismatch (swapped: false) just retries.
        expected: current.length === 0 ? undefined : current,
        value: next,
      })
      .catch(() => null)
    const parsed = z.object({ swapped: z.boolean() }).safeParse(res)
    if (parsed.success && parsed.data.swapped) return
    // another writer won (or the shape surprised us); re-read and retry
  }
}

export async function saveAnnotationSet(
  iii: ExtensionIii,
  set: AnnotationSet,
): Promise<string> {
  const key = savedSetKey(set)
  const imageUrl = await boundedImageUrl(set.imageUrl)
  await iii.trigger('state::set', {
    scope: ANNOTATION_SETS_SCOPE,
    key,
    value: {
      subject: set.subject,
      imageUrl,
      width: set.width,
      height: set.height,
      annotations: set.annotations,
      capturedAt: set.capturedAt,
    },
  })
  const summary: SavedSetSummary = {
    key,
    subject: set.subject,
    count: set.annotations.length,
    capturedAt: set.capturedAt,
  }
  await updateIndex(iii, (index) => [
    summary,
    ...index.filter((entry) => entry.key !== key),
  ])
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

/** Newest first; one read of the summary index. */
export async function listAnnotationSets(
  iii: ExtensionIii,
  signal?: AbortSignal,
): Promise<SavedSetSummary[]> {
  const index = await readIndex(iii)
  if (index === null) throw new Error('the saved-sets list could not be read')
  if (signal?.aborted) return []
  return [...index].sort((a, b) => b.capturedAt - a.capturedAt)
}

export async function deleteAnnotationSet(
  iii: ExtensionIii,
  key: string,
): Promise<void> {
  await iii.trigger('state::delete', {
    scope: ANNOTATION_SETS_SCOPE,
    key,
  })
  await updateIndex(iii, (index) =>
    index.filter((entry) => entry.key !== key),
  )
}
