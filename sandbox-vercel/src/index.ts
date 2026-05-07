import { Logger, registerWorker } from 'iii-sdk'
import { authFromEnv, VercelClient } from './client.js'
import { DEFAULT_CONFIG, loadConfig } from './config.js'
import {
  doCreate,
  doExec,
  doExposePort,
  doFsRead,
  doFsWrite,
  doList,
  doSnapshot,
  doStop,
  type HandlerCtx,
} from './handlers.js'

const cfgPath = process.env.SANDBOX_VERCEL_CONFIG ?? './config.yaml'
const config = loadConfig(cfgPath)
const auth = authFromEnv(config)
const client = new VercelClient(config.api_base, auth)
const ctx: HandlerCtx = { config, client, inFlight: { value: 0 } }

const iii = registerWorker(process.env.III_URL ?? 'ws://localhost:49134')
const logger = new Logger(undefined, 'sandbox-vercel')

function reg(id: string, handler: (input: Record<string, unknown>) => Promise<unknown>) {
  iii.registerFunction(id, async (input) => {
    try {
      return await handler((input ?? {}) as Record<string, unknown>)
    } catch (e) {
      const err = e as Error
      logger.warn?.(`${id} failed: ${err.message}`)
      throw err
    }
  })
}

reg('sandbox::vercel::create', (i) => doCreate(ctx, i))
reg('sandbox::vercel::exec', (i) => doExec(ctx, i))
reg('sandbox::vercel::stop', (i) => doStop(ctx, i))
reg('sandbox::vercel::list', (i) => doList(ctx, i))
reg('sandbox::vercel::snapshot', (i) => doSnapshot(ctx, i))
reg('sandbox::vercel::expose_port', (i) => doExposePort(ctx, i))
reg('sandbox::vercel::fs::read', (i) => doFsRead(ctx, i))
reg('sandbox::vercel::fs::write', (i) => doFsWrite(ctx, i))

logger.info?.('sandbox-vercel registered, awaiting invocations')

const _keepConfig = DEFAULT_CONFIG // keep symbol referenced for tree-shake friendliness

process.on('SIGTERM', async () => {
  logger.info?.('SIGTERM received, shutting down')
  process.exit(0)
})
