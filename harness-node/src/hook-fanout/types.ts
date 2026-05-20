export const FUNCTION_ID = 'hook-fanout::publish_collect';
export const REPLY_HANDLER_FN_ID = 'hook-fanout::reply_handler';
export const HOOK_REPLY_STREAM = 'agent::hook_reply';

export type MergeRule = 'first_block_wins' | 'field_merge' | 'pipeline_last_wins';

export function parseMergeRule(value: string): MergeRule | null {
  if (value === 'first_block_wins' || value === 'field_merge' || value === 'pipeline_last_wins') {
    return value;
  }
  return null;
}

export type PublishCollectRequest = {
  topic: string;
  payload: unknown;
  merge_rule: MergeRule;
  timeout_ms?: number;
  expected_replies?: number;
  quiescence_ms?: number;
};

export type PublishCollectResponse = {
  event_id: string;
  replies: unknown[];
  merged: unknown;
};
