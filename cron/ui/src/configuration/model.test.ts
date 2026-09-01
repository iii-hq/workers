import { describe, expect, it } from 'vitest'
import { readCronConfiguration, withAdapterConfigValue, withAdapterName, withAdapterValue, withRedisUrl } from './model'

const CRON_MODE_TEMPLATE = `\${CRON_MODE}`
const REDIS_URL_TEMPLATE = `\${REDIS_URL}`
const CRON_ADAPTER_CONFIG_TEMPLATE = `\${CRON_ADAPTER_CONFIG}`
const CRON_ADAPTER_TEMPLATE = `\${CRON_ADAPTER}`

describe('Cron configuration model', () => {
  it('preserves an opaque root until the form explicitly converts it', () => {
    const value = `\${CRON_CONFIG}`

    expect(readCronConfiguration(value)).toMatchObject({
      hasOpaqueRoot: true,
      value: {},
    })
    expect(withAdapterName(value, 'redis')).toBe(value)
    expect(withAdapterValue(value, { name: 'local' })).toBe(value)
    expect(withAdapterConfigValue(value, undefined)).toBe(value)
    expect(withRedisUrl(value, 'redis://cache:6379')).toBe(value)
  })

  it('reads the runtime default without materializing it', () => {
    const value = { future_root: CRON_MODE_TEMPLATE }
    const model = readCronConfiguration(value)

    expect(model.adapterName).toBe('local')
    expect(model.value).toEqual(value)
    expect(withAdapterName(value, 'local')).toEqual(value)
  })

  it('keeps the implicit Redis URL absent until the user overrides it', () => {
    const value = { adapter: { name: 'redis' } }

    expect(readCronConfiguration(value).redisUrl).toBe('')
    expect(withRedisUrl(value, '')).toEqual(value)
  })

  it('preserves future root, adapter and config fields when changing adapters', () => {
    const value = {
      future_root: true,
      adapter: {
        name: 'local',
        future_adapter: 3,
        config: {
          redis_url: REDIS_URL_TEMPLATE,
          future_config: { enabled: true },
        },
      },
    }

    expect(withAdapterName(value, 'redis')).toEqual({
      future_root: true,
      adapter: {
        name: 'redis',
        future_adapter: 3,
        config: {
          redis_url: REDIS_URL_TEMPLATE,
          future_config: { enabled: true },
        },
      },
    })
  })

  it('updates or clears only redis_url', () => {
    const value = {
      adapter: {
        name: 'redis',
        config: {
          redis_url: REDIS_URL_TEMPLATE,
          namespace: 'scheduled-jobs',
        },
      },
    }

    expect(withRedisUrl(value, 'redis://cache:6379')).toEqual({
      adapter: {
        name: 'redis',
        config: {
          redis_url: 'redis://cache:6379',
          namespace: 'scheduled-jobs',
        },
      },
    })
    expect(withRedisUrl(value, '')).toEqual({
      adapter: {
        name: 'redis',
        config: { namespace: 'scheduled-jobs' },
      },
    })
  })

  it('removes an otherwise empty config object when the URL is cleared', () => {
    expect(
      withRedisUrl(
        {
          adapter: {
            name: 'redis',
            config: { redis_url: 'redis://cache:6379' },
          },
        },
        '',
      ),
    ).toEqual({
      adapter: { name: 'redis' },
    })
  })

  it('keeps opaque future payloads intact instead of coercing them', () => {
    const opaqueConfig = {
      adapter: { name: 'redis', config: CRON_ADAPTER_CONFIG_TEMPLATE },
    }
    const opaqueAdapter = { adapter: CRON_ADAPTER_TEMPLATE }

    expect(readCronConfiguration(opaqueConfig).hasOpaqueAdapterConfig).toBe(true)
    expect(withRedisUrl(opaqueConfig, 'redis://cache:6379')).toEqual(opaqueConfig)
    expect(readCronConfiguration(opaqueAdapter).hasOpaqueAdapter).toBe(true)
    expect(withAdapterName(opaqueAdapter, 'redis')).toEqual(opaqueAdapter)
    expect(withAdapterValue(opaqueAdapter, { name: 'local' })).toEqual({
      adapter: { name: 'local' },
    })
    expect(withAdapterConfigValue(opaqueConfig, undefined)).toEqual({
      adapter: { name: 'redis' },
    })

    const nonStringAdapter = { adapter: ['future-adapter-shape'] }
    const nonStringConfig = {
      adapter: { name: 'redis', config: ['future-config-shape'] },
    }
    expect(readCronConfiguration(nonStringAdapter).hasOpaqueAdapter).toBe(true)
    expect(readCronConfiguration(nonStringConfig).hasOpaqueAdapterConfig).toBe(true)
    expect(withAdapterName(nonStringAdapter, 'local')).toEqual(nonStringAdapter)
    expect(withRedisUrl(nonStringConfig, 'redis://cache:6379')).toEqual(nonStringConfig)
  })

  it('retains an unknown adapter so it remains visible and selectable', () => {
    const value = { adapter: { name: 'future-lock', config: { mode: 'fast' } } }

    expect(readCronConfiguration(value)).toMatchObject({
      adapterName: 'future-lock',
      adapterConfig: { mode: 'fast' },
    })
    expect(withAdapterName(value, 'local')).toEqual({
      adapter: { name: 'local', config: { mode: 'fast' } },
    })
  })
})
