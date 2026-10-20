/**
 * One bucket entry (`buckets.<name>`) as the form edits it. Mirrors
 * `BucketConfig` in storage/src/config.rs.
 *
 * The provider is one of `local`, `s3`, `r2` and `gcs`; "S3-compatible"
 * (MinIO, Backblaze B2, Wasabi, DigitalOcean Spaces, …) is `s3` with an
 * `endpoint_url`, which the form offers as its own choice with presets.
 * Credentials are references (`secret://NAME`, `env://NAME`) resolved by
 * the worker through the secrets worker; `${VAR}` templates and plain
 * values from older configurations keep working and are shown as such.
 *
 * Everything here is pure: it reads and returns JSON objects, so the form
 * keeps keys it does not know about and templates it must not coerce.
 */

import type { JsonValue } from '@iii-dev/console-ui'
import { envRefName, secretRefName } from '../lib/secrets'

export type JsonObject = { [key: string]: JsonValue }

export type ProviderName = 'local' | 's3' | 'gcs' | 'r2'

export const PROVIDERS: readonly ProviderName[] = ['local', 's3', 'gcs', 'r2']

/** What the form's "Provider" choice says; `s3-compatible` is `s3` + `endpoint_url`. */
export type ProviderChoice = 'local' | 's3' | 's3-compatible' | 'r2' | 'gcs'

export const PROVIDER_CHOICES: ReadonlyArray<{ value: ProviderChoice; label: string; description: string }> = [
  {
    value: 'local',
    label: 'Local filesystem',
    description: 'Files on the engine host, under the local data directory.',
  },
  { value: 's3', label: 'Amazon S3', description: 'An AWS bucket in one region.' },
  {
    value: 's3-compatible',
    label: 'S3-compatible',
    description: 'MinIO, Backblaze B2, Wasabi, DigitalOcean Spaces, Ceph, SeaweedFS…',
  },
  { value: 'r2', label: 'Cloudflare R2', description: 'An R2 bucket of a Cloudflare account.' },
  {
    value: 'gcs',
    label: 'Google Cloud Storage',
    description: 'A GCS bucket, with a service account or the host’s credentials.',
  },
]

/** What `storage::getBucket` puts where a stored literal secret was. */
export const REDACTED = '__redacted__'

function isObject(v: JsonValue | undefined): v is JsonObject {
  return v !== null && typeof v === 'object' && !Array.isArray(v)
}

function str(v: JsonValue | undefined): string {
  return typeof v === 'string' ? v : ''
}

export function providerOf(bucket: JsonObject): ProviderName {
  const provider = str(bucket.provider)
  return (PROVIDERS as readonly string[]).includes(provider) ? (provider as ProviderName) : 'local'
}

export function providerChoiceOf(bucket: JsonObject): ProviderChoice {
  const provider = providerOf(bucket)
  if (provider === 's3' && str(bucket.endpoint_url).trim() !== '') return 's3-compatible'
  return provider
}

export function providerLabel(choice: ProviderChoice | string): string {
  switch (choice) {
    case 'local':
      return 'Local'
    case 's3':
      return 'S3'
    case 's3-compatible':
      return 'S3-compatible'
    case 'r2':
      return 'R2'
    case 'gcs':
      return 'GCS'
    default:
      return choice
  }
}

/* ---------------- S3-compatible presets ---------------- */

export type PresetId = 'minio' | 'b2' | 'wasabi' | 'spaces' | 'other'

export interface S3Preset {
  id: PresetId
  label: string
  /** `{region}` is replaced by the bucket's region. */
  endpoint: string
  region: string
  pathStyle: boolean
  regionHint?: string
}

export const S3_PRESETS: readonly S3Preset[] = [
  { id: 'minio', label: 'MinIO', endpoint: 'http://localhost:9000', region: 'us-east-1', pathStyle: true },
  {
    id: 'b2',
    label: 'Backblaze B2',
    endpoint: 'https://s3.{region}.backblazeb2.com',
    region: 'us-west-004',
    pathStyle: false,
    regionHint: 'The region in the bucket’s S3 endpoint, e.g. us-west-004 or eu-central-003.',
  },
  {
    id: 'wasabi',
    label: 'Wasabi',
    endpoint: 'https://s3.{region}.wasabisys.com',
    region: 'us-east-1',
    pathStyle: false,
    regionHint: 'The bucket’s Wasabi region, e.g. us-east-1, eu-central-1 or ap-northeast-1.',
  },
  {
    id: 'spaces',
    label: 'DigitalOcean Spaces',
    endpoint: 'https://{region}.digitaloceanspaces.com',
    region: 'nyc3',
    pathStyle: false,
    regionHint: 'The Space’s datacenter, e.g. nyc3, ams3, sfo3 or sgp1.',
  },
  { id: 'other', label: 'Other (Ceph, SeaweedFS, LocalStack…)', endpoint: '', region: 'us-east-1', pathStyle: true },
]

export function presetById(id: PresetId): S3Preset {
  return S3_PRESETS.find((p) => p.id === id) ?? S3_PRESETS[S3_PRESETS.length - 1]
}

export function presetEndpoint(preset: S3Preset, region: string): string {
  return preset.endpoint.replaceAll('{region}', region.trim() || preset.region)
}

const PRESET_HOSTS: ReadonlyArray<readonly [PresetId, RegExp]> = [
  ['b2', /(^|\.)backblazeb2\.com$/i],
  ['wasabi', /(^|\.)wasabisys\.com$/i],
  ['spaces', /(^|\.)digitaloceanspaces\.com$/i],
]

/** Which preset an endpoint belongs to — by host for the hosted services, by MinIO's default port otherwise. */
export function detectPreset(endpoint: string): PresetId {
  const trimmed = endpoint.trim()
  if (!trimmed) return 'other'
  let host = ''
  let port = ''
  try {
    const url = new URL(trimmed)
    host = url.hostname
    port = url.port
  } catch {
    return 'other'
  }
  for (const [id, re] of PRESET_HOSTS) if (re.test(host)) return id
  if (port === '9000' || /minio/i.test(host)) return 'minio'
  return 'other'
}

/**
 * The bucket with a preset applied: its endpoint, region and addressing. A
 * hosted service's region resets to the service's default (another
 * service's region means nothing there). "Other" keeps an endpoint typed by
 * hand and drops one that belonged to a preset.
 */
export function applyPreset(bucket: JsonObject, id: PresetId): JsonObject {
  const preset = presetById(id)
  const next: JsonObject = { ...bucket, provider: 's3' }
  const current = str(bucket.endpoint_url)
  const region = preset.endpoint.includes('{region}') ? preset.region : str(bucket.region).trim() || preset.region
  next.region = region
  if (preset.endpoint) next.endpoint_url = presetEndpoint(preset, region)
  else if (current.trim() && detectPreset(current) !== 'other') delete next.endpoint_url
  if (preset.pathStyle) next.force_path_style = true
  else delete next.force_path_style
  return next
}

/**
 * A region edit on an S3-compatible bucket: when the endpoint is the one its
 * preset derives from the old region, it follows the new region; an
 * endpoint typed by hand is left alone.
 */
export function withRegion(bucket: JsonObject, region: string): JsonObject {
  const next: JsonObject = { ...bucket, region }
  const endpoint = str(bucket.endpoint_url)
  const preset = presetById(detectPreset(endpoint))
  if (preset.endpoint.includes('{region}') && endpoint === presetEndpoint(preset, str(bucket.region))) {
    next.endpoint_url = presetEndpoint(preset, region)
  }
  return next
}

/* ---------------- provider switch ---------------- */

/** Keys each provider's entry knows (besides `provider`). */
const PROVIDER_KEYS: Readonly<Record<ProviderName, readonly string[]>> = {
  local: ['bucket', 'display'],
  s3: [
    'bucket',
    'display',
    'region',
    'access_key_id',
    'secret_access_key',
    'session_token',
    'endpoint_url',
    'force_path_style',
    'notifications',
  ],
  r2: ['bucket', 'display', 'account_id', 'access_key_id', 'secret_access_key', 'endpoint_url', 'notifications'],
  gcs: ['bucket', 'display', 'credentials_file', 'credentials_json', 'endpoint_url', 'notifications'],
}

/** Keys that mean the same thing under two providers, so a switch carries them over. */
const PORTABLE_KEYS = ['bucket', 'display', 'access_key_id', 'secret_access_key'] as const

/** A fresh entry for a provider choice. */
export function newBucketEntry(choice: ProviderChoice): JsonObject {
  switch (choice) {
    case 'local':
      return { provider: 'local' }
    case 's3':
      return { provider: 's3', region: 'us-east-1' }
    case 's3-compatible':
      return applyPreset({ provider: 's3' }, 'minio')
    case 'r2':
      return { provider: 'r2', account_id: '', access_key_id: '', secret_access_key: '' }
    case 'gcs':
      return { provider: 'gcs' }
  }
}

/**
 * Switch an entry's provider choice. The underlying bucket name, the
 * display block and access keys carry over where the new provider has
 * them; everything provider-specific starts from the new provider's
 * defaults. S3 ↔ S3-compatible only adds or drops the endpoint.
 */
export function switchProvider(bucket: JsonObject, choice: ProviderChoice): JsonObject {
  const current = providerChoiceOf(bucket)
  if (current === choice) return bucket
  if (current === 's3' && choice === 's3-compatible') return applyPreset(bucket, 'minio')
  if (current === 's3-compatible' && choice === 's3') {
    const next = { ...bucket }
    delete next.endpoint_url
    delete next.force_path_style
    // A hosted service's region (us-west-004, nyc3) means nothing to AWS.
    const hosted = presetById(detectPreset(str(bucket.endpoint_url))).endpoint.includes('{region}')
    if (str(next.region).trim() === '' || hosted) next.region = 'us-east-1'
    return next
  }
  const next = newBucketEntry(choice)
  const keys = PROVIDER_KEYS[next.provider as ProviderName]
  for (const key of PORTABLE_KEYS) {
    const value = bucket[key]
    if (!keys.includes(key) || value === undefined || value === null) continue
    if (typeof value === 'string' && value.trim() === '') continue
    next[key] = value
  }
  return next
}

/* ---------------- credentials ---------------- */

/**
 * Every value the form keeps out of configuration. `access_key_id` and
 * `account_id` are identifiers rather than secrets (the worker accepts them
 * inline), but they go to the secrets store by the same rules when the user
 * keeps them there.
 */
export type CredentialField =
  | 'access_key_id'
  | 'secret_access_key'
  | 'session_token'
  | 'account_id'
  | 'credentials_json'
  | 'notifications.api_token'

export const CREDENTIAL_FIELDS: readonly CredentialField[] = [
  'access_key_id',
  'secret_access_key',
  'session_token',
  'account_id',
  'credentials_json',
  'notifications.api_token',
]

/** Sensitive: never written into `bucket` by `saveBucket`/`testBucket`, redacted by `getBucket`. */
export const SENSITIVE_FIELDS: readonly CredentialField[] = [
  'secret_access_key',
  'session_token',
  'credentials_json',
  'notifications.api_token',
]

const FIELD_SUFFIX: Readonly<Record<CredentialField, string>> = {
  access_key_id: 'ACCESS_KEY_ID',
  secret_access_key: 'SECRET_ACCESS_KEY',
  session_token: 'SESSION_TOKEN',
  account_id: 'ACCOUNT_ID',
  credentials_json: 'CREDENTIALS_JSON',
  'notifications.api_token': 'NOTIFICATIONS_API_TOKEN',
}

/** The key each field travels under in a `testBucket`/`saveBucket` `credentials` envelope. */
const CREDENTIAL_KEY: Readonly<Partial<Record<CredentialField, string>>> = {
  access_key_id: 'access_key_id',
  secret_access_key: 'secret_access_key',
  session_token: 'session_token',
  credentials_json: 'credentials_json',
  'notifications.api_token': 'notifications_api_token',
}

export const MAX_SECRET_NAME = 128

/**
 * The secret a bucket's credential is stored under:
 * `STORAGE_<BUCKET>_<FIELD>`, uppercased, every character outside
 * `[A-Z0-9_]` replaced by `_`, `NEW` for a bucket without a name yet. Long
 * names are cut so the secret stays within the secrets worker's
 * 128-character limit with the field intact. The same names in the console,
 * the iii app and the Apple app.
 */
export function secretNameFor(bucket: string, field: CredentialField): string {
  const suffix = `_${FIELD_SUFFIX[field]}`
  const handle =
    bucket
      .trim()
      .toUpperCase()
      .replace(/[^A-Z0-9_]/g, '_') || 'NEW'
  const room = MAX_SECRET_NAME - 'STORAGE_'.length - suffix.length
  return `STORAGE_${handle.slice(0, room)}${suffix}`
}

/** `base`, or `base_2`, `base_3`, … — the first one nothing else references. */
export function uniqueSecretName(base: string, taken: ReadonlySet<string>): string {
  if (!taken.has(base)) return base
  for (let n = 2; ; n++) {
    const suffix = `_${n}`
    const candidate = `${base.slice(0, MAX_SECRET_NAME - suffix.length)}${suffix}`
    if (!taken.has(candidate)) return candidate
  }
}

/** The value at a dotted field path (`notifications.api_token`). */
export function fieldValue(bucket: JsonObject, field: string): JsonValue | undefined {
  const [head, ...rest] = field.split('.')
  if (rest.length === 0) return bucket[head]
  const block = bucket[head]
  return isObject(block) ? fieldValue(block, rest.join('.')) : undefined
}

/** The bucket with one (dotted) field set, or removed when `value` is `undefined`. */
export function withField(bucket: JsonObject, field: string, value: JsonValue | undefined): JsonObject {
  const [head, key] = field.split('.')
  const next = { ...bucket }
  if (key === undefined) {
    if (value === undefined) delete next[head]
    else next[head] = value
    return next
  }
  const block = isObject(bucket[head]) ? { ...bucket[head] } : {}
  if (value === undefined) delete block[key]
  else block[key] = value
  next[head] = block
  return next
}

/**
 * Secret names referenced by any credential in the configuration, except
 * the one field being edited. Two buckets can derive the same name
 * (`a-b` and `a_b`); this is how the second notices and picks `_2` instead
 * of overwriting the first one's key.
 */
export function secretNamesInUse(
  buckets: JsonObject,
  except?: { bucket: string; field: CredentialField },
): Set<string> {
  const names = new Set<string>()
  for (const [name, value] of Object.entries(buckets)) {
    if (!isObject(value)) continue
    for (const field of CREDENTIAL_FIELDS) {
      if (except && except.bucket === name && except.field === field) continue
      const ref = secretRefName(fieldValue(value, field))
      if (ref) names.add(ref)
    }
  }
  return names
}

/**
 * Where a new value for a field goes. A field that already references a
 * secret rotates that secret in place — the reference stays valid — unless
 * another field (of this bucket or another) references it too: then the
 * value goes to the field's own name and only this field changes.
 */
export function storeTarget(
  buckets: JsonObject,
  bucket: string,
  field: CredentialField,
  current: JsonValue | undefined,
): string {
  const taken = secretNamesInUse(buckets, { bucket, field })
  const ref = secretRefName(current)
  if (ref && !taken.has(ref)) return ref
  return uniqueSecretName(secretNameFor(bucket, field), taken)
}

export type ValueKind = 'empty' | 'secret' | 'env' | 'template' | 'redacted' | 'literal' | 'opaque'

/** How a credential is held: by reference, as a template, or inline. */
export function valueKind(value: JsonValue | undefined): ValueKind {
  if (value === undefined || value === null) return 'empty'
  if (typeof value !== 'string') return 'opaque'
  if (value.trim() === '') return 'empty'
  if (value === REDACTED) return 'redacted'
  if (secretRefName(value)) return 'secret'
  if (envRefName(value)) return 'env'
  if (value.includes('${')) return 'template'
  return 'literal'
}

/** Whether any credential of the entry is a reference the secrets worker resolves. */
export function usesSecrets(bucket: JsonObject): boolean {
  return CREDENTIAL_FIELDS.some((field) => {
    const kind = valueKind(fieldValue(bucket, field))
    return kind === 'secret' || kind === 'env'
  })
}

/* ---------------- gcs service-account key ---------------- */

export type ServiceAccountCheck = { ok: true; email: string; project?: string } | { ok: false; reason: string }

/** A pasted service-account key, checked without keeping it: the fields GCS signing needs. */
export function checkServiceAccountJson(text: string): ServiceAccountCheck {
  const trimmed = text.trim().replace(/^﻿/, '')
  if (!trimmed) return { ok: false, reason: 'Paste the key’s JSON.' }
  let parsed: unknown
  try {
    parsed = JSON.parse(trimmed)
  } catch {
    return { ok: false, reason: 'This is not JSON.' }
  }
  if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) {
    return { ok: false, reason: 'Expected a JSON object.' }
  }
  const rec = parsed as Record<string, unknown>
  if (rec.type !== 'service_account')
    return { ok: false, reason: 'Not a service-account key (type is not “service_account”).' }
  if (typeof rec.client_email !== 'string' || !rec.client_email)
    return { ok: false, reason: 'The key has no client_email.' }
  if (typeof rec.private_key !== 'string' || !rec.private_key.includes('PRIVATE KEY')) {
    return { ok: false, reason: 'The key has no private_key.' }
  }
  return {
    ok: true,
    email: rec.client_email,
    project: typeof rec.project_id === 'string' ? rec.project_id : undefined,
  }
}

/* ---------------- display ---------------- */

/** The iii app's tones (Android `Palette.Tones`), as the database connections use them. */
export const BUCKET_COLORS: readonly string[] = [
  '#FAFAFA',
  '#2563EB',
  '#7C3AED',
  '#0891B2',
  '#16A34A',
  '#EA580C',
  '#DB2777',
  '#F59E0B',
  '#DC2626',
  '#64748B',
]

export function isHexColor(value: unknown): value is string {
  return typeof value === 'string' && /^#[0-9A-Fa-f]{6}$/.test(value)
}

export interface BucketDisplay {
  icon?: string
  color?: string
}

/** `display.icon` / `display.color` of an entry or a `listBuckets` row; a malformed colour is dropped. */
export function bucketDisplay(value: JsonValue | undefined): BucketDisplay {
  if (!isObject(value) || !isObject(value.display)) return {}
  const icon = str(value.display.icon) || undefined
  const color = isHexColor(value.display.color) ? value.display.color : undefined
  return { icon, color }
}

/** The entry with `display.<key>` set, or removed (and the block with it when empty). */
export function withDisplay(bucket: JsonObject, key: 'icon' | 'color', value: string | undefined): JsonObject {
  const next = withField(bucket, `display.${key}`, value)
  if (isObject(next.display) && Object.keys(next.display).length === 0) delete next.display
  return next
}

/* ---------------- names ---------------- */

/** The worker's bucket-name rule (`validate_bucket_names`). */
export function bucketNameError(name: string): string | undefined {
  if (/^[a-z0-9][a-z0-9_-]{0,62}$/.test(name)) return undefined
  return 'Use 1–63 characters: start with a lowercase letter or number, then only a–z, 0–9, _ or -.'
}

export function nextBucketName(buckets: JsonObject): string {
  if (buckets.bucket === undefined) return 'bucket'
  let suffix = 2
  while (buckets[`bucket-${suffix}`] !== undefined) suffix += 1
  return `bucket-${suffix}`
}

/* ---------------- what's missing ---------------- */

/**
 * What an entry still needs before the worker can build it — the checks a
 * form can see coming. Field paths are relative to the entry.
 */
export function bucketProblems(bucket: JsonObject): { field: string; message: string }[] {
  const out: { field: string; message: string }[] = []
  const blank = (v: JsonValue | undefined) => typeof v !== 'string' || v.trim() === ''
  const choice = providerChoiceOf(bucket)
  if ((choice === 's3' || choice === 's3-compatible') && blank(bucket.region)) {
    out.push({ field: 'region', message: 'Required.' })
  }
  if (choice === 's3' || choice === 's3-compatible') {
    const hasId = !blank(bucket.access_key_id)
    const hasSecret = !blank(bucket.secret_access_key)
    if (hasId !== hasSecret) {
      out.push({
        field: hasId ? 'secret_access_key' : 'access_key_id',
        message: 'Static credentials need both the access key ID and the secret access key.',
      })
    }
    if (!blank(bucket.session_token) && !hasId) {
      out.push({ field: 'session_token', message: 'A session token only applies with an access key.' })
    }
  }
  if (choice === 'r2') {
    if (blank(bucket.account_id)) out.push({ field: 'account_id', message: 'Required.' })
    if (blank(bucket.access_key_id)) out.push({ field: 'access_key_id', message: 'Required.' })
    if (blank(bucket.secret_access_key)) out.push({ field: 'secret_access_key', message: 'Required.' })
  }
  const endpoint = str(bucket.endpoint_url).trim()
  if (endpoint && !endpoint.includes('${') && !/^https?:\/\/[^/\s]+/i.test(endpoint)) {
    out.push({ field: 'endpoint_url', message: 'Use an http:// or https:// URL.' })
  }
  if (choice === 'gcs' && !blank(bucket.credentials_file) && !blank(bucket.credentials_json)) {
    out.push({
      field: 'credentials_json',
      message: 'Set one: the pasted service-account key or a key file on the engine host.',
    })
  }
  return out
}

/* ---------------- test bucket ---------------- */

/**
 * The `storage::testBucket` request for a draft entry. Literal credentials
 * (a typed secret key, a pasted service-account key, an access key ID kept
 * as text) travel in `credentials`, which the worker merges back: the
 * worker refuses sensitive literals inside `bucket`, and traces redact a
 * key named `credentials` whole. References (`secret://`, `env://`),
 * `${VAR}` templates and the `__redacted__` sentinel stay in place; `name`
 * goes along only when the draft holds the sentinel, which the worker fills
 * from the stored entry of that name.
 */
export function testBucketPayload(name: string, bucket: JsonObject, timeoutMs = 10_000): Record<string, unknown> {
  const entry: JsonObject = JSON.parse(JSON.stringify(bucket))
  const credentials: JsonObject = {}
  for (const field of CREDENTIAL_FIELDS) {
    const key = CREDENTIAL_KEY[field]
    if (!key) continue
    const value = fieldValue(entry, field)
    if (valueKind(value) !== 'literal') continue
    credentials[key] = value as JsonValue
    const [head, sub] = field.split('.')
    if (sub === undefined) delete entry[head]
    else if (isObject(entry[head])) delete entry[head][sub]
  }
  const payload: Record<string, unknown> = { bucket: entry, timeout_ms: timeoutMs }
  if (Object.keys(credentials).length > 0) payload.credentials = credentials
  if (JSON.stringify(bucket).includes(`"${REDACTED}"`)) payload.name = name
  return payload
}

export const TEST_STAGE_LABELS: Readonly<Record<string, string>> = {
  config: 'Configuration',
  secrets: 'Resolving secrets',
  endpoint: 'Endpoint',
  auth: 'Authentication',
  bucket: 'Bucket',
  list: 'Listing objects',
}

export function stageLabel(stage: string | null | undefined): string | null {
  return stage ? (TEST_STAGE_LABELS[stage] ?? stage) : null
}

export interface TestOutcome {
  ok: boolean
  provider?: string
  latency_ms?: number
  message?: string | null
  stage?: string | null
  bucket_missing?: boolean
  direct_downloads?: boolean
}

export function parseTestOutcome(raw: unknown): TestOutcome | null {
  if (typeof raw !== 'object' || raw === null || Array.isArray(raw)) return null
  const rec = raw as Record<string, unknown>
  if (typeof rec.ok !== 'boolean') return null
  return {
    ok: rec.ok,
    provider: typeof rec.provider === 'string' ? rec.provider : undefined,
    latency_ms: typeof rec.latency_ms === 'number' ? rec.latency_ms : undefined,
    message: typeof rec.message === 'string' ? rec.message : null,
    stage: typeof rec.stage === 'string' ? rec.stage : null,
    bucket_missing: rec.bucket_missing === true,
    direct_downloads: typeof rec.direct_downloads === 'boolean' ? rec.direct_downloads : undefined,
  }
}

/** One status line: `Connected · 42 ms` or where it failed. */
export function testOutcomeText(outcome: TestOutcome): string {
  const latency = typeof outcome.latency_ms === 'number' ? `${outcome.latency_ms} ms` : null
  if (outcome.ok) return ['Connected', latency].filter(Boolean).join(' · ')
  const stage = stageLabel(outcome.stage)
  const message = outcome.message || 'The bucket could not be reached'
  return stage ? `${stage} failed · ${message}` : message
}

/** The extra line under a result, when the worker said something actionable. */
export function testOutcomeHint(outcome: TestOutcome, underlying: string): string | null {
  if (!outcome.ok && outcome.bucket_missing) {
    return `There is no bucket “${underlying}” at this endpoint yet — create it with the provider (its console or CLI), then test again.`
  }
  if (outcome.ok && outcome.direct_downloads === false) {
    return 'Files can be read through the worker only: this bucket cannot sign download URLs.'
  }
  return null
}

/** The provider's bucket this entry addresses: `bucket`, or the entry's name. */
export function underlyingBucket(name: string, bucket: JsonObject): string {
  return str(bucket.bucket).trim() || name
}

/**
 * Whether this worker's configuration schema knows credentials by reference
 * (`credentials_json` and `display` arrived with them). Detected from the
 * JSON Schema the worker registered rather than assumed: an older worker
 * would read `secret://NAME` as the key itself. `null` without a schema.
 */
export function supportsSecretReferences(schema: Record<string, unknown> | null | undefined): boolean | null {
  if (!schema) return null
  let found = false
  const walk = (node: unknown, depth: number) => {
    if (found || depth > 14 || node === null || typeof node !== 'object') return
    if (Array.isArray(node)) {
      for (const item of node) walk(item, depth + 1)
      return
    }
    const rec = node as Record<string, unknown>
    const props = rec.properties
    if (props && typeof props === 'object' && !Array.isArray(props)) {
      const keys = Object.keys(props)
      if (keys.includes('credentials_json') || keys.includes('display')) {
        found = true
        return
      }
    }
    for (const value of Object.values(rec)) walk(value, depth + 1)
  }
  walk(schema, 0)
  return found
}

const FIELD_PHRASES: Readonly<Record<string, string>> = {
  region: 'the region',
  account_id: 'the account ID',
  access_key_id: 'the access key ID',
  secret_access_key: 'the secret access key',
  session_token: 'the session token',
  endpoint_url: 'the endpoint URL',
  credentials_json: 'the service-account key',
}

function joined(parts: readonly string[]): string {
  if (parts.length <= 1) return parts[0] ?? ''
  return `${parts.slice(0, -1).join(', ')} and ${parts[parts.length - 1]}`
}

/** Why "Test bucket" can't go yet, in one line beside the button; `null` when nothing blocks it. */
export function testBlocker(problems: readonly { field: string; message: string }[]): string | null {
  const missing = problems.filter((p) => p.message === 'Required.')
  if (missing.length > 0) return `Fill in ${joined(missing.map((p) => FIELD_PHRASES[p.field] ?? p.field))} to test`
  return problems[0]?.message ?? null
}
