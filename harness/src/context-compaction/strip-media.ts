import type { AgentMessage } from '../types/agent-message.js';
import type { ContentBlock } from '../types/content.js';

export function stripMedia(
  messages: AgentMessage[],
  opts: { toolOutputMaxChars: number },
): AgentMessage[] {
  return messages.map((m) => {
    if (m.role === 'function_result') {
      return {
        ...m,
        content: m.content
          .map(stripImageBlock)
          .map((b) => truncateBlock(b, opts.toolOutputMaxChars)),
      };
    }
    return { ...m, content: m.content.map(stripImageBlock) } as AgentMessage;
  });
}

function stripImageBlock(b: ContentBlock): ContentBlock {
  if ((b as { type?: string }).type === 'image') {
    return { type: 'text', text: '[image stripped]' } as ContentBlock;
  }
  return b;
}

function truncateBlock(b: ContentBlock, max: number): ContentBlock {
  if ((b as { type?: string }).type !== 'text') return b;
  const text = (b as { type: 'text'; text: string }).text;
  if (text.length <= max) return b;
  return { type: 'text', text: `${text.slice(0, max)}... [truncated]` } as ContentBlock;
}
