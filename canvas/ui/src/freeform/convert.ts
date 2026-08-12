/**
 * VENDOR-BUNDLE-ONLY module — imported by freeform-entry.ts, never from any
 * page.js code path (a direct import would drag excalidraw + mermaid into the
 * main bundle and blow the console's 8 MiB asset cap). Page code reaches
 * `convertMermaidToScene` through `loadFreeform(host)` (src/lib/loaders.ts).
 */

import {
  convertToExcalidrawElements,
  serializeAsJSON,
} from '@excalidraw/excalidraw'
import { parseMermaidToExcalidraw } from '@excalidraw/mermaid-to-excalidraw'

import { isImageFallback } from './scene'

/** Result of a mermaid→freeform conversion. */
export interface ConvertedScene {
  /**
   * Canonical excalidraw scene JSON (elements + appState + referenced files)
   * — drop it straight into a new freeform `CanvasRecord.source`.
   */
  source: string
  /**
   * True when the diagram family has no native converter and the library fell
   * back to a single rasterized image element (the drawing is a picture of
   * the diagram, not editable shapes). Surface a small note to the user when
   * set — the page owns that wiring.
   */
  imageFallback: boolean
}

/**
 * Convert mermaid source into an excalidraw scene — the "to canvas" action.
 *
 * Flow: `parseMermaidToExcalidraw` renders the diagram off-screen and emits
 * element skeletons (natively for flowchart/sequence/class/er/state families;
 * everything else becomes one image element — see `imageFallback`), then
 * `convertToExcalidrawElements` inflates the skeletons into full elements with
 * fresh ids, and `serializeAsJSON` produces the scene JSON the canvas worker
 * stores as a freeform record's `source`.
 *
 * Page-side wiring (the page agent's job): call this off `loadFreeform(host)`,
 * then `canvas::create` a new `format: "freeform"` record with the returned
 * `source` — conversion never mutates the mermaid record it started from.
 *
 * Rejects when mermaid cannot parse the source at all; callers show the error
 * and keep the original record untouched.
 */
export async function convertMermaidToScene(
  mermaidSource: string,
): Promise<ConvertedScene> {
  const { elements: skeleton, files } =
    await parseMermaidToExcalidraw(mermaidSource)
  const imageFallback = isImageFallback(skeleton)
  const elements = convertToExcalidrawElements(skeleton, {
    regenerateIds: true,
  })
  const source = serializeAsJSON(elements, {}, files ?? {}, 'local')
  return { source, imageFallback }
}
