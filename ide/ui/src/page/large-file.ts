/* Size policy for what the editor asks the worker for. The worker's
   default full-read budget (128 KiB) is sized for an LLM context, not a
   code editor; the explorer raises it per call and steps down to a
   read-only line window when a file is bigger than any editor should
   hold as one string. */

/** Bytes a single editor read may return; the worker clamps this to its
    own max_read_bytes (10 MiB by default). Monaco with its own viewport
    handles this size comfortably. */
export const EDITOR_FULL_READ_BUDGET = 8 * 1024 * 1024

/** Lines shown for a file over the budget. */
export const LARGE_FILE_PREVIEW_LINES = 5_000

/** Raster images above this are not previewed; the note offers the path. */
export const IMAGE_PREVIEW_MAX_BYTES = 64 * 1024 * 1024

/** Raw bytes per chunk of a streamed image read. Base64 grows this by a
    third; the frame stays well under 2 MiB either way. */
export const IMAGE_CHUNK_BYTES = 1024 * 1024

export type ReadPlan =
  | { kind: 'full' }
  | { kind: 'window'; lineTo: number }
  | { kind: 'too-large' }

/** What to ask for, given a size the caller already knows. `null` size
    (unknown) plans a full read and lets the worker's error redirect. */
export function readPlanForSize(size: number | null): ReadPlan {
  if (size === null || size <= EDITOR_FULL_READ_BUDGET) return { kind: 'full' }
  return { kind: 'window', lineTo: LARGE_FILE_PREVIEW_LINES }
}

/** The worker refuses a full read over budget with C218; that is the
    signal to step down to a window rather than an error to show. */
export function isTooLargeError(message: string): boolean {
  return message.includes('C218') || message.includes('exceeds max_output_bytes') || message.includes('exceeds max_read_bytes')
}

export function imagePlanForSize(size: number | null): 'stream' | 'too-large' {
  if (size !== null && size > IMAGE_PREVIEW_MAX_BYTES) return 'too-large'
  return 'stream'
}
