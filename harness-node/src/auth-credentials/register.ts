import { loadConfig } from '../runtime/config.js';
import type { ISdk } from '../runtime/iii.js';
import { loadAuthCredentialsConfig } from './config.js';
import { register as registerDelete } from './handlers/delete-token.js';
import { register as registerGet } from './handlers/get-token.js';
import { register as registerList } from './handlers/list-providers.js';
import { register as registerSet } from './handlers/set-token.js';
import { register as registerStatus } from './handlers/status.js';
import { FileStore } from './store.js';

export async function register(iii: ISdk, ctx: { configPath: string }): Promise<void> {
  const cfg = await loadConfig(ctx.configPath);
  const auth = loadAuthCredentialsConfig(cfg);
  const store = new FileStore(auth.store_path);
  registerGet(iii, store);
  registerSet(iii, store);
  registerDelete(iii, store);
  registerList(iii, store);
  registerStatus(iii, store);
}
