// Shared types and the round-trip RPC suite. Each case is self-contained;
// keys are namespaced under `harness/<case-name>/...` so they don't collide
// across runs (the schema-reset case best-effort-deletes the namespace at
// the start of the run).
import type { ISdk } from 'iii-sdk';

export type Provider = 'local' | 's3' | 'r2';

export const ALL_PROVIDERS: readonly Provider[] = ['local', 's3', 'r2'];

export const BUCKETS: Record<Provider, string> = {
  local: 'scratch-local',
  s3:    'scratch-s3',
  r2:    'scratch-r2',
};

/** Single-bucket constant retained for trigger cases that pin to local. */
export const LOCAL_BUCKET = BUCKETS.local;

/** Discriminator used by trigger cases to route on `harness::on_object_event`. */
export type ObjectEventKind = 'created' | 'deleted';

export interface ObjectEvent {
  kind: ObjectEventKind;
  bucket: string;
  key: string;
  size?: number;
  etag?: string;
  raw_event_id: string;
  /** Wall-clock time the harness saw the dispatch (not the source-side ts). */
  received_at: number;
}

export interface CaseContext {
  bucket: string;
  /** Which provider this case execution is targeting. */
  provider: Provider;
  iii: ISdk;

  /** Invoke an storage RPC; throws on Err. */
  call: (functionId: string, payload: unknown) => Promise<any>;

  /** base64-encode a string or Buffer for putObject's body_base64 field. */
  b64: (s: string | Buffer) => string;

  /** Decode a getObject response's body_base64 back to UTF-8. */
  fromB64: (s: string) => string;

  /**
   * Wait up to `timeoutMs` for an event of kind `kind` whose key equals
   * `key`. Returns the event (oldest-first match). If `timeoutMs` elapses
   * with no match, throws.
   */
  waitForEvent: (kind: ObjectEventKind, key: string, timeoutMs: number) => Promise<ObjectEvent>;

  /**
   * Wait `timeoutMs` and assert no events arrived in that window. Used by
   * the "GET should not produce events" case.
   */
  expectSilence: (timeoutMs: number) => Promise<void>;

  /** Drop buffered events; resolves any pending waiters with rejection. */
  resetEvents: () => void;

  /**
   * Run `fn`, expect it to throw, and assert the thrown error's message
   * contains `expectedCode` (substring match — error bodies are JSON like
   * `{"code":"OBJECT_NOT_FOUND",...}`).
   */
  expectError: (fn: () => Promise<unknown>, expectedCode: string) => Promise<void>;
}

export interface TestCase {
  name: string;
  /** When set, the case targets a specific provider. Unset means "any provider"
   *  (used by trigger cases that pin to local). */
  provider?: Provider;
  /** When set, overrides the runner's default bucket selection for this case. */
  bucket?: string;
  run(ctx: CaseContext): Promise<void>;
}

// ---------------- assertions ----------------

function assertEqual<T>(actual: T, expected: T, label: string): void {
  if (actual !== expected) {
    throw new Error(`${label}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`);
  }
}

function assertTruthy(cond: unknown, msg: string): void {
  if (!cond) throw new Error(msg);
}

// ---------------- schema reset ----------------

/**
 * Best-effort cleanup of harness-owned keys at the start of a run. Failures
 * here are tolerated — the bucket may be empty on the first run, and
 * deleteObject is idempotent (returns deleted: false for missing keys).
 */
export function buildSchemaReset(providers: readonly Provider[]): TestCase {
  return {
    name: 'schema-reset',
    async run(ctx) {
      const keys = [
        'harness/roundtrip', 'harness/overwrite', 'harness/delete-target',
        'harness/presign-get', 'harness/presign-put',
        'harness/triggers/created-target', 'harness/triggers/deleted-target',
        'harness/triggers/multi-put', 'harness/triggers/silence-probe',
        'harness/errors/missing-key',
      ];
      for (const provider of providers) {
        for (const key of keys) {
          try {
            await ctx.call('storage::deleteObject', { bucket: BUCKETS[provider], key });
          } catch {
            // best effort
          }
        }
      }
    },
  };
}

// ---------------- RPC round-trip suite ----------------

export function buildFunctionCases(providers: readonly Provider[]): TestCase[] {
  return [
    ...forEachProvider('putObject + getObject round-trip', providers, async (ctx) => {
      const key = 'harness/roundtrip';
      const body = 'hello-storage';
      const put = await ctx.call('storage::putObject', {
        bucket: ctx.bucket, key, body_base64: ctx.b64(body), content_type: 'text/plain',
      });
      assertTruthy(typeof put.etag === 'string' && put.etag.length > 0, `etag missing: ${JSON.stringify(put)}`);
      assertEqual(put.size, body.length, 'put.size');
      const got = await ctx.call('storage::getObject', { bucket: ctx.bucket, key });
      assertEqual(ctx.fromB64(got.body_base64), body, 'getObject body');
      assertEqual(got.size, body.length, 'getObject size');
      assertEqual(got.content_type, 'text/plain', 'getObject content_type');
      assertTruthy(typeof got.etag === 'string' && got.etag.length > 0, 'getObject etag missing');
      assertTruthy(typeof got.last_modified === 'string', 'getObject last_modified missing');
    }),
    ...forEachProvider('putObject overwrite changes etag', providers, async (ctx) => {
      const key = 'harness/overwrite';
      const v1 = await ctx.call('storage::putObject', {
        bucket: ctx.bucket, key, body_base64: ctx.b64('first'), content_type: 'text/plain',
      });
      const v2 = await ctx.call('storage::putObject', {
        bucket: ctx.bucket, key, body_base64: ctx.b64('second-larger'), content_type: 'text/plain',
      });
      assertTruthy(v2.etag !== v1.etag, `etag should change on overwrite: ${v1.etag} == ${v2.etag}`);
      assertEqual(v2.size, 'second-larger'.length, 'overwrite size');
      const got = await ctx.call('storage::getObject', { bucket: ctx.bucket, key });
      assertEqual(ctx.fromB64(got.body_base64), 'second-larger', 'overwrite body');
    }),
    ...forEachProvider('deleteObject removes the key', providers, async (ctx) => {
      const key = 'harness/delete-target';
      await ctx.call('storage::putObject', {
        bucket: ctx.bucket, key, body_base64: ctx.b64('byebye'), content_type: 'text/plain',
      });
      const del = await ctx.call('storage::deleteObject', { bucket: ctx.bucket, key });
      assertEqual(del.deleted, true, 'delete returned deleted=false');
      await ctx.expectError(
        async () => ctx.call('storage::getObject', { bucket: ctx.bucket, key }),
        'OBJECT_NOT_FOUND',
      );
    }),
    ...forEachProvider('presignUrl GET returns a usable URL', providers, async (ctx) => {
      const key = 'harness/presign-get';
      await ctx.call('storage::putObject', {
        bucket: ctx.bucket, key, body_base64: ctx.b64('presign-target'), content_type: 'text/plain',
      });
      const r = await ctx.call('storage::presignUrl', {
        bucket: ctx.bucket, key, method: 'GET', expires_in_seconds: 60,
      });
      assertTruthy(typeof r.url === 'string' && r.url.startsWith('http'), `url not http: ${r.url}`);
      assertTruthy(r.url.includes(key), `url missing key: ${r.url}`);
      const expiresAt = Date.parse(r.expires_at);
      assertTruthy(Number.isFinite(expiresAt), `expires_at not parseable: ${r.expires_at}`);
      assertTruthy(expiresAt > Date.now(), `expires_at in past: ${r.expires_at}`);
    }),
    ...forEachProvider('presignUrl PUT with content_type succeeds', providers, async (ctx) => {
      const key = 'harness/presign-put';
      const r = await ctx.call('storage::presignUrl', {
        bucket: ctx.bucket, key, method: 'PUT', content_type: 'application/json', expires_in_seconds: 60,
      });
      assertTruthy(typeof r.url === 'string' && r.url.startsWith('http'), `url not http: ${r.url}`);
      const expiresAt = Date.parse(r.expires_at);
      assertTruthy(expiresAt > Date.now(), `expires_at in past: ${r.expires_at}`);
    }),
  ];
}

/**
 * Generate one TestCase per active provider from a single template.
 * Trigger cases that pin to local stay as plain TestCase entries (no helper).
 */
export function forEachProvider(
  baseName: string,
  providers: readonly Provider[],
  body: (ctx: CaseContext) => Promise<void>,
): TestCase[] {
  return providers.map((p) => ({
    name: `[${p}] ${baseName}`,
    provider: p,
    bucket: BUCKETS[p],
    run: body,
  }));
}
