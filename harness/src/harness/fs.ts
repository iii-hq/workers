/**
 * `harness::fs::read_inline` — wrap `shell::fs::read` and inline the
 * binary stream into the legacy `{content, details}` envelope. Mirrors
 * `harness/src/fs.rs`.
 */

import { unwrapBody } from '../runtime/handler.js';
import type { ISdk, StreamChannelRef } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';

export const DEFAULT_MAX_INLINE_BYTES = 256 * 1024;

type ContentRef = { channel_id: string; access_key: string; direction?: string };
type ShellFsReadResponse = {
  content: ContentRef;
  size: number;
  mode?: string;
  mtime?: number;
};

export function buildInlineEnvelope(bytes: Uint8Array, totalSize: number): Record<string, unknown> {
  const bytes_read = bytes.length;
  const truncated = bytes_read < totalSize;
  let text: string;
  try {
    text = new TextDecoder('utf-8', { fatal: true }).decode(bytes);
  } catch {
    text = `<binary ${bytes_read} bytes>`;
  }
  return {
    content: [{ type: 'text', text }],
    details: { size: totalSize, truncated, bytes_read },
    terminate: false,
  };
}

export function register(iii: ISdk, engineWsBase: string): void {
  iii.registerFunction(
    'harness::fs::read_inline',
    async (raw: unknown) => {
      const body = unwrapBody(raw);
      const path = typeof body.path === 'string' ? body.path : null;
      if (!path) throw new Error('missing required field: path');
      const max_bytes =
        typeof body.max_bytes === 'number' && body.max_bytes > 0
          ? body.max_bytes
          : DEFAULT_MAX_INLINE_BYTES;

      const raw2 = await iii.trigger<unknown, ShellFsReadResponse>({
        function_id: 'shell::fs::read',
        payload: { path },
        timeoutMs: 30_000,
      });
      if (!raw2 || typeof raw2 !== 'object' || !raw2.content) {
        throw new Error('shell::fs::read returned unexpected shape');
      }

      const { ChannelReader } = await import('iii-sdk');
      const ref: StreamChannelRef = {
        channel_id: raw2.content.channel_id,
        access_key: raw2.content.access_key,
        direction: 'read',
      };
      const reader = new ChannelReader(engineWsBase, ref);
      try {
        return await new Promise<Record<string, unknown>>((resolve, reject) => {
          const chunks: Buffer[] = [];
          let total = 0;
          let resolved = false;
          const finish = () => {
            if (resolved) return;
            resolved = true;
            try {
              reader.close();
            } catch (err) {
              logger.debug('reader.close failed', { err: String(err) });
            }
            const concat = Buffer.concat(chunks, Math.min(total, max_bytes));
            resolve(buildInlineEnvelope(concat, raw2.size));
          };
          reader.stream.on('data', (chunk: Buffer) => {
            if (resolved) return;
            const remaining = max_bytes - total;
            if (remaining <= 0) {
              finish();
              return;
            }
            const slice = chunk.length <= remaining ? chunk : chunk.subarray(0, remaining);
            chunks.push(slice);
            total += slice.length;
            if (total >= max_bytes) finish();
          });
          reader.stream.on('end', finish);
          reader.stream.on('close', finish);
          reader.stream.on('error', (err: unknown) => {
            if (resolved) return;
            resolved = true;
            try {
              reader.close();
            } catch {
              // ignore
            }
            reject(err instanceof Error ? err : new Error(String(err)));
          });
        });
      } catch (err) {
        throw new Error(`channel drain failed: ${String(err)}`);
      }
    },
    {
      description:
        'Read a host file via shell::fs::read, drain its channel, and return a {content:[{text}], details:{size, truncated, bytes_read}} envelope (max 256 KiB inline by default).',
    },
  );
}
