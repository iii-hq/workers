//! Per-provider identity prompt bodies (engine-grounded capability ladder).

pub const ANTHROPIC: &str = include_str!("../../prompts/anthropic.txt");
/// Code-mode identity: replaces the mesh identity entirely — native tool
/// exposure has no `agent_trigger` to document. The GPT/codex variant
/// swaps the edit discipline to the apply_patch format that family is
/// trained on.
pub const CODE: &str = include_str!("../../prompts/code.txt");
pub const CODE_GPT: &str = include_str!("../../prompts/code-gpt.txt");
pub const GPT: &str = include_str!("../../prompts/gpt.txt");
pub const KIMI: &str = include_str!("../../prompts/kimi.txt");
pub const DEFAULT: &str = include_str!("../../prompts/default.txt");
