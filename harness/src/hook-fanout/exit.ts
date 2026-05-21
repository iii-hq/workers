/**
 * Exit decision for the publish-collect loop. Pure helper, mirrors
 * `hook-fanout/src/handler.rs::decide_exit`.
 */

export type ExitReason = 'expected_replies' | 'quiescence' | 'deadline' | 'deadline_no_replies';

export type ExitInput = {
  replies_count: number;
  expected_replies?: number;
  quiescence_ms: number;
  last_reply_at_ms?: number;
  now_ms: number;
  deadline_ms: number;
};

export function decideExit(input: ExitInput): ExitReason | null {
  if (input.expected_replies !== undefined && input.replies_count >= input.expected_replies) {
    return 'expected_replies';
  }
  if (input.quiescence_ms > 0 && input.last_reply_at_ms !== undefined) {
    if (input.now_ms - input.last_reply_at_ms >= input.quiescence_ms) {
      return 'quiescence';
    }
  }
  if (input.now_ms >= input.deadline_ms) {
    return input.last_reply_at_ms !== undefined ? 'deadline' : 'deadline_no_replies';
  }
  return null;
}
