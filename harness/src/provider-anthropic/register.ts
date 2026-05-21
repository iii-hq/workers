import { loadConfig } from '../runtime/config.js';
import type { ISdk } from '../runtime/iii.js';
import { register as registerComplete } from './complete.js';
import { loadWorkerConfig } from './config.js';
import { register as registerStream } from './stream-fn.js';

export async function register(iii: ISdk, ctx: { configPath: string }): Promise<void> {
  const cfg = await loadConfig(ctx.configPath);
  const worker = loadWorkerConfig(cfg);
  registerComplete(iii, worker);
  registerStream(iii, worker);
}
