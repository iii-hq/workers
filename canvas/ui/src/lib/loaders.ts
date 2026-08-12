/**
 * Lazy loaders for the two vendor bundles (`canvas/mermaid.js`,
 * `canvas/freeform.js`).
 *
 * The console serves every injected asset at `GET <base>/ui/<path>` and its
 * own loader resolves that URL against `document.baseURI` (the console
 * supports arbitrary subpath mounting) — these loaders do the same. `host` is
 * accepted for future host-provided resolution, but the current Host contract
 * only carries `host.path` as a plain string naming the CALLING script's own
 * asset path (checked against packages/console-ui/index.d.ts), so sibling
 * assets are resolved by URL.
 *
 * Both loaders memoize the in-flight promise (a failed load clears it so the
 * next call retries), and loadFreeform injects the excalidraw stylesheet into
 * the document exactly once, marked with data-iii-ui="canvas" so it is
 * attributable in the DOM.
 */

import type { Host } from '@iii-dev/console-ui'

export const MERMAID_ASSET_PATH = 'canvas/mermaid.js'
export const FREEFORM_ASSET_PATH = 'canvas/freeform.js'

/** The named surface of the mermaid vendor bundle (see ../../mermaid-entry.ts). */
export interface MermaidBundle {
  mermaid: typeof import('mermaid').default
}

/** The named surface of the freeform vendor bundle (see ../../freeform-entry.ts). */
export interface FreeformBundle {
  Excalidraw: typeof import('@excalidraw/excalidraw').Excalidraw
  convertMermaidToScene: typeof import('../freeform/convert').convertMermaidToScene
  convertToExcalidrawElements: typeof import('@excalidraw/excalidraw').convertToExcalidrawElements
  exportToBlob: typeof import('@excalidraw/excalidraw').exportToBlob
  exportToSvg: typeof import('@excalidraw/excalidraw').exportToSvg
  parseMermaidToExcalidraw: typeof import('@excalidraw/mermaid-to-excalidraw').parseMermaidToExcalidraw
  serializeAsJSON: typeof import('@excalidraw/excalidraw').serializeAsJSON
  excalidrawCss: string
}

/** Resolve a sibling asset URL the way the console's own loader does. */
function assetUrl(path: string): string {
  return new URL(`ui/${path}`, new URL('.', document.baseURI)).href
}

let mermaidLoad: Promise<MermaidBundle> | null = null

/** Import the mermaid vendor bundle, once. */
export function loadMermaid(_host: Host): Promise<MermaidBundle> {
  mermaidLoad ??= import(/* @vite-ignore */ assetUrl(MERMAID_ASSET_PATH)).then(
    (mod) => mod as unknown as MermaidBundle,
    (err) => {
      mermaidLoad = null
      throw err
    },
  )
  return mermaidLoad
}

const FREEFORM_CSS_MARKER = 'data-canvas-freeform-css'

/**
 * Put the excalidraw stylesheet in the document, exactly once.
 *
 * WHY it rides JS injection instead of the `console:style` channel: injected
 * style assets must scope every rule under `[data-iii-ui="canvas"]`, and
 * excalidraw's vendor sheet cannot be rewritten that way — so it ships as a
 * text export of the vendor bundle and lands here, tagged
 * `data-iii-ui="canvas"` per the portal-tagging rule (any DOM a worker puts
 * outside its wrapper must carry its scope attribute for attribution).
 */
function injectFreeformCss(css: string): void {
  if (document.head.querySelector(`style[${FREEFORM_CSS_MARKER}]`)) return
  const style = document.createElement('style')
  style.setAttribute('data-iii-ui', 'canvas')
  style.setAttribute(FREEFORM_CSS_MARKER, '')
  style.textContent = css
  document.head.appendChild(style)
}

let freeformLoad: Promise<FreeformBundle> | null = null

/** Import the freeform (excalidraw) vendor bundle, once, injecting its CSS. */
export function loadFreeform(_host: Host): Promise<FreeformBundle> {
  freeformLoad ??= import(/* @vite-ignore */ assetUrl(FREEFORM_ASSET_PATH)).then(
    (mod) => {
      const bundle = mod as unknown as FreeformBundle
      injectFreeformCss(bundle.excalidrawCss)
      return bundle
    },
    (err) => {
      freeformLoad = null
      throw err
    },
  )
  return freeformLoad
}
