/**
 * The claude page: the shared agent terminal, told which worker it belongs to.
 * Everything else — the session, the ordered writer, the lease across
 * remounts — lives in @iii-workers/agent-terminal-ui, because pi's page
 * is the same page with another name on it.
 */
import { createAgentTerminalPage } from '@iii-workers/agent-terminal-ui';

export default createAgentTerminalPage({
  worker: 'claude',
  title: 'claude',
  description: 'Claude Code on this engine',
});
