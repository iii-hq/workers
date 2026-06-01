import type { ISdk } from '../runtime/iii.js';
import { register as registerGet } from './handlers/get.js';
import { register as registerList } from './handlers/list.js';
import { register as registerRegister } from './handlers/register.js';
import { register as registerSupports } from './handlers/supports.js';

export async function register(iii: ISdk): Promise<void> {
  // No embedded seed: the catalog is populated solely by provider discovery
  // (`models::register`). list/get/supports read only from iii state.
  registerList(iii);
  registerGet(iii);
  registerSupports(iii);
  registerRegister(iii);
}
