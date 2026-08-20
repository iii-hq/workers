/**
 * Engine RBAC hooks on the governed listener. A sibling worker must not
 * register gantry::* (or reserved suffixes) or bind triggers into that
 * namespace. Trigger-type registration is always denied so agents cannot
 * mint a competing gantry::verdict on this port.
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

export function onFunctionRegistration(input) {
  if (isReservedGovernanceFunctionId(input.function_id)) {
    throw new GantryDenied('REGISTRATION_DENIED', `reserved namespace: ${input.function_id}`);
  }
  return { function_id: input.function_id };
}

export function onTriggerRegistration(input) {
  if (isGantryNamespaceFunctionId(input.function_id)) {
    throw new GantryDenied('REGISTRATION_DENIED', 'cannot bind trigger to gantry namespace');
  }
  return input;
}

export function onTriggerTypeRegistration() {
  throw new GantryDenied(
    'TRIGGER_TYPE_REGISTRATION_DENIED',
    'trigger type registration is not allowed on governed port',
  );
}
