// biome-ignore-all lint/suspicious/noTemplateCurlyInString: these literals intentionally exercise ${VAR} configuration templates.
import type { Host } from '@iii-dev/console-ui'
import { describe, expect, it } from 'vitest'
import { runBucketTest, testRequest } from './test-bucket'

function fakeHost(respond: (fn: string, payload: Record<string, unknown>) => unknown) {
  const calls: Array<{ fn: string; payload: Record<string, unknown> }> = []
  const host = {
    iii: {
      trigger: async (fn: string, payload: Record<string, unknown>) => {
        calls.push({ fn, payload })
        return respond(fn, payload)
      },
    },
  } as unknown as Host
  return { host, calls }
}

const LEGACY = {
  provider: 's3',
  bucket: 'legacy',
  region: 'us-east-1',
  endpoint_url: 'http://minio:9000',
  force_path_style: true,
  access_key_id: '${IIIST_LEGACY_ACCESS_KEY_ID}',
  secret_access_key: '${IIIST_LEGACY_SECRET_ACCESS_KEY}',
}

describe('the test request', () => {
  it('tests an unchanged stored bucket by name, so its templates are expanded', async () => {
    const { host } = fakeHost(() => ({ name: 'legacy-env', bucket: { ...LEGACY } }))
    expect(await testRequest(host, 'legacy-env', { ...LEGACY })).toEqual({ name: 'legacy-env', timeout_ms: 10_000 })
  })

  it('tests an edited draft as a bucket', async () => {
    const { host } = fakeHost(() => ({ name: 'legacy-env', bucket: { ...LEGACY } }))
    const draft = { ...LEGACY, region: 'eu-west-1' }
    const request = await testRequest(host, 'legacy-env', draft)
    expect(request.bucket).toEqual(draft)
    expect(request.name).toBeUndefined()
  })

  it('tests the draft of a new bucket, or on a worker without getBucket', async () => {
    const unknown = fakeHost((fn) => {
      if (fn === 'storage::getBucket')
        throw { code: 'invocation_failed', message: 'handler error: {"code":"UNKNOWN_BUCKET","bucket":"new"}' }
      return null
    })
    expect((await testRequest(unknown.host, 'new', { provider: 'local' })).bucket).toEqual({
      provider: 'local',
      bucket: 'new',
    })
    const old = fakeHost(() => {
      throw { code: 'function_not_found', message: 'Function storage::getBucket not found in namespace default.' }
    })
    expect((await testRequest(old.host, 'new', { provider: 'local' })).bucket).toEqual({
      provider: 'local',
      bucket: 'new',
    })
  })
})

describe('running the test', () => {
  it('words a passing test and its relay note', async () => {
    const { host, calls } = fakeHost((fn) =>
      fn === 'storage::getBucket' ? null : { ok: true, provider: 'gcs', latency_ms: 9, direct_downloads: false },
    )
    expect(await runBucketTest(host, 'docs', { provider: 'gcs', credentials_json: 'secret://K' })).toEqual({
      status: 'done',
      ok: true,
      text: 'Connected · 9 ms',
      hint: 'Files can be read through the worker only: this bucket cannot sign download URLs.',
    })
    expect(calls.map((c) => c.fn)).toEqual(['storage::getBucket', 'storage::testBucket'])
  })

  it('points an edited template draft at saving first', async () => {
    const { host } = fakeHost((fn) =>
      fn === 'storage::getBucket'
        ? { bucket: { ...LEGACY } }
        : { ok: false, provider: 's3', latency_ms: 4, stage: 'auth', message: 'HTTP 403 InvalidAccessKeyId' },
    )
    const result = await runBucketTest(host, 'legacy-env', { ...LEGACY, region: 'eu-west-1' })
    expect(result).toMatchObject({
      status: 'done',
      ok: false,
      text: 'Authentication failed · HTTP 403 InvalidAccessKeyId',
    })
    expect(result.status === 'done' && result.hint).toContain('save it, then test again')
  })

  it('explains a worker without testBucket', async () => {
    const { host } = fakeHost((fn) => {
      if (fn === 'storage::getBucket') return null
      throw { code: 'function_not_found', message: 'Function storage::testBucket not found in namespace default.' }
    })
    expect(await runBucketTest(host, 'a', { provider: 'local' })).toEqual({ status: 'unsupported' })
  })

  it('reports any other failure as text', async () => {
    const { host } = fakeHost((fn) => {
      if (fn === 'storage::getBucket') return null
      throw { code: 'TIMEOUT', message: 'timed out after 40000ms' }
    })
    expect(await runBucketTest(host, 'a', { provider: 'local' })).toEqual({
      status: 'done',
      ok: false,
      text: 'TIMEOUT: timed out after 40000ms',
    })
  })
})
