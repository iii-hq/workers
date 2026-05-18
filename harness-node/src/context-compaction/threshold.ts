import { triggerTokens } from './config.js';

export function shouldCompact(total_tokens: number): boolean {
  return total_tokens >= triggerTokens();
}
