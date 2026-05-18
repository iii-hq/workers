/// <reference types="vite/client" />

interface ImportMetaEnv {
  /**
   * Truthy in dev (and any build that opts in) to ship the Examples + Playground
   * pages and the mock backend. Empty/falsy in prod so Rolldown tree-shakes
   * those modules out of the production chunk. See PLAYGROUND.md.
   */
  readonly VITE_PLAYGROUND?: string
  /**
   * Override the engine WebSocket URL used by `iii-client.ts`. Defaults to
   * `wss://${window.location.host}/iii/ws` (HTTPS) or the `ws://` variant,
   * which lets dev hit the Vite `/iii/ws` proxy and prod reach whatever
   * reverse proxy is forwarding to the engine. Set this for direct, non-
   * proxied connections (e.g. `wss://engine.example.com/iii/ws`).
   */
  readonly VITE_ENGINE_WS_URL?: string
}

interface ImportMeta {
  readonly env: ImportMetaEnv
}
