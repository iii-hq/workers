/// <reference types="vite/client" />

interface ImportMetaEnv {
  /**
   * Truthy in dev (and any build that opts in) to ship the Examples + Playground
   * pages and the mock backend. Empty/falsy in prod so Rolldown tree-shakes
   * those modules out of the production chunk. See PLAYGROUND.md.
   */
  readonly VITE_PLAYGROUND?: string
}

interface ImportMeta {
  readonly env: ImportMetaEnv
}
