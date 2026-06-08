import type { ISdk } from '../runtime/iii.js';
import { registerTree } from './tree/register.js';
import { IiiStateSessionStore } from './tree/store.js';

export async function register(iii: ISdk, _ctx: { configPath: string }): Promise<void> {
  registerTree(iii, new IiiStateSessionStore(iii));
}
