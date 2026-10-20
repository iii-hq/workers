// Bucket management (getBucket / saveBucket / deleteBucket / testBucket),
// the extended listBuckets, ranged getObject and HTTP Range on presigned
// downloads. Bucket edits rewrite the worker's configuration, so the
// runner schedules these after every other suite; each case removes the
// buckets it adds.
import { forEachProvider, type Provider, type TestCase } from './cases.ts';

function assertEqual<T>(a: T, e: T, label: string): void {
  if (a !== e) throw new Error(`${label}: expected ${JSON.stringify(e)}, got ${JSON.stringify(a)}`);
}
function assertTruthy(c: unknown, m: string): void { if (!c) throw new Error(m); }

const MINIO = 'http://127.0.0.1:9000';
const MINIO_KEYS = { access_key_id: 'minioadmin', secret_access_key: 'minioadmin' };

async function bucketNames(call: (id: string, p: unknown) => Promise<any>): Promise<string[]> {
  const listed = await call('storage::listBuckets', {});
  return listed.buckets.map((b: any) => b.name);
}

// ---------------- ranged reads ----------------

export function buildRangeCases(providers: readonly Provider[]): TestCase[] {
  return [
    ...forEachProvider('ranged getObject: span, end, past the end', providers, async (ctx) => {
      const key = 'harness/range/ten';
      await ctx.call('storage::putObject', {
        bucket: ctx.bucket, key, body_base64: ctx.b64('0123456789'), content_type: 'text/plain',
      });
      const mid = await ctx.call('storage::getObject', { bucket: ctx.bucket, key, offset: 3, length: 4 });
      assertEqual(ctx.fromB64(mid.body_base64), '3456', 'mid body');
      assertEqual(mid.offset, 3, 'mid.offset');
      assertEqual(mid.length, 4, 'mid.length');
      assertEqual(mid.size, 10, 'mid.size is the object size');
      const tail = await ctx.call('storage::getObject', { bucket: ctx.bucket, key, offset: 8, length: 100 });
      assertEqual(ctx.fromB64(tail.body_base64), '89', 'clamped tail');
      const open = await ctx.call('storage::getObject', { bucket: ctx.bucket, key, offset: 6 });
      assertEqual(ctx.fromB64(open.body_base64), '6789', 'open-ended');
      const end = await ctx.call('storage::getObject', { bucket: ctx.bucket, key, offset: 10 });
      assertEqual(end.length, 0, 'a read at the end is empty');
      assertEqual(end.size, 10, 'end.size');
      const none = await ctx.call('storage::getObject', { bucket: ctx.bucket, key, offset: 2, length: 0 });
      assertEqual(none.length, 0, 'length 0');
      await ctx.expectError(
        () => ctx.call('storage::getObject', { bucket: ctx.bucket, key, offset: 11 }),
        'INVALID_RANGE',
      );
      const whole = await ctx.call('storage::getObject', { bucket: ctx.bucket, key });
      assertEqual(whole.offset, 0, 'whole.offset');
      assertEqual(whole.length, 10, 'whole.length');
    }),
    ...forEachProvider('ranged getObject: the cap is on the bytes returned', providers, async (ctx) => {
      const key = 'harness/range/eleven-mib';
      const big = Buffer.alloc(11 * 1024 * 1024, 7);
      big[big.length - 1] = 9;
      // Larger than the inline put cap: upload through a presigned PUT.
      const put = await ctx.call('storage::presignUrl', {
        bucket: ctx.bucket, key, method: 'PUT', content_type: 'application/octet-stream', expires_in_seconds: 60,
      });
      const uploaded = await fetch(put.url, {
        method: 'PUT', body: big, headers: { 'content-type': 'application/octet-stream' },
      });
      assertTruthy(uploaded.ok, `presigned PUT failed: ${uploaded.status}`);
      await ctx.expectError(() => ctx.call('storage::getObject', { bucket: ctx.bucket, key }), 'OBJECT_TOO_LARGE');
      const chunk = await ctx.call('storage::getObject', {
        bucket: ctx.bucket, key, offset: big.length - 2 * 1024 * 1024, length: 2 * 1024 * 1024,
      });
      assertEqual(chunk.length, 2 * 1024 * 1024, 'chunk.length');
      assertEqual(chunk.size, big.length, 'chunk.size');
      const bytes = Buffer.from(chunk.body_base64, 'base64');
      assertEqual(bytes[bytes.length - 1], 9, 'last byte');
      await ctx.call('storage::deleteObject', { bucket: ctx.bucket, key });
    }),
    ...forEachProvider('presigned GET serves HTTP Range and HEAD', providers, async (ctx) => {
      const key = 'harness/range/http';
      await ctx.call('storage::putObject', {
        bucket: ctx.bucket, key, body_base64: ctx.b64('0123456789'), content_type: 'video/mp4',
      });
      const { url } = await ctx.call('storage::presignUrl', {
        bucket: ctx.bucket, key, method: 'GET', expires_in_seconds: 60,
      });
      const part = await fetch(url, { headers: { range: 'bytes=2-5' } });
      assertEqual(part.status, 206, 'ranged status');
      assertEqual(part.headers.get('content-range'), 'bytes 2-5/10', 'content-range');
      assertEqual(await part.text(), '2345', 'ranged body');
      const tail = await fetch(url, { headers: { range: 'bytes=-3' } });
      assertEqual(await tail.text(), '789', 'suffix body');
      const past = await fetch(url, { headers: { range: 'bytes=10-' } });
      assertEqual(past.status, 416, 'unsatisfiable');
      await past.arrayBuffer();
      if (ctx.provider === 'local') {
        // Presigned S3 URLs are signed for GET only; the local route takes HEAD.
        const head = await fetch(url, { method: 'HEAD' });
        assertEqual(head.status, 200, 'HEAD status');
        assertEqual(head.headers.get('accept-ranges'), 'bytes', 'accept-ranges');
        assertEqual(head.headers.get('content-length'), '10', 'HEAD content-length');
      }
    }),
  ];
}

// ---------------- bucket management ----------------

export function buildBucketCases(providers: readonly Provider[]): TestCase[] {
  const cases: TestCase[] = [
    {
      name: 'listBuckets reports provider details, count and config_id',
      async run(ctx) {
        const listed = await ctx.call('storage::listBuckets', {});
        assertEqual(listed.count, listed.buckets.length, 'count');
        assertTruthy(typeof listed.config_id === 'string' && listed.config_id.length > 0, 'config_id');
        const local = listed.buckets.find((b: any) => b.name === 'scratch-local');
        assertTruthy(local, `scratch-local missing: ${JSON.stringify(listed)}`);
        assertEqual(local.provider, 'local', 'provider');
        assertEqual(local.bucket, 'scratch-local', 'underlying bucket');
        assertEqual(local.uses_secrets, false, 'uses_secrets');
        assertEqual(local.direct_downloads, true, 'direct_downloads');
        assertTruthy(local.error === undefined, `unexpected error: ${local.error}`);
      },
    },
    {
      name: 'testBucket: a stored local bucket by name is ok',
      async run(ctx) {
        const r = await ctx.call('storage::testBucket', { name: 'scratch-local' });
        assertEqual(r.ok, true, `ok (${r.message})`);
        assertEqual(r.provider, 'local', 'provider');
        assertTruthy(typeof r.latency_ms === 'number', 'latency_ms');
      },
    },
    {
      name: 'testBucket: literals, unknown keys and missing secrets fail as data',
      async run(ctx) {
        const literal = await ctx.call('storage::testBucket', {
          bucket: { provider: 's3', region: 'us-east-1', bucket: 'b', secret_access_key: 'hunter2' },
        });
        assertEqual(literal.ok, false, 'literal ok');
        assertEqual(literal.stage, 'config', 'literal stage');
        assertTruthy(!String(literal.message).includes('hunter2'), 'the literal is never echoed');
        const unknown = await ctx.call('storage::testBucket', {
          bucket: { provider: 'local', bucket: 'x', Bucket: 'y' },
        });
        assertEqual(unknown.stage, 'config', 'unknown key stage');
        const secret = await ctx.call('storage::testBucket', {
          bucket: {
            provider: 's3', region: 'us-east-1', bucket: 'b', endpoint_url: 'http://127.0.0.1:1',
            access_key_id: 'AKIA', secret_access_key: 'secret://HARNESS_MISSING_SECRET',
          },
          timeout_ms: 8000,
        });
        assertEqual(secret.ok, false, 'secret ok');
        assertEqual(secret.stage, 'secrets', `secret stage (${secret.message})`);
        assertTruthy(String(secret.message).includes('secret://HARNESS_MISSING_SECRET'), `names the reference: ${secret.message}`);
      },
    },
    {
      name: 'saveBucket: create, update, rename, delete a local bucket',
      async run(ctx) {
        const created = await ctx.call('storage::saveBucket', {
          name: 'harness-tmp',
          bucket: { provider: 'local', bucket: 'harness-tmp', display: { icon: 'folder', color: '#22C55E' } },
        });
        assertEqual(created.saved, true, 'saved');
        assertTruthy(created.error === undefined, `error: ${created.error}`);
        try {
          const listed = await ctx.call('storage::listBuckets', {});
          const tmp = listed.buckets.find((b: any) => b.name === 'harness-tmp');
          assertEqual(tmp?.display?.color, '#22C55E', 'display.color');
          await ctx.call('storage::putObject', {
            bucket: 'harness-tmp', key: 'kept.txt', body_base64: ctx.b64('kept'), content_type: 'text/plain',
          });
          const got = await ctx.call('storage::getBucket', { name: 'harness-tmp' });
          assertEqual(got.bucket.provider, 'local', 'getBucket provider');
          await ctx.expectError(
            () => ctx.call('storage::saveBucket', { name: 'harness-tmp', bucket: { provider: 'local' } }),
            'already exists',
          );
          await ctx.expectError(
            () => ctx.call('storage::saveBucket', {
              name: 'harness-tmp', previous_name: 'harness-tmp', bucket: { provider: 'local', colour: 'x' },
            }),
            'CONFIG_ERROR',
          );
          // Rename without an explicit provider-side bucket keeps the data.
          const renamed = await ctx.call('storage::saveBucket', {
            name: 'harness-tmp2', previous_name: 'harness-tmp', bucket: { provider: 'local' },
          });
          assertEqual(renamed.saved, true, 'renamed');
          const moved = await ctx.call('storage::getObject', { bucket: 'harness-tmp2', key: 'kept.txt' });
          assertEqual(ctx.fromB64(moved.body_base64), 'kept', 'the renamed bucket still holds the object');
          await ctx.expectError(
            () => ctx.call('storage::getObject', { bucket: 'harness-tmp', key: 'kept.txt' }),
            'UNKNOWN_BUCKET',
          );
          // The scratch bucket was untouched by all of it.
          assertTruthy((await bucketNames(ctx.call)).includes('scratch-local'), 'scratch-local kept');
        } finally {
          for (const name of ['harness-tmp', 'harness-tmp2']) {
            await ctx.call('storage::deleteBucket', { name }).catch(() => undefined);
          }
        }
        const again = await ctx.call('storage::deleteBucket', { name: 'harness-tmp2' });
        assertEqual(again.deleted, false, 'a second delete finds nothing');
        assertTruthy(!(await bucketNames(ctx.call)).includes('harness-tmp2'), 'deleted');
      },
    },
    {
      name: 'saveBucket: an unresolvable reference is saved and the bucket is unavailable',
      async run(ctx) {
        try {
          const saved = await ctx.call('storage::saveBucket', {
            name: 'harness-nosecret',
            bucket: {
              provider: 's3', region: 'us-east-1', bucket: 'nope', endpoint_url: 'http://127.0.0.1:1',
              access_key_id: 'AKIA', secret_access_key: 'secret://HARNESS_MISSING_SECRET',
            },
          });
          assertEqual(saved.saved, true, 'saved');
          assertEqual(saved.error_stage, 'secrets', `error_stage (${saved.error})`);
          await ctx.expectError(
            () => ctx.call('storage::listObjects', { bucket: 'harness-nosecret' }),
            'BUCKET_UNAVAILABLE',
          );
          const listed = await ctx.call('storage::listBuckets', {});
          const entry = listed.buckets.find((b: any) => b.name === 'harness-nosecret');
          assertEqual(entry?.error_stage, 'secrets', 'listed error_stage');
          assertEqual(entry?.uses_secrets, true, 'uses_secrets');
          // The others keep working.
          await ctx.call('storage::listObjects', { bucket: 'scratch-local', limit: 1 });
        } finally {
          await ctx.call('storage::deleteBucket', { name: 'harness-nosecret' }).catch(() => undefined);
        }
      },
    },
  ];

  if (providers.includes('s3')) {
    cases.push(
      {
        name: '[s3] testBucket against MinIO: ok, rejected keys, missing bucket',
        provider: 's3',
        async run(ctx) {
          const bucket = { provider: 's3', region: 'us-east-1', endpoint_url: MINIO, force_path_style: true };
          const ok = await ctx.call('storage::testBucket', {
            bucket: { ...bucket, bucket: 'scratch-s3' }, credentials: MINIO_KEYS,
          });
          assertEqual(ok.ok, true, `ok (${ok.message})`);
          assertEqual(ok.direct_downloads, true, 'direct_downloads');
          const auth = await ctx.call('storage::testBucket', {
            bucket: { ...bucket, bucket: 'scratch-s3' },
            credentials: { access_key_id: 'minioadmin', secret_access_key: 'wrong-secret' },
          });
          assertEqual(auth.stage, 'auth', `auth stage (${auth.message})`);
          assertTruthy(!String(auth.message).includes('wrong-secret'), 'scrubbed');
          const missing = await ctx.call('storage::testBucket', {
            bucket: { ...bucket, bucket: `harness-missing-${Date.now()}` }, credentials: MINIO_KEYS,
          });
          assertEqual(missing.stage, 'bucket', `bucket stage (${missing.message})`);
          assertEqual(missing.bucket_missing, true, 'bucket_missing');
          const down = await ctx.call('storage::testBucket', {
            bucket: { ...bucket, endpoint_url: 'http://127.0.0.1:1', bucket: 'x' }, credentials: MINIO_KEYS,
            timeout_ms: 5000,
          });
          assertEqual(down.stage, 'endpoint', `endpoint stage (${down.message})`);
        },
      },
      {
        name: '[s3] saveBucket create_bucket creates a missing MinIO bucket',
        provider: 's3',
        async run(ctx) {
          const underlying = `harness-created-${Date.now()}`;
          const entry = {
            provider: 's3', region: 'us-east-1', endpoint_url: MINIO, force_path_style: true, bucket: underlying,
            access_key_id: 'minioadmin',
          };
          try {
            const first = await ctx.call('storage::saveBucket', {
              name: 'harness-created', bucket: entry,
              credentials: { secret_access_key: 'minioadmin' }, create_bucket: true,
            });
            assertTruthy(first.error === undefined, `error: ${first.error}`);
            assertEqual(first.created_bucket, true, 'created_bucket');
            await ctx.call('storage::putObject', {
              bucket: 'harness-created', key: 'a.txt', body_base64: ctx.b64('a'), content_type: 'text/plain',
            });
            const shown = await ctx.call('storage::getBucket', { name: 'harness-created' });
            assertEqual(shown.bucket.secret_access_key, '__redacted__', 'the stored literal is redacted');
            const second = await ctx.call('storage::saveBucket', {
              name: 'harness-created', previous_name: 'harness-created',
              bucket: { ...entry, secret_access_key: '__redacted__' }, create_bucket: true,
            });
            assertEqual(second.created_bucket, false, 'already there');
            const got = await ctx.call('storage::getObject', { bucket: 'harness-created', key: 'a.txt' });
            assertEqual(ctx.fromB64(got.body_base64), 'a', 'the redacted secret was kept');
          } finally {
            await ctx.call('storage::deleteBucket', { name: 'harness-created' }).catch(() => undefined);
          }
        },
      },
    );
  }
  return cases;
}
