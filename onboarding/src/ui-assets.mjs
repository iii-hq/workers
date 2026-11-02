/**
 * The injected console page: the content function that serves its assets,
 * and one `console::ui::asset` observer per asset.
 *
 * Kept apart from `index.mjs` (which connects on import) so a test can drive
 * it with a stub `iii`.
 */

import { readFile } from 'node:fs/promises'
import { join } from 'node:path'

export const UI_CONTENT = 'learn-iii::ui-content'

/** How long an asset observer may stay unanswered before the worker says so. */
export const READY_WARN_MS = 5_000

export const uiAssets = {
  'learn-iii/page.js': {
    file: 'page.js',
    kind: 'script',
    content_type: 'text/javascript',
  },
  'learn-iii/styles.css': {
    file: 'styles.css',
    kind: 'style',
    content_type: 'text/css',
  },
}

const object = (properties = {}, required = []) => ({
  type: 'object',
  properties,
  ...(required.length ? { required } : {}),
})
const string = { type: 'string' }

/**
 * Assets are read from disk at call time rather than inlined at build time,
 * so the watcher in `index.mjs` is the same code path as production.
 *
 * A standby, as the console-ui contract registers its content function: a
 * second instance of this worker (scale-out, or an overlap during a restart)
 * waits to take the function over instead of conflicting on it.
 */
export const registerUiContent = (iii, uiDir) =>
  iii.registerFunction(
    UI_CONTENT,
    async (input) => {
      const asset = uiAssets[input?.path]
      if (!asset) throw new Error(`unknown ui asset: ${input?.path}`)
      const content = await readFile(join(uiDir, asset.file), 'utf8')
      return { content, content_type: asset.content_type }
    },
    {
      description: 'Serve the injectable learn-iii console page assets.',
      metadata: { internal: true },
      standby: true,
      request_format: object({ path: string }, ['path']),
      response_format: object({ content: string, content_type: string }, ['content', 'content_type']),
    },
  )

/**
 * One `console::ui::asset` observer per asset; the console fetches its
 * content by calling through the observer with `{ path }`.
 *
 * Nothing else says when the page cannot load, so the handle's states are
 * logged: a refusal (`error`), the engine dropping the observer (`disposed`,
 * other than this worker's own dispose on hot reload), and an observer still
 * unanswered after `warnAfterMs` — no console connected, or one that does not
 * host `console::ui::asset`.
 */
export const observeUiAsset = (
  iii,
  path,
  { worker = 'onboarding', warnAfterMs = READY_WARN_MS, log = console.error } = {},
) => {
  const handle = iii.observe(
    { function_id: 'console::ui::asset', args: { kind: uiAssets[path].kind, path } },
    UI_CONTENT,
  )
  handle.onStateChange((state, reason) => {
    const why = `${reason?.code ?? ''} ${reason?.message ?? ''}`.trim()
    // Hot reload observes the path afresh, which supersedes the previous one.
    if (state === 'error' && reason?.code !== 'UI_ASSET_SUPERSEDED') {
      log(`[${worker}] ui asset ${path} rejected: ${why}`)
    } else if (state === 'disposed' && reason?.code !== 'UNOBSERVED') {
      log(`[${worker}] ui asset ${path} observer disposed: ${why || 'no reason given'}`)
    }
  })
  let settled = false
  const timer = setTimeout(() => {
    if (!settled) {
      log(
        `[${worker}] ui asset ${path} is still not served after ${warnAfterMs} ms: ` +
          'no console is connected, or it does not host console::ui::asset (it needs the 0.26 ADE)',
      )
    }
  }, warnAfterMs)
  timer.unref?.()
  // A rejection is logged by the state listener above.
  handle
    .ready()
    .catch(() => {})
    .finally(() => {
      settled = true
      clearTimeout(timer)
    })
  return handle
}
