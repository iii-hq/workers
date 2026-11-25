/**
 * The claude page: the shared agent terminal, told which worker it belongs to,
 * with this worker's Sessions rail beside it. Everything else — the session,
 * the ordered writer, the lease across remounts — lives in
 * @iii-workers/agent-terminal-ui, because pi's page is the same page with
 * another name on it (and no rail).
 */
import { createAgentTerminalPage } from '@iii-workers/agent-terminal-ui';
import { SessionsRail } from './sessions';

export default createAgentTerminalPage({
  worker: 'claude',
  title: 'claude',
  description: 'Claude Code on this engine',
  // Its own folder pill: the session starts in the chat's folder, and a pick
  // restarts Claude there (claude::terminal::describe takes { cwd }).
  folderPicker: true,
  sidebar: {
    label: 'Sessions',
    storageKey: 'claude:sessions',
    render: (host, terminal) => <SessionsRail host={host} terminal={terminal} />,
  },
});
