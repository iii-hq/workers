import { getNumber, getSection } from '../runtime/config.js';
import { DEFAULT_CONFIG, type PublishCollectConfig } from './publish-collect.js';

export function loadHookFanoutConfig(cfg: Record<string, unknown>): PublishCollectConfig {
  const section = getSection(cfg, 'hook_fanout');
  return {
    default_timeout_ms: getNumber(section, 'default_timeout_ms', DEFAULT_CONFIG.default_timeout_ms),
    min_timeout_ms: getNumber(section, 'min_timeout_ms', DEFAULT_CONFIG.min_timeout_ms),
    poll_interval_ms: getNumber(section, 'poll_interval_ms', DEFAULT_CONFIG.poll_interval_ms),
    quiescence_ms: getNumber(section, 'quiescence_ms', DEFAULT_CONFIG.quiescence_ms),
  };
}
