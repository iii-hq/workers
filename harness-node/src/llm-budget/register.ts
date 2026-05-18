import type { ISdk } from '../runtime/iii.js';
import {
  registerAlertSet,
  registerCheck,
  registerCreate,
  registerDelete,
  registerEnforce,
  registerExempt,
  registerForecast,
  registerGet,
  registerList,
  registerPause,
  registerRecord,
  registerReset,
  registerUpdate,
  registerUsage,
} from './handlers/index.js';

export async function register(iii: ISdk): Promise<void> {
  registerCreate(iii);
  registerList(iii);
  registerGet(iii);
  registerUpdate(iii);
  registerDelete(iii);
  registerCheck(iii);
  registerRecord(iii);
  registerReset(iii);
  registerAlertSet(iii);
  registerUsage(iii);
  registerForecast(iii);
  registerEnforce(iii);
  registerExempt(iii);
  registerPause(iii);
}
