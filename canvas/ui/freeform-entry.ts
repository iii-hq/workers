/**
 * Vendor bundle entry: the excalidraw whiteboard plus the mermaid→excalidraw
 * converter, built by build.mjs into dist/freeform.js and served as the
 * `canvas/freeform.js` console:script asset.
 *
 * Only the react family stays external — excalidraw must render inside the
 * console's React tree (a second React copy breaks hooks). The excalidraw
 * stylesheet ships as the `excalidrawCss` text export; src/lib/loaders.ts
 * injects it into the document exactly once.
 *
 * The asset-path import MUST stay first: it sets
 * `window.EXCALIDRAW_ASSET_PATH` (the CDN base excalidraw fetches its fonts
 * from) before the excalidraw module body evaluates — ES module bodies run
 * depth-first in import order.
 *
 * The console eagerly import()s every console:script asset and calls its
 * default export as setup(host), so the default export here is a no-op setup;
 * the real payload is the named exports, reached lazily through
 * src/lib/loaders.ts (loadFreeform).
 */

import './src/freeform/asset-path'

import {
  Excalidraw,
  convertToExcalidrawElements,
  exportToBlob,
  exportToSvg,
  serializeAsJSON,
} from '@excalidraw/excalidraw'
import { parseMermaidToExcalidraw } from '@excalidraw/mermaid-to-excalidraw'
import excalidrawCss from '@excalidraw/excalidraw/index.css'

import { convertMermaidToScene } from './src/freeform/convert'

export type { ConvertedScene } from './src/freeform/convert'

export {
  Excalidraw,
  convertMermaidToScene,
  convertToExcalidrawElements,
  excalidrawCss,
  exportToBlob,
  exportToSvg,
  parseMermaidToExcalidraw,
  serializeAsJSON,
}

export default function setup(): void {}
