import { loadConfig } from '../runtime/config.js';
import type { ISdk } from '../runtime/iii.js';
import { loadSessionConfig } from './config.js';
import { registerInbox } from './inbox/handlers.js';
import { registerTree } from './tree/register.js';
import { IiiStateSessionStore, InMemoryStore, type SessionStore } from './tree/store.js';

export async function register(iii: ISdk, ctx: { configPath: string }): Promise<void> {
  const cfg = await loadConfig(ctx.configPath);
  const sessionCfg = loadSessionConfig(cfg);
  const store: SessionStore =
    sessionCfg.store_backend === 'memory' ? new InMemoryStore() : new IiiStateSessionStore(iii);
  registerTree(iii, store);
  registerInbox(iii, { state_scope: sessionCfg.state_scope });
}
