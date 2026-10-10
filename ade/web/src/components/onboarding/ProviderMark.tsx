import { ProviderIcon } from '@/components/chat/ProviderIcon'
import { CURSOR_ICON_SVG } from '../../../../../cursor/src/provider-icon'
import anthropic from '../../../../../provider-anthropic/assets/icon.svg?raw'
import claude from '../../../../../provider-claude-code/assets/icon.svg?raw'
import commandCode from '../../../../../provider-command-code/assets/icon.svg?raw'
import deepseek from '../../../../../provider-deepseek/assets/icon.svg?raw'
import copilot from '../../../../../provider-github-copilot/assets/icon.svg?raw'
import kimi from '../../../../../provider-kimi/assets/icon.svg?raw'
import llama from '../../../../../provider-llamacpp/assets/icon.svg?raw'
import openai from '../../../../../provider-openai/assets/icon.svg?raw'
import opencode from '../../../../../provider-opencode-go/assets/icon.svg?raw'
import openrouter from '../../../../../provider-openrouter/assets/icon.svg?raw'
import xai from '../../../../../provider-xai/assets/icon.svg?raw'
import zai from '../../../../../provider-zai/assets/icon.svg?raw'

// Use the same marks the workers declare to the router, even before install.
// CSS masks keep them crisp, monochrome, and legible in either theme.
const MARKS: Record<string, string> = {
  anthropic,
  'claude-code': claude,
  openai,
  'openai-codex': openai,
  openrouter,
  deepseek,
  xai,
  kimi,
  zai,
  'github-copilot': copilot,
  'command-code': commandCode,
  llamacpp: llama,
  'opencode-go': opencode,
  cursor: CURSOR_ICON_SVG,
}

export function ProviderMark({
  id,
  label,
  className,
}: {
  id: string
  label: string
  className?: string
}) {
  return (
    <ProviderIcon iconSvg={MARKS[id]} label={label} className={className} />
  )
}
