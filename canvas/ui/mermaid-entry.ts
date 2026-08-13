/**
 * Vendor bundle entry: ALL of mermaid, self-contained (no externals — mermaid
 * never touches React), built by build.mjs into dist/mermaid.js and served as
 * the `canvas/mermaid.js` console:script asset.
 *
 * The console eagerly import()s every console:script asset and calls its
 * default export as setup(host), so the default export here is a no-op setup;
 * the real payload is the named `mermaid` export, reached lazily through
 * src/lib/loaders.ts (loadMermaid).
 */

import mermaid from 'mermaid'

export { mermaid }

export default function setup(): void {}
