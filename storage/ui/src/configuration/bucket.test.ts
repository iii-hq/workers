// biome-ignore-all lint/suspicious/noTemplateCurlyInString: these literals intentionally exercise ${VAR} configuration templates.
import { describe, expect, it } from 'vitest'
import {
  applyPreset,
  bucketDisplay,
  bucketNameError,
  bucketProblems,
  checkServiceAccountJson,
  detectPreset,
  type JsonObject,
  MAX_SECRET_NAME,
  newBucketEntry,
  nextBucketName,
  parseTestOutcome,
  presetById,
  presetEndpoint,
  providerChoiceOf,
  providerLabel,
  REDACTED,
  secretNameFor,
  secretNamesInUse,
  storeTarget,
  supportsSecretReferences,
  switchProvider,
  testBlocker,
  testBucketPayload,
  testOutcomeHint,
  testOutcomeText,
  underlyingBucket,
  uniqueSecretName,
  usesSecrets,
  valueKind,
  withDisplay,
  withField,
  withRegion,
} from './bucket'

const SA_KEY = JSON.stringify({
  type: 'service_account',
  project_id: 'docs-prod',
  client_email: 'storage@docs-prod.iam.gserviceaccount.com',
  private_key: '-----BEGIN PRIVATE KEY-----\nMIIE\n-----END PRIVATE KEY-----\n',
})

describe('secret names', () => {
  it('derives STORAGE_<BUCKET>_<FIELD>, uppercased, with every other character as _', () => {
    expect(secretNameFor('photos', 'secret_access_key')).toBe('STORAGE_PHOTOS_SECRET_ACCESS_KEY')
    expect(secretNameFor('my-photos', 'access_key_id')).toBe('STORAGE_MY_PHOTOS_ACCESS_KEY_ID')
    expect(secretNameFor('docs_2', 'credentials_json')).toBe('STORAGE_DOCS_2_CREDENTIALS_JSON')
    expect(secretNameFor('r2', 'account_id')).toBe('STORAGE_R2_ACCOUNT_ID')
    expect(secretNameFor('cdn', 'session_token')).toBe('STORAGE_CDN_SESSION_TOKEN')
    expect(secretNameFor('cdn', 'notifications.api_token')).toBe('STORAGE_CDN_NOTIFICATIONS_API_TOKEN')
  })

  it('names a bucket without a name NEW', () => {
    expect(secretNameFor('', 'secret_access_key')).toBe('STORAGE_NEW_SECRET_ACCESS_KEY')
    expect(secretNameFor('   ', 'access_key_id')).toBe('STORAGE_NEW_ACCESS_KEY_ID')
  })

  it('caps the name at 128 characters and keeps the field', () => {
    const name = secretNameFor('a'.repeat(200), 'secret_access_key')
    expect(name.length).toBe(MAX_SECRET_NAME)
    expect(name.startsWith('STORAGE_AAAA')).toBe(true)
    expect(name.endsWith('_SECRET_ACCESS_KEY')).toBe(true)
  })

  it('adds _2, _3 on a clash, within the cap', () => {
    expect(uniqueSecretName('A', new Set())).toBe('A')
    expect(uniqueSecretName('A', new Set(['A']))).toBe('A_2')
    expect(uniqueSecretName('A', new Set(['A', 'A_2']))).toBe('A_3')
    const long = 'X'.repeat(MAX_SECRET_NAME)
    const next = uniqueSecretName(long, new Set([long]))
    expect(next.length).toBe(MAX_SECRET_NAME)
    expect(next.endsWith('_2')).toBe(true)
  })

  it('collects the names other fields reference, except the one being edited', () => {
    const buckets: JsonObject = {
      'a-b': { provider: 's3', region: 'x', secret_access_key: 'secret://STORAGE_A_B_SECRET_ACCESS_KEY' },
      a_b: {
        provider: 'r2',
        account_id: 'env://CF_ACCOUNT',
        access_key_id: 'secret://SHARED_ID',
        secret_access_key: 'plain',
        notifications: { queue_id: 'q', api_token: 'secret://CF_QUEUE' },
      },
      docs: { provider: 'gcs', credentials_json: 'secret://STORAGE_DOCS_CREDENTIALS_JSON' },
      broken: 'not an object',
    }
    expect([...secretNamesInUse(buckets)].sort()).toEqual([
      'CF_QUEUE',
      'SHARED_ID',
      'STORAGE_A_B_SECRET_ACCESS_KEY',
      'STORAGE_DOCS_CREDENTIALS_JSON',
    ])
    expect(secretNamesInUse(buckets, { bucket: 'a-b', field: 'secret_access_key' }).has('STORAGE_A_B_SECRET_ACCESS_KEY')).toBe(
      false,
    )
  })

  it('picks _2 when another bucket derives the same name', () => {
    const buckets: JsonObject = {
      'a-b': { provider: 's3', region: 'x', secret_access_key: 'secret://STORAGE_A_B_SECRET_ACCESS_KEY' },
      a_b: { provider: 's3', region: 'x' },
    }
    expect(storeTarget(buckets, 'a_b', 'secret_access_key', undefined)).toBe('STORAGE_A_B_SECRET_ACCESS_KEY_2')
  })

  it('rotates a referenced secret in place unless another field uses it too', () => {
    const own: JsonObject = { photos: { provider: 's3', region: 'x', secret_access_key: 'secret://MY_KEY' } }
    expect(storeTarget(own, 'photos', 'secret_access_key', 'secret://MY_KEY')).toBe('MY_KEY')

    const shared: JsonObject = {
      photos: { provider: 's3', region: 'x', secret_access_key: 'secret://MINIO_SECRET' },
      videos: { provider: 's3', region: 'x', secret_access_key: 'secret://MINIO_SECRET' },
    }
    expect(storeTarget(shared, 'photos', 'secret_access_key', 'secret://MINIO_SECRET')).toBe(
      'STORAGE_PHOTOS_SECRET_ACCESS_KEY',
    )
    // An env:// or literal value always gets the derived name.
    expect(storeTarget(own, 'photos', 'access_key_id', 'env://AWS_ACCESS_KEY_ID')).toBe('STORAGE_PHOTOS_ACCESS_KEY_ID')
  })
})

describe('value kinds', () => {
  it('tells references, templates, the sentinel and literals apart', () => {
    expect(valueKind(undefined)).toBe('empty')
    expect(valueKind(null)).toBe('empty')
    expect(valueKind('  ')).toBe('empty')
    expect(valueKind('secret://STORAGE_X_SECRET_ACCESS_KEY')).toBe('secret')
    expect(valueKind(' Secret://X ')).toBe('secret')
    expect(valueKind('env://AWS_SECRET_ACCESS_KEY')).toBe('env')
    expect(valueKind('${AWS_SECRET_ACCESS_KEY}')).toBe('template')
    expect(valueKind('prefix-${X}')).toBe('template')
    expect(valueKind(REDACTED)).toBe('redacted')
    expect(valueKind('wJalrXUtnFEMI/K7MDENG')).toBe('literal')
    expect(valueKind('secret://9bad')).toBe('literal')
    expect(valueKind(42)).toBe('opaque')
    expect(valueKind({ a: 1 })).toBe('opaque')
  })

  it('says whether an entry resolves anything through the secrets worker', () => {
    expect(usesSecrets({ provider: 's3', secret_access_key: 'secret://K' })).toBe(true)
    expect(usesSecrets({ provider: 'r2', account_id: 'env://CF' })).toBe(true)
    expect(usesSecrets({ provider: 'r2', notifications: { api_token: 'secret://T' } })).toBe(true)
    expect(usesSecrets({ provider: 's3', secret_access_key: '${KEY}' })).toBe(false)
    expect(usesSecrets({ provider: 'local' })).toBe(false)
  })
})

describe('providers', () => {
  it('reads S3 with an endpoint as S3-compatible', () => {
    expect(providerChoiceOf({ provider: 's3', region: 'us-east-1' })).toBe('s3')
    expect(providerChoiceOf({ provider: 's3', endpoint_url: 'http://minio:9000' })).toBe('s3-compatible')
    expect(providerChoiceOf({ provider: 's3', endpoint_url: '  ' })).toBe('s3')
    expect(providerChoiceOf({ provider: 'r2', endpoint_url: 'http://minio:9000' })).toBe('r2')
    expect(providerChoiceOf({ provider: 'nope' })).toBe('local')
    expect(providerLabel('s3-compatible')).toBe('S3-compatible')
    expect(providerLabel('gcs')).toBe('GCS')
  })

  it('starts each provider from its defaults', () => {
    expect(newBucketEntry('local')).toEqual({ provider: 'local' })
    expect(newBucketEntry('s3')).toEqual({ provider: 's3', region: 'us-east-1' })
    expect(newBucketEntry('s3-compatible')).toEqual({
      provider: 's3',
      region: 'us-east-1',
      endpoint_url: 'http://localhost:9000',
      force_path_style: true,
    })
    expect(newBucketEntry('r2')).toEqual({ provider: 'r2', account_id: '', access_key_id: '', secret_access_key: '' })
    expect(newBucketEntry('gcs')).toEqual({ provider: 'gcs' })
  })

  it('carries the underlying bucket, display and access keys across a switch', () => {
    const s3: JsonObject = {
      provider: 's3',
      bucket: 'photos-prod',
      region: 'eu-west-1',
      display: { icon: 'cloud', color: '#2563EB' },
      access_key_id: 'secret://ID',
      secret_access_key: 'secret://KEY',
      session_token: 'secret://TOKEN',
      notifications: { sqs_queue_url: 'https://sqs' },
    }
    expect(switchProvider(s3, 'r2')).toEqual({
      provider: 'r2',
      account_id: '',
      bucket: 'photos-prod',
      display: { icon: 'cloud', color: '#2563EB' },
      access_key_id: 'secret://ID',
      secret_access_key: 'secret://KEY',
    })
    expect(switchProvider(s3, 'gcs')).toEqual({
      provider: 'gcs',
      bucket: 'photos-prod',
      display: { icon: 'cloud', color: '#2563EB' },
    })
    expect(switchProvider(s3, 'local')).toEqual({
      provider: 'local',
      bucket: 'photos-prod',
      display: { icon: 'cloud', color: '#2563EB' },
    })
    expect(switchProvider(s3, 's3')).toBe(s3)
  })

  it('does not carry empty access keys over', () => {
    const r2 = newBucketEntry('r2')
    expect(switchProvider(r2, 's3')).toEqual({ provider: 's3', region: 'us-east-1' })
  })

  it('moves between S3 and S3-compatible by the endpoint alone', () => {
    const s3: JsonObject = { provider: 's3', region: 'eu-west-1', secret_access_key: 'secret://K' }
    expect(switchProvider(s3, 's3-compatible')).toEqual({
      provider: 's3',
      region: 'eu-west-1',
      secret_access_key: 'secret://K',
      endpoint_url: 'http://localhost:9000',
      force_path_style: true,
    })
    const b2: JsonObject = {
      provider: 's3',
      region: 'us-west-004',
      endpoint_url: 'https://s3.us-west-004.backblazeb2.com',
      secret_access_key: 'secret://K',
    }
    // A hosted service's region means nothing to AWS.
    expect(switchProvider(b2, 's3')).toEqual({ provider: 's3', region: 'us-east-1', secret_access_key: 'secret://K' })
    const ceph: JsonObject = { provider: 's3', region: 'default', endpoint_url: 'http://ceph:7480', force_path_style: true }
    expect(switchProvider(ceph, 's3')).toEqual({ provider: 's3', region: 'default' })
  })
})

describe('S3-compatible presets', () => {
  it('derives endpoints from the region', () => {
    expect(presetEndpoint(presetById('b2'), 'eu-central-003')).toBe('https://s3.eu-central-003.backblazeb2.com')
    expect(presetEndpoint(presetById('wasabi'), '')).toBe('https://s3.us-east-1.wasabisys.com')
    expect(presetEndpoint(presetById('spaces'), 'ams3')).toBe('https://ams3.digitaloceanspaces.com')
    expect(presetEndpoint(presetById('minio'), 'anything')).toBe('http://localhost:9000')
    expect(presetEndpoint(presetById('other'), 'x')).toBe('')
  })

  it('recognises a stored endpoint', () => {
    expect(detectPreset('https://s3.us-west-004.backblazeb2.com')).toBe('b2')
    expect(detectPreset('https://s3.eu-central-1.wasabisys.com')).toBe('wasabi')
    expect(detectPreset('https://s3.wasabisys.com')).toBe('wasabi')
    expect(detectPreset('https://nyc3.digitaloceanspaces.com')).toBe('spaces')
    expect(detectPreset('http://100.70.145.81:9000')).toBe('minio')
    expect(detectPreset('http://minio.lan')).toBe('minio')
    expect(detectPreset('http://ceph:7480')).toBe('other')
    expect(detectPreset('')).toBe('other')
    expect(detectPreset('not a url')).toBe('other')
    expect(detectPreset('${S3_ENDPOINT}')).toBe('other')
  })

  it('applies a preset: endpoint, region and addressing', () => {
    const base: JsonObject = { provider: 's3', region: 'us-east-1', secret_access_key: 'secret://K', force_path_style: true }
    expect(applyPreset(base, 'spaces')).toEqual({
      provider: 's3',
      region: 'nyc3',
      endpoint_url: 'https://nyc3.digitaloceanspaces.com',
      secret_access_key: 'secret://K',
    })
    expect(applyPreset({ provider: 's3', region: 'eu-1' }, 'minio')).toEqual({
      provider: 's3',
      region: 'eu-1',
      endpoint_url: 'http://localhost:9000',
      force_path_style: true,
    })
  })

  it('"Other" keeps a hand-typed endpoint and drops a preset one', () => {
    expect(applyPreset({ provider: 's3', region: 'r', endpoint_url: 'http://ceph:7480' }, 'other')).toEqual({
      provider: 's3',
      region: 'r',
      endpoint_url: 'http://ceph:7480',
      force_path_style: true,
    })
    expect(
      applyPreset({ provider: 's3', region: 'nyc3', endpoint_url: 'https://nyc3.digitaloceanspaces.com' }, 'other'),
    ).toEqual({ provider: 's3', region: 'nyc3', force_path_style: true })
  })

  it('moves a derived endpoint with the region, and leaves a typed one alone', () => {
    const wasabi = applyPreset({ provider: 's3' }, 'wasabi')
    expect(withRegion(wasabi, 'eu-central-1').endpoint_url).toBe('https://s3.eu-central-1.wasabisys.com')
    const typed: JsonObject = { provider: 's3', region: 'us-east-1', endpoint_url: 'https://s3.wasabisys.com' }
    expect(withRegion(typed, 'eu-central-1')).toEqual({ ...typed, region: 'eu-central-1' })
    const minio = applyPreset({ provider: 's3' }, 'minio')
    expect(withRegion(minio, 'eu-west-1').endpoint_url).toBe('http://localhost:9000')
  })
})

describe('fields and display', () => {
  it('sets and removes dotted fields without touching the rest', () => {
    const bucket: JsonObject = { provider: 'r2', notifications: { queue_id: 'q' } }
    expect(withField(bucket, 'notifications.api_token', 'secret://T')).toEqual({
      provider: 'r2',
      notifications: { queue_id: 'q', api_token: 'secret://T' },
    })
    expect(withField(bucket, 'account_id', 'abc')).toEqual({ ...bucket, account_id: 'abc' })
    expect(withField({ ...bucket, account_id: 'abc' }, 'account_id', undefined)).toEqual(bucket)
    expect(bucket).toEqual({ provider: 'r2', notifications: { queue_id: 'q' } })
  })

  it('reads display, dropping a malformed colour, and removes an empty block', () => {
    expect(bucketDisplay({ display: { icon: 'cloud', color: '#2563EB' } })).toEqual({ icon: 'cloud', color: '#2563EB' })
    expect(bucketDisplay({ display: { color: 'blue' } })).toEqual({ icon: undefined, color: undefined })
    expect(bucketDisplay({ provider: 'local' })).toEqual({})
    expect(bucketDisplay('x')).toEqual({})
    const withIcon = withDisplay({ provider: 'local' }, 'icon', 'cloud')
    expect(withIcon).toEqual({ provider: 'local', display: { icon: 'cloud' } })
    expect(withDisplay(withIcon, 'icon', undefined)).toEqual({ provider: 'local' })
  })

  it('validates names like the worker', () => {
    expect(bucketNameError('photos')).toBeUndefined()
    expect(bucketNameError('0-a_b')).toBeUndefined()
    expect(bucketNameError('Photos')).toBeDefined()
    expect(bucketNameError('-photos')).toBeDefined()
    expect(bucketNameError('a'.repeat(64))).toBeDefined()
    expect(nextBucketName({})).toBe('bucket')
    expect(nextBucketName({ bucket: {}, 'bucket-2': {} })).toBe('bucket-3')
    expect(underlyingBucket('photos', {})).toBe('photos')
    expect(underlyingBucket('photos', { bucket: 'photos-prod' })).toBe('photos-prod')
  })

  it('checks a pasted service-account key without keeping it', () => {
    expect(checkServiceAccountJson(SA_KEY)).toEqual({
      ok: true,
      email: 'storage@docs-prod.iam.gserviceaccount.com',
      project: 'docs-prod',
    })
    expect(checkServiceAccountJson(`﻿${SA_KEY}\n`).ok).toBe(true)
    expect(checkServiceAccountJson('')).toEqual({ ok: false, reason: 'Paste the key’s JSON.' })
    expect(checkServiceAccountJson('{nope').ok).toBe(false)
    expect(checkServiceAccountJson('[]').ok).toBe(false)
    expect(checkServiceAccountJson(JSON.stringify({ type: 'authorized_user' })).ok).toBe(false)
    expect(checkServiceAccountJson(JSON.stringify({ type: 'service_account', client_email: 'a@b' })).ok).toBe(false)
  })
})

describe('problems', () => {
  it('lists what the worker would refuse', () => {
    expect(bucketProblems({ provider: 'local' })).toEqual([])
    expect(bucketProblems({ provider: 's3', region: '' })).toEqual([{ field: 'region', message: 'Required.' }])
    expect(bucketProblems(newBucketEntry('r2')).map((p) => p.field)).toEqual([
      'account_id',
      'access_key_id',
      'secret_access_key',
    ])
    expect(bucketProblems({ provider: 's3', region: 'x', access_key_id: 'AKIA' })).toEqual([
      { field: 'secret_access_key', message: 'Static credentials need both the access key ID and the secret access key.' },
    ])
    expect(bucketProblems({ provider: 's3', region: 'x', session_token: 'secret://T' })[0].field).toBe('session_token')
    expect(bucketProblems({ provider: 's3', region: 'x', endpoint_url: 'minio:9000' })[0].field).toBe('endpoint_url')
    expect(bucketProblems({ provider: 's3', region: 'x', endpoint_url: '${S3_URL}' })).toEqual([])
    expect(bucketProblems({ provider: 'gcs', credentials_file: '/k.json', credentials_json: 'secret://K' })[0].field).toBe(
      'credentials_json',
    )
  })

  it('says in one line why the test cannot run', () => {
    expect(testBlocker([])).toBeNull()
    expect(testBlocker(bucketProblems({ provider: 's3', region: '' }))).toBe('Fill in the region to test')
    expect(testBlocker(bucketProblems(newBucketEntry('r2')))).toBe(
      'Fill in the account ID, the access key ID and the secret access key to test',
    )
    expect(testBlocker([{ field: 'endpoint_url', message: 'Use an http:// or https:// URL.' }])).toBe(
      'Use an http:// or https:// URL.',
    )
  })
})

describe('testBucket', () => {
  it('moves literal credentials into the envelope and keeps references in place', () => {
    const bucket: JsonObject = {
      provider: 's3',
      region: 'us-east-1',
      endpoint_url: 'http://minio:9000',
      access_key_id: 'minioadmin',
      secret_access_key: 'minio-secret',
      session_token: 'secret://STORAGE_PHOTOS_SESSION_TOKEN',
      display: { icon: 'cloud' },
    }
    const payload = testBucketPayload('photos', bucket)
    expect(payload).toEqual({
      bucket: {
        provider: 's3',
        region: 'us-east-1',
        endpoint_url: 'http://minio:9000',
        session_token: 'secret://STORAGE_PHOTOS_SESSION_TOKEN',
        display: { icon: 'cloud' },
      },
      credentials: { access_key_id: 'minioadmin', secret_access_key: 'minio-secret' },
      timeout_ms: 10_000,
    })
    // The draft itself is untouched.
    expect(bucket.secret_access_key).toBe('minio-secret')
  })

  it('sends a pasted key, a queue token and leaves templates and env references alone', () => {
    const gcs = testBucketPayload('docs', { provider: 'gcs', credentials_json: SA_KEY }, 5000)
    expect(gcs).toEqual({ bucket: { provider: 'gcs' }, credentials: { credentials_json: SA_KEY }, timeout_ms: 5000 })

    const r2 = testBucketPayload('cdn', {
      provider: 'r2',
      account_id: 'abc123',
      access_key_id: '${R2_ID}',
      secret_access_key: 'env://R2_SECRET',
      notifications: { queue_id: 'q', api_token: 'tok' },
    })
    expect(r2).toEqual({
      bucket: {
        provider: 'r2',
        account_id: 'abc123',
        access_key_id: '${R2_ID}',
        secret_access_key: 'env://R2_SECRET',
        notifications: { queue_id: 'q' },
      },
      credentials: { notifications_api_token: 'tok' },
      timeout_ms: 10_000,
    })
  })

  it('names the stored entry only when the draft holds the sentinel', () => {
    const redacted = testBucketPayload('photos', { provider: 's3', region: 'x', secret_access_key: REDACTED })
    expect(redacted).toEqual({
      bucket: { provider: 's3', region: 'x', secret_access_key: REDACTED },
      name: 'photos',
      timeout_ms: 10_000,
    })
    expect(testBucketPayload('photos', { provider: 'local' })).toEqual({ bucket: { provider: 'local' }, timeout_ms: 10_000 })
  })

  it('reads the staged result', () => {
    expect(parseTestOutcome(null)).toBeNull()
    expect(parseTestOutcome({ provider: 's3' })).toBeNull()
    const ok = parseTestOutcome({ ok: true, provider: 's3', latency_ms: 42, direct_downloads: true })
    expect(ok && testOutcomeText(ok)).toBe('Connected · 42 ms')
    expect(ok && testOutcomeHint(ok, 'photos')).toBeNull()

    const auth = parseTestOutcome({ ok: false, stage: 'auth', message: 'SignatureDoesNotMatch' })
    expect(auth && testOutcomeText(auth)).toBe('Authentication failed · SignatureDoesNotMatch')

    const missing = parseTestOutcome({ ok: false, stage: 'bucket', message: 'NoSuchBucket', bucket_missing: true })
    expect(missing && testOutcomeText(missing)).toBe('Bucket failed · NoSuchBucket')
    expect(missing && testOutcomeHint(missing, 'photos-prod')).toContain('no bucket “photos-prod”')

    const relay = parseTestOutcome({ ok: true, latency_ms: 3, direct_downloads: false })
    expect(relay && testOutcomeHint(relay, 'docs')).toContain('cannot sign download URLs')

    const unknownStage = parseTestOutcome({ ok: false, stage: 'quantum' })
    expect(unknownStage && testOutcomeText(unknownStage)).toBe('quantum failed · The bucket could not be reached')
  })
})

describe('worker generation', () => {
  it('detects references support from the registered schema', () => {
    expect(supportsSecretReferences(null)).toBeNull()
    const legacy = {
      properties: {
        buckets: {
          additionalProperties: {
            oneOf: [{ properties: { provider: {}, bucket: {}, credentials_file: {}, endpoint_url: {} } }],
          },
        },
      },
    }
    expect(supportsSecretReferences(legacy)).toBe(false)
    const current = {
      properties: {
        buckets: {
          additionalProperties: {
            oneOf: [{ properties: { provider: {}, credentials_file: {}, credentials_json: {} } }],
          },
        },
      },
    }
    expect(supportsSecretReferences(current)).toBe(true)
    expect(supportsSecretReferences({ $defs: { Display: {}, Local: { properties: { bucket: {}, display: {} } } } })).toBe(
      true,
    )
  })
})
