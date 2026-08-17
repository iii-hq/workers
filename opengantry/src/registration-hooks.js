import { isGantryNamespaceFunctionId, isReservedGovernanceFunctionId } from './namespace.js';

export function onFunctionRegistration(input) {
  if (isReservedGovernanceFunctionId(input.function_id)) {
    throw new Error(`reserved namespace: ${input.function_id}`);
  }
  return { function_id: input.function_id };
}

export function onTriggerRegistration(input) {
  if (isGantryNamespaceFunctionId(input.function_id)) {
    throw new Error('cannot bind trigger to gantry namespace');
  }
  return input;
}

export function onTriggerTypeRegistration() {
  return { denied: true };
}
