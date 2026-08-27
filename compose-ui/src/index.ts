import { watch } from 'node:fs'
import { readFile } from 'node:fs/promises'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { uiPage, uiStyles } from 'virtual:compose-ui-assets'
import { registerWorker } from 'iii-sdk'
import { DEFAULT_LINES, MAX_LINES, readLogTail } from './logs.js'
import { type ChangedEvent, createStateWatcher, type ProjectLocation } from './watch.js'

const WORKER = 'compose-ui'
const CHANGED_TRIGGER = 'compose-ui::changed'

const iii = registerWorker(process.env.III_URL ?? process.env.III_ENGINE_URL, {
  workerName: WORKER,
  workerDescription:
    'Compose project supervision in the Console: live container state, lifecycle actions, worker packages, and log tails.',
})

const object = (properties: Record<string, unknown>, required: string[] = []) => ({
  type: 'object' as const,
  properties,
  ...(required.length ? { required } : {}),
})
const string = { type: 'string' }
const nullableString = { type: ['string', 'null'] }

type Status = { file?: string; namespace?: string; state_dir?: string }

async function locate(file?: string | null): Promise<ProjectLocation | null> {
  const status = await iii.trigger<Record<string, unknown>, Status>({
    function_id: 'compose::status',
    payload: file ? { file } : {},
    timeoutMs: 10_000,
  })
  if (!status?.state_dir || !status.file || !status.namespace) return null
  return { file: status.file, namespace: status.namespace, stateDir: status.state_dir }
}

type Binding = { id: string; function_id: string; namespace?: string }
const bindings = new Map<string, Binding>()

const watcher = createStateWatcher({
  locate: () => locate(),
  emit: (event: ChangedEvent) => {
    for (const binding of bindings.values()) {
      void iii
        .trigger({
          function_id: binding.function_id,
          payload: event,
          timeoutMs: 10_000,
          ...(binding.namespace ? { namespace: binding.namespace } : {}),
        })
        .catch((error) => console.error(`[${WORKER}] ${binding.function_id} rejected a change event: ${String(error)}`))
    }
  },
  log: (message) => console.error(`[${WORKER}] ${message}`),
})

iii.registerTriggerType<Record<string, never>>(
  {
    id: CHANGED_TRIGGER,
    description:
      'Fires when the compose daemon writes its durable project state or the compose file changes on disk. Payload: kind (state|file), file, namespace, state_dir, path, captured_at. Bind with an empty config.',
  },
  {
    async registerTrigger({ id, function_id, namespace }) {
      bindings.set(id, { id, function_id, namespace })
      const location = await watcher.ensure()
      if (!location) console.error(`[${WORKER}] compose daemon not reachable yet; watching starts on the next binding`)
    },
    async unregisterTrigger({ id }) {
      bindings.delete(id)
    },
  },
)

iii.registerFunction(
  'compose-ui::logs',
  async (input: { container: string; lines?: number; file?: string | null }) => {
    const location = input.file ? await locate(input.file) : await watcher.ensure()
    if (!location) throw new Error('COMPOSE_UNAVAILABLE: compose::status answered without a state directory')
    return readLogTail(location.stateDir, input.container, input.lines)
  },
  {
    description: `Last lines of one compose container's log from the daemon's state directory (default ${DEFAULT_LINES}, at most ${MAX_LINES}; a missing log answers with missing: true).`,
    request_format: object(
      {
        container: { ...string, description: 'Container name as declared in the compose file.' },
        lines: {
          type: 'integer',
          minimum: 1,
          maximum: MAX_LINES,
          description: `Lines from the end; default ${DEFAULT_LINES}.`,
        },
        file: { ...nullableString, description: 'Compose file on the daemon host; defaults to the daemon project.' },
      },
      ['container'],
    ),
    response_format: object(
      {
        container: string,
        path: string,
        lines: { type: 'array', items: string },
        size: { type: 'integer' },
        truncated: { type: 'boolean' },
        missing: { type: 'boolean' },
      },
      ['container', 'path', 'lines', 'size', 'truncated', 'missing'],
    ),
  },
)

type UiAsset = { file: string; type: 'console:script' | 'console:style'; content_type: string; content: string }

const uiAssets: Record<string, UiAsset> = {
  'compose-ui/page.js': { file: 'page.js', type: 'console:script', content_type: 'text/javascript', content: uiPage },
  'compose-ui/styles.css': { file: 'styles.css', type: 'console:style', content_type: 'text/css', content: uiStyles },
}
const uiWatch = process.env.III_COMPOSE_UI_UI_WATCH
const uiWatchEnabled = Boolean(uiWatch)
const uiWatchDir =
  uiWatchEnabled && uiWatch !== '1' && uiWatch !== 'true'
    ? (uiWatch as string)
    : join(dirname(fileURLToPath(import.meta.url)), '..', '..', 'ui', 'dist')

async function uiContent(path: string) {
  const asset = uiAssets[path]
  if (!asset) throw new Error(`unknown ui asset: ${path}`)
  const content = uiWatchEnabled ? await readFile(join(uiWatchDir, asset.file), 'utf8') : asset.content
  return { content, content_type: asset.content_type }
}

iii.registerFunction('compose-ui::ui-content', (input: { path: string }) => uiContent(input.path), {
  description: 'Serve the injectable Compose Console page assets.',
  metadata: { internal: true },
  request_format: object({ path: string }, ['path']),
  response_format: object({ content: string, content_type: string }, ['content', 'content_type']),
})

function registerUiAsset(path: string) {
  return iii.registerTrigger({ type: uiAssets[path].type, function_id: 'compose-ui::ui-content', config: { path } })
}

const uiTriggers = new Map(Object.keys(uiAssets).map((path) => [path, registerUiAsset(path)]))

if (uiWatchEnabled) {
  const pending = new Map<string, NodeJS.Timeout>()
  watch(uiWatchDir, (_event, file) => {
    const path = Object.keys(uiAssets).find((key) => uiAssets[key].file === file)
    if (!path) return
    clearTimeout(pending.get(path))
    pending.set(
      path,
      setTimeout(() => {
        const previous = uiTriggers.get(path)
        uiTriggers.set(path, registerUiAsset(path))
        previous?.unregister()
        console.error(`[${WORKER}] reloaded ui asset ${path}`)
      }, 150),
    )
  })
  console.error(`[${WORKER}] serving ui assets from ${uiWatchDir}`)
}

void watcher.ensure().catch((error) => console.error(`[${WORKER}] compose daemon not reachable yet: ${String(error)}`))

const shutdown = async () => {
  watcher.close()
  await iii.shutdown()
  process.exit(0)
}
process.on('SIGINT', shutdown)
process.on('SIGTERM', shutdown)
