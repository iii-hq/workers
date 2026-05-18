import { getSection, getString } from '../runtime/config.js';

export type ApprovalGateConfig = {
  topic: string;
  approval_state_scope: string;
  policy_function_id: string;
};

export function loadApprovalGateConfig(cfg: Record<string, unknown>): ApprovalGateConfig {
  const section = getSection(cfg, 'approval_gate');
  return {
    topic: getString(section, 'topic', 'agent::before_function_call'),
    approval_state_scope: getString(section, 'approval_state_scope', 'approvals'),
    policy_function_id: getString(section, 'policy_function_id', 'policy::check_permissions'),
  };
}
