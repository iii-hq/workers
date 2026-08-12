/**
 * VENDOR-BUNDLE-ONLY module — imported FIRST by freeform-entry.ts, never from
 * any page.js code path.
 *
 * Excalidraw fetches its handwriting fonts (dist/prod/fonts/…, content-hashed
 * filenames) at runtime, resolving them against `window.EXCALIDRAW_ASSET_PATH`.
 * The injected console asset is a single JS file with no sibling font files to
 * serve, so the path points at the unpkg CDN pinned to the EXACT bundled
 * excalidraw version — the font filenames are hashed per release, so a version
 * drift means 404s, not wrong glyphs.
 *
 * The import-order placement matters: ES module bodies run depth-first in
 * import order, so this assignment lands before the excalidraw module body
 * evaluates — the path is set however early the library chooses to read it.
 *
 * NETWORK DEPENDENCY + GRACEFUL DEGRADATION: fonts are the only runtime
 * network fetch this bundle performs. On an offline (or CDN-blocked) console
 * the font requests fail and excalidraw falls back to system fonts — drawing,
 * saving, conversion, and export all keep working; only the handwritten look
 * degrades.
 *
 * `EXCALIDRAW_ASSET_PATH` is a page-global shared by every excalidraw copy on
 * the page; this assignment is unconditional so OUR bundle's hashes win at our
 * load time.
 */

/** Keep in lockstep with `@excalidraw/excalidraw` in ui/package.json. */
export const EXCALIDRAW_VERSION = '0.18.1'

declare global {
  interface Window {
    EXCALIDRAW_ASSET_PATH?: string | string[]
  }
}

window.EXCALIDRAW_ASSET_PATH = `https://unpkg.com/@excalidraw/excalidraw@${EXCALIDRAW_VERSION}/dist/prod/`
