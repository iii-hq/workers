import { forEachProvider, type Provider, type TestCase } from './cases.ts';

function assertTruthy(c: unknown, m: string): void { if (!c) throw new Error(m); }

export function buildConcurrencyCases(providers: readonly Provider[]): TestCase[] {
  return [
    ...forEachProvider('concurrent overwrite — last write wins', providers, async (ctx) => {
      const key = 'harness/concurrency/overwrite';
      const bodies = ['v1', 'v2', 'v3', 'v4', 'v5', 'v6', 'v7', 'v8'];
      const puts = await Promise.all(
        bodies.map((body) =>
          ctx.call('storage::putObject', {
            bucket: ctx.bucket, key, body_base64: ctx.b64(body), content_type: 'text/plain',
          }),
        ),
      );
      const etags = new Set(puts.map((p) => p.etag as string));
      assertTruthy(etags.size >= 1, `expected at least one etag, got: ${JSON.stringify([...etags])}`);

      const got = await ctx.call('storage::getObject', { bucket: ctx.bucket, key });
      const finalBody = ctx.fromB64(got.body_base64);
      assertTruthy(bodies.includes(finalBody), `final body "${finalBody}" not one of the 8 puts`);
      assertTruthy(etags.has(got.etag),
        `final etag ${got.etag} doesn't match any of: ${JSON.stringify([...etags])}`);
    }),
  ];
}
