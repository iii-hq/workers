import { getSection, getString } from '../runtime/config.js';

export type ApprovalGateConfig = {
  approval_state_scope: string;
};

export function loadApprovalGateConfig(cfg: Record<string, unknown>): ApprovalGateConfig {
  const section = getSection(cfg, 'approval_gate');
  return {
    approval_state_scope: getString(section, 'approval_state_scope', 'approvals'),
  };
}
