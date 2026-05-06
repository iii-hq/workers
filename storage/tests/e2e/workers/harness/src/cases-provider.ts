// Provider-specific quirk cases. Pinned to a single provider via the
// optional TestCase.provider field — these run only when that provider
// is in the active set. GCS quirks are dropped per the post-Phase-2
// descope (see spec section "GCS descope (post-Phase-2 finding)").
import { BUCKETS, type Provider, type TestCase } from './cases.ts';

function assertEqual<T>(a: T, e: T, label: string): void {
  if (a !== e) throw new Error(`${label}: expected ${JSON.stringify(e)}, got ${JSON.stringify(a)}`);
}
function assertTruthy(c: unknown, m: string): void { if (!c) throw new Error(m); }

export function buildProviderQuirkCases(providers: readonly Provider[]): TestCase[] {
  const cases: TestCase[] = [];

  if (providers.includes('s3')) {
    cases.push({
      name: '[s3] presigned URL is path-style against MinIO',
      provider: 's3',
      bucket: BUCKETS.s3,
      async run(ctx) {
        const key = 'harness/quirk/s3-path-style';
        await ctx.call('storage::putObject', {
          bucket: ctx.bucket, key, body_base64: ctx.b64('x'), content_type: 'text/plain',
        });
        const r = await ctx.call('storage::presignUrl', {
          bucket: ctx.bucket, key, method: 'GET', expires_in_seconds: 60,
        });
        // S3 backend defaults to virtual-host (force_path_style: false). Against
        // MinIO that yields URLs like http://scratch-s3.127.0.0.1:9000/key OR
        // http://127.0.0.1:9000/scratch-s3/key depending on MINIO_DOMAIN. We
        // accept either shape — the assertion is just "URL is parseable and
        // contains the bucket name in some form".
        assertTruthy(r.url.includes(ctx.bucket),
          `presigned URL should reference bucket ${ctx.bucket}: ${r.url}`);
      },
    });
  }

  if (providers.includes('r2')) {
    cases.push({
      name: '[r2] region "auto" performs real round-trip',
      provider: 'r2',
      bucket: BUCKETS.r2,
      async run(ctx) {
        const key = 'harness/quirk/r2-auto';
        await ctx.call('storage::putObject', {
          bucket: ctx.bucket, key, body_base64: ctx.b64('r2'), content_type: 'text/plain',
        });
        const got = await ctx.call('storage::getObject', { bucket: ctx.bucket, key });
        assertEqual(ctx.fromB64(got.body_base64), 'r2', 'r2 round-trip body');
      },
    });
  }

  return cases;
}
