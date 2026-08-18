/**
 * Reserved ids for the governed bus. Prefixes belong to this worker.
 * Suffixes ::verify / ::attest / ::promote are reserved bus-wide so a
 * sibling cannot register demo::verify as a fake gate.
 */
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
