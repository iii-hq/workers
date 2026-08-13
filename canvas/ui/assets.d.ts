/**
 * Non-code imports the vendor entry points pull in. esbuild's `text` loader
 * turns the excalidraw stylesheet into a string export (injected at runtime
 * by src/lib/loaders.ts); this declaration keeps `tsc --noEmit` in agreement.
 */

declare module '*.css' {
  const css: string
  export default css
}
