import { Logger, registerWorker } from 'iii-sdk'
import { authFromEnv, CfBridgeClient } from './client.js'
import { loadConfig } from './config.js'
import { doCreate, doExec, doExposePort, doFsRead, doFsWrite, doList, doStop, type HandlerCtx } from './handlers.js'

const cfgPath = process.env.SANDBOX_CF_CONFIG ?? './config.yaml'
const config = loadConfig(cfgPath)
const auth = authFromEnv(config)
const client = new CfBridgeClient(auth)
const ctx: HandlerCtx = { config, client, inFlight: { value: 0 } }

const iii = registerWorker(process.env.III_URL ?? 'ws://localhost:49134')
const logger = new Logger(undefined, 'sandbox-cf')

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

reg('sandbox::cf::create', (i) => doCreate(ctx, i))
reg('sandbox::cf::exec', (i) => doExec(ctx, i))
reg('sandbox::cf::stop', (i) => doStop(ctx, i))
reg('sandbox::cf::list', (i) => doList(ctx, i))
reg('sandbox::cf::expose_port', (i) => doExposePort(ctx, i))
reg('sandbox::cf::fs::read', (i) => doFsRead(ctx, i))
reg('sandbox::cf::fs::write', (i) => doFsWrite(ctx, i))

logger.info?.('sandbox-cf registered, awaiting invocations')

process.on('SIGTERM', () => {
  logger.info?.('SIGTERM received, shutting down')
  process.exit(0)
})
