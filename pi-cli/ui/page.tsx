/**
 * The pi page: the shared agent terminal, told which worker it belongs to.
 * Everything else lives in @iii-workers/agent-terminal-ui, because
 * claude-code's page is the same page with another name on it.
 */
import { createAgentTerminalPage } from '@iii-workers/agent-terminal-ui';

export default createAgentTerminalPage({
  worker: 'pi-cli',
  title: 'pi',
  description: 'The pi coding agent on this engine',
});
