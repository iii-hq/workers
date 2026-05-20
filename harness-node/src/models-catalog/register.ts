import type { ISdk } from '../runtime/iii.js';
import { register as registerGet } from './handlers/get.js';
import { register as registerList } from './handlers/list.js';
import { register as registerRegister } from './handlers/register.js';
import { register as registerSupports } from './handlers/supports.js';
import { DEFAULT_STATE_CONFIG, seedStateIfEmpty } from './state.js';

export async function register(iii: ISdk): Promise<void> {
  // Seed asynchronously; don't block boot — the engine may not have
  // state::* online yet, but list/get/supports fall back to embedded.
  void seedStateIfEmpty(iii, DEFAULT_STATE_CONFIG);
  registerList(iii);
  registerGet(iii);
  registerSupports(iii);
  registerRegister(iii);
}
