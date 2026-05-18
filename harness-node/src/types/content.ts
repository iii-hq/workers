/**
 * Content blocks shared by AgentMessage / FunctionResult.
 * Mirrors `harness/crates/harness-types/src/content.rs`.
 */

export type TextContent = {
  type: 'text';
  text: string;
};

export type ImageContent = {
  type: 'image';
  mime: string;
  data: string;
};

export type FunctionCallContent = {
  type: 'function_call';
  id: string;
  function_id: string;
  arguments: unknown;
};

export type FunctionResultContent = {
  type: 'function_result';
  function_call_id: string;
  content: ContentBlock[];
  is_error?: boolean;
};

export type ThinkingContent = {
  type: 'thinking';
  text: string;
  signature?: string;
};

export type ContentBlock =
  | TextContent
  | ImageContent
  | FunctionCallContent
  | FunctionResultContent
  | ThinkingContent;

export const text = (s: string): TextContent => ({ type: 'text', text: s });
