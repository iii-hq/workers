/**
 * Re-export of the iii-sdk surface every worker actually uses. Keeps
 * import paths short (`from '../runtime/iii.js'`) and makes it cheap to
 * mock the SDK in tests.
 */

export { registerWorker, TriggerAction, IIIInvocationError } from 'iii-sdk';
export type {
  ISdk,
  Channel,
  ChannelReader,
  ChannelWriter,
  FunctionRef,
  StreamChannelRef,
  Trigger,
  TriggerRequest,
  RegisterFunctionInput,
  RegisterFunctionOptions,
  RegisterTriggerInput,
  RegisterTriggerTypeInput,
  RemoteFunctionHandler,
  EnqueueResult,
  TriggerConfig,
  TriggerHandler,
} from 'iii-sdk';
