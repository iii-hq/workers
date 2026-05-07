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
      // Every put must report a non-empty etag. The previous `>= 1` check
      // could pass even when most puts returned `undefined`/`''`, hiding
      // backend bugs that drop the ETag header on concurrent writes.
      assertTruthy(
        puts.every((p) => typeof p.etag === 'string' && p.etag.length > 0),
        `every put must return a non-empty etag; got: ${JSON.stringify(puts.map((p) => p.etag))}`,
      );
      const etags = new Set(puts.map((p) => p.etag as string));

      const got = await ctx.call('storage::getObject', { bucket: ctx.bucket, key });
      const finalBody = ctx.fromB64(got.body_base64);
      assertTruthy(bodies.includes(finalBody), `final body "${finalBody}" not one of the 8 puts`);
      assertTruthy(etags.has(got.etag),
        `final etag ${got.etag} doesn't match any of: ${JSON.stringify([...etags])}`);
    }),
  ];
}
