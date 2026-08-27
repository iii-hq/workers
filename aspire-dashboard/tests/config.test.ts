import { describe, expect, it } from 'vitest'
import { RuntimeConfigSchema } from '../src/config.js'

describe('Aspire Dashboard config', () => {
  it('defaults to distinct dashboard, proxy, OTLP/gRPC, and OTLP/HTTP ports', () => {
    const config = RuntimeConfigSchema.parse({})
    expect(config.dashboard_port).toBe(18888)
    expect(config.proxy_port).toBe(18887)
    expect(config.otlp_port).toBe(4317)
    expect(config.otlp_http_port).toBe(4318)
    expect(config.aspire_command).toEqual(['npx', '-y', '@microsoft/aspire-cli', 'dashboard', 'run'])
  })

  it('leaves OTLP ingestion unsecured by default so iii-observability can export to it', () => {
    // iii-observability has no OTLP header/auth support, and its config schema
    // is additionalProperties:false, so no API key can reach the exporter.
    expect(RuntimeConfigSchema.parse({}).secure_otlp).toBe(false)
  })

  it('rejects reused ports', () => {
    expect(() =>
      RuntimeConfigSchema.parse({
        dashboard_port: 18888,
        proxy_port: 18887,
        otlp_port: 4317,
        otlp_http_port: 4317,
      }),
    ).toThrow(/must be different/)
  })
})
