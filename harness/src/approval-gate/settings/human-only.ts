const HUMAN_ONLY_FUNCTION_IDS = [
  'approval::set_mode',
  'approval::add_always_allow',
  'approval::remove_always_allow',
  'approval::approve_always',
  'approval::get_settings',
  'approval::clear_settings',
] as const;

export type HumanOnlyFunctionId = (typeof HUMAN_ONLY_FUNCTION_IDS)[number];

export function isHumanOnlyApprovalFunction(function_id: string): boolean {
  return (HUMAN_ONLY_FUNCTION_IDS as readonly string[]).includes(function_id);
}
