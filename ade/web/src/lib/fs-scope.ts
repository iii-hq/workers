/**
 * The filesystem scope a browser-side call hands the shell worker.
 *
 * The harness stamps every agent call with `fs_scope` (`harness/src/
 * filesystem_scope.rs`); when the console calls a `coder::*` function
 * itself — file mentions, the composer's file search — it has to send the
 * same shape. `boundary` is REQUIRED by the worker's `FsScope` (no serde
 * default): a scope without it fails to deserialize and the call errors
 * before it reaches the jail. `workspace` is what the harness sends for a
 * chat's working directory: the root is the folder and grants extend it.
 */

export interface FsScopeWire {
  root: string
  grants?: string[]
  boundary: 'workspace' | 'configured_roots'
}

/** The scope for a conversation's working directory. */
export function workspaceScope(workingDir: string): FsScopeWire {
  return { root: workingDir, boundary: 'workspace' }
}
