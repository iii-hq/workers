import type { ISdk } from '../runtime/iii.js';
import { register as registerGet } from './handlers/get.js';
import { register as registerList } from './handlers/list.js';
import { register as registerReconcile } from './handlers/reconcile.js';
import { register as registerSupports } from './handlers/supports.js';

export function register(iii: ISdk): void {
  registerList(iii);
  registerGet(iii);
  registerSupports(iii);
  registerReconcile(iii);
}
