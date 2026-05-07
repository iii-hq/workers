// Negative-path coverage. The wire-stable error codes are defined in
// `storage/src/error.rs::StorageError`; substring-match against
// `e.message` because the SDK surfaces the JSON body verbatim there.
import type { TestCase } from './cases.ts';

export const ERROR_CASES: TestCase[] = [
  {
    name: 'getObject on unknown bucket → UNKNOWN_BUCKET',
    async run(ctx) {
      await ctx.expectError(
        async () =>
          ctx.call('storage::getObject', {
            bucket: 'definitely-not-configured',
            key: 'whatever',
          }),
        'UNKNOWN_BUCKET',
      );
    },
  },
  {
    name: 'getObject on missing key → OBJECT_NOT_FOUND',
    async run(ctx) {
      await ctx.expectError(
        async () =>
          ctx.call('storage::getObject', {
            bucket: ctx.bucket,
            key: 'harness/errors/missing-key',
          }),
        'OBJECT_NOT_FOUND',
      );
    },
  },
  {
    name: 'putObject with empty key → CONFIG_ERROR',
    async run(ctx) {
      await ctx.expectError(
        async () =>
          ctx.call('storage::putObject', {
            bucket: ctx.bucket,
            key: '',
            body_base64: ctx.b64('x'),
            content_type: 'text/plain',
          }),
        'CONFIG_ERROR',
      );
    },
  },
  {
    name: 'presignUrl PUT without content_type → INVALID_PRESIGN_PARAMS',
    async run(ctx) {
      await ctx.expectError(
        async () =>
          ctx.call('storage::presignUrl', {
            bucket: ctx.bucket,
            key: 'harness/errors/presign-no-ct',
            method: 'PUT',
            expires_in_seconds: 60,
          }),
        'INVALID_PRESIGN_PARAMS',
      );
    },
  },
  {
    name: 'presignUrl with expires_in_seconds below floor → INVALID_PRESIGN_PARAMS',
    async run(ctx) {
      await ctx.expectError(
        async () =>
          ctx.call('storage::presignUrl', {
            bucket: ctx.bucket,
            key: 'harness/errors/presign-too-short',
            method: 'GET',
            expires_in_seconds: 5, // floor is 30; cap is 86400 (see presign_url.rs)
          }),
        'INVALID_PRESIGN_PARAMS',
      );
    },
  },
];
