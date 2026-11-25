import { CURSOR_ICON_SVG } from '../../../../cursor/src/provider-icon'
import anthropic from '../../../../provider-anthropic/assets/icon.svg?raw'
import claude from '../../../../provider-claude-code/assets/icon.svg?raw'
import commandCode from '../../../../provider-command-code/assets/icon.svg?raw'
import deepseek from '../../../../provider-deepseek/assets/icon.svg?raw'
import elevenlabs from '../../../../provider-elevenlabs/assets/icon.svg?raw'
import gemini from '../../../../provider-gemini/assets/icon.svg?raw'
import copilot from '../../../../provider-github-copilot/assets/icon.svg?raw'
import kimi from '../../../../provider-kimi/assets/icon.svg?raw'
import llama from '../../../../provider-llamacpp/assets/icon.svg?raw'
import openai from '../../../../provider-openai/assets/icon.svg?raw'
import opencode from '../../../../provider-opencode-go/assets/icon.svg?raw'
import openrouter from '../../../../provider-openrouter/assets/icon.svg?raw'
import sarvam from '../../../../provider-sarvam/assets/icon.svg?raw'
import xai from '../../../../provider-xai/assets/icon.svg?raw'
import zai from '../../../../provider-zai/assets/icon.svg?raw'

/**
 * The marks the provider workers in this repo declare to the router
 * (`icon_svg`), bundled so a provider shows its real logo before it is
 * installed: onboarding's provider list and the picker's Add a provider page.
 * Keyed by provider id (the registry worker name without `provider-`).
 */
const MARKS: Record<string, string> = {
  anthropic,
  'claude-code': claude,
  openai,
  'openai-codex': openai,
  openrouter,
  deepseek,
  elevenlabs,
  gemini,
  xai,
  kimi,
  sarvam,
  zai,
  'github-copilot': copilot,
  'command-code': commandCode,
  llamacpp: llama,
  'opencode-go': opencode,
  cursor: CURSOR_ICON_SVG,
}

/** The bundled mark for a provider id; `undefined` when none ships here. */
export function bundledProviderMark(
  providerId: string | null | undefined,
): string | undefined {
  return providerId ? MARKS[providerId] : undefined
}
