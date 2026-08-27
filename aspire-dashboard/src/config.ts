import { randomBytes } from 'node:crypto'
import { readFile } from 'node:fs/promises'
import { isIPv4 } from 'node:net'
import { parse } from 'yaml'
import { z } from 'zod'

const port = z.number().int().min(1).max(65535)

// The dashboard process runs with `--allow-anonymous`, and OTLP ingestion is
// unauthenticated whenever secure_otlp is false - which is the default, because
// iii-observability cannot authenticate to a secure endpoint. A non-loopback
// bind_host would therefore put an open dashboard and an open trace sink on the
// network. Remote viewing goes through the Console proxy instead.
function isLoopbackHost(host: string) {
  if (host === 'localhost' || host === '::1') return true
  // isIPv4 checks the octet ranges, which a `127(\.\d{1,3}){3}` pattern does
  // not: it would take 127.999.999.999 and fail later, at listen time.
  return isIPv4(host) && host.startsWith('127.')
}

const RuntimeFields = z
  .object({
    aspire_command: z
      .array(z.string())
      .min(1)
      .default(['npx', '-y', '@microsoft/aspire-cli', 'dashboard', 'run'])
      .describe('Command that launches the standalone Aspire Dashboard process (no Docker)'),
    bind_host: z
      .string()
      .default('127.0.0.1')
      .refine(isLoopbackHost, {
        message:
          'bind_host must be a loopback address (127.x.x.x, ::1, or localhost): the dashboard runs anonymously and OTLP ingestion is unauthenticated',
      })
      .describe('Loopback interface for the dashboard and OTLP ports'),
    dashboard_port: port.default(18888).describe('Host port for the Aspire Dashboard web UI'),
    proxy_port: port.default(18887).describe('Host port for the frame-safe reverse proxy used by the Console page'),
    otlp_port: port.default(4317).describe('Host port for the Aspire Dashboard OTLP/gRPC endpoint'),
    otlp_http_port: port
      .default(4318)
      .describe('Host port for the Aspire Dashboard OTLP/HTTP endpoint used by iii-observability logs export'),
    // Defaults to false: iii-observability cannot authenticate to a secure OTLP
    // endpoint. Its exporter never sets gRPC metadata, and its config schema is
    // additionalProperties:false, so no key can be stored there either. The only
    // channel that reaches the exporter is OTEL_EXPORTER_OTLP_HEADERS on the
    // engine process, which this worker cannot set. The ports are loopback-bound.
    secure_otlp: z.boolean().default(false).describe('Require an Aspire OTLP API key for telemetry ingestion'),
    otlp_api_key: z
      .string()
      .min(16)
      .optional()
      .describe(
        'Aspire OTLP API key, used only when secure_otlp is true. If omitted, the worker generates one for this process. iii-observability cannot send it — set OTEL_EXPORTER_OTLP_HEADERS on the engine instead.',
      ),
    start_timeout_ms: z
      .number()
      .int()
      .positive()
      .default(120_000)
      .describe('How long to wait for the dashboard UI to answer over HTTP'),
    stop_grace_ms: z
      .number()
      .int()
      .positive()
      .default(5_000)
      .describe('Grace period between SIGTERM and SIGKILL when stopping the dashboard process'),
    auto_start: z.boolean().default(true).describe('Start the Aspire Dashboard process when the worker boots'),
  })
  .refine((cfg) => new Set([cfg.dashboard_port, cfg.proxy_port, cfg.otlp_port, cfg.otlp_http_port]).size === 4, {
    message: 'dashboard_port, proxy_port, otlp_port, and otlp_http_port must be different',
    path: ['proxy_port'],
  })

export const RuntimeConfigSchema = RuntimeFields
export type RuntimeConfig = z.infer<typeof RuntimeConfigSchema>

const ConfigSchema = RuntimeFields.extend({
  engine_url: z.string().default('ws://127.0.0.1:49134'),
})

export type Config = z.infer<typeof ConfigSchema>

const generatedOtlpApiKey = randomBytes(32).toString('base64url')

export function otlpApiKey(config: Pick<Config, 'otlp_api_key'>): string {
  return config.otlp_api_key ?? generatedOtlpApiKey
}

export function runtimeJsonSchema(): Record<string, unknown> {
  const out = z.toJSONSchema(RuntimeConfigSchema) as Record<string, unknown>
  delete out.$schema
  return out
}

export function toRuntime(cfg: Config): RuntimeConfig {
  const { engine_url: _drop, ...runtime } = cfg
  return runtime
}

export async function loadConfig(path: string): Promise<Config> {
  let raw: unknown = {}
  try {
    raw = parse(await readFile(path, 'utf8')) ?? {}
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code !== 'ENOENT') throw err
  }
  return ConfigSchema.parse(raw)
}
