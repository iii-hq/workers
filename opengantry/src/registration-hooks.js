/**
 * Engine RBAC hooks on the governed listener. A sibling worker must not
 * register gantry::* (or reserved suffixes), and an observer created on this
 * port must not target the gantry namespace. Only this worker calls
 * gantry::on-verdict (it is private), so a sibling cannot forge a verdict
 * event; observing it is allowed.
 */
import { GantryDenied } from './denied.js';

const RESERVED_PREFIXES = ['gantry::', 'opengantry::'];
const RESERVED_SUFFIXES = ['::verify', '::attest', '::promote'];

export function isReservedGovernanceFunctionId(functionId) {
  const id = functionId.toLowerCase();
  if (RESERVED_PREFIXES.some((p) => id.startsWith(p))) return true;
  if (RESERVED_SUFFIXES.some((s) => id.endsWith(s))) return true;
  return false;
}

export function isGantryNamespaceFunctionId(functionId) {
  return functionId.toLowerCase().startsWith('gantry::');
}

/**
 * `on_function_registration_function_id`: the engine calls it with
 * `{ function_id, description, metadata, namespace, private, observable,
 * context, args_format? }`. An object answer accepts the registration and
 * rewrites only the keys it carries: `{ function_id }` keeps the declared
 * description, metadata, private, observable and args_format. A throw denies
 * it.
 */
export function onFunctionRegistration(input) {
  if (isReservedGovernanceFunctionId(input.function_id)) {
    throw new GantryDenied('REGISTRATION_DENIED', `reserved namespace: ${input.function_id}`);
  }
  return { function_id: input.function_id };
}

/**
 * `on_observer_registration_function_id`: the engine calls it with
 * `{ observer_id, function_id, namespace, args, filters, target_function_id,
 * target_namespace, metadata, context }`. An object answer accepts the
 * observer (its keys rewrite the registration; the empty object rewrites
 * nothing); a throw denies it with `FORBIDDEN`.
 */
export function onObserverRegistration(input) {
  if (isGantryNamespaceFunctionId(input.target_function_id)) {
    throw new GantryDenied(
      'REGISTRATION_DENIED',
      'cannot target an observer at the gantry namespace',
    );
  }
  return {};
}
