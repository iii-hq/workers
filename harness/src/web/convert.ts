/**
 * Content transforms for `web::fetch` page-reading mode (`format` set).
 *
 * Pure functions only — no I/O. HTML→Markdown uses Turndown, plain-text
 * extraction uses a streaming htmlparser2 pass that skips non-content
 * subtrees. Both run AFTER the byte cap in fetch.ts, so input is always
 * bounded by `max_bytes`.
 *
 * The browser UA below is deliberately a mainstream Chrome string: many
 * sites (and Cloudflare's cheapest bot rule) gate on UA. When that rule
 * instead trips on the TLS-fingerprint/UA mismatch (403 +
 * `cf-mitigated: challenge`), fetch.ts retries once with the honest
 * configured UA — same strategy as opencode's webfetch tool.
 */

import { Parser } from 'htmlparser2';
import TurndownService from 'turndown';
import type { PageFormat } from './schemas.js';

export const BROWSER_USER_AGENT =
  'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36';

export const ACCEPT_LANGUAGE = 'en-US,en;q=0.9';

export function acceptHeaderFor(format: PageFormat): string {
  switch (format) {
    case 'markdown':
      return 'text/markdown;q=1.0, text/x-markdown;q=0.9, text/plain;q=0.8, text/html;q=0.7, */*;q=0.1';
    case 'text':
      return 'text/plain;q=1.0, text/markdown;q=0.9, text/html;q=0.8, */*;q=0.1';
    case 'html':
      return 'text/html;q=1.0, application/xhtml+xml;q=0.9, text/plain;q=0.8, text/markdown;q=0.7, */*;q=0.1';
  }
}

export function isImageMime(mime: string): boolean {
  return mime.startsWith('image/');
}

// The image media types the Anthropic Messages API accepts in tool_result
// image blocks. Anything else (image/svg+xml, image/avif, image/x-icon, …)
// is rejected by the provider with a 400 that fails the whole turn — those
// must NOT be emitted as image content blocks.
const VIEWABLE_IMAGE_MIMES = new Set(['image/jpeg', 'image/png', 'image/gif', 'image/webp']);

export function isViewableImageMime(mime: string): boolean {
  return VIEWABLE_IMAGE_MIMES.has(mime);
}

const SKIPPED_TEXT_TAGS = new Set(['script', 'style', 'noscript', 'iframe', 'object', 'embed']);

// Tags whose close marks a visual line break — without a separator,
// adjacent blocks collapse into unreadable runs ("Titlepara").
const BLOCK_TEXT_TAGS = new Set([
  'p',
  'div',
  'h1',
  'h2',
  'h3',
  'h4',
  'h5',
  'h6',
  'li',
  'tr',
  'section',
  'article',
  'header',
  'footer',
  'blockquote',
  'pre',
  'table',
  'ul',
  'ol',
]);

export function convertHtmlToMarkdown(html: string): string {
  const turndown = new TurndownService({
    headingStyle: 'atx',
    hr: '---',
    bulletListMarker: '-',
    codeBlockStyle: 'fenced',
    emDelimiter: '*',
  });
  turndown.remove(['script', 'style', 'meta', 'link']);
  return turndown.turndown(html);
}

export function extractTextFromHtml(html: string): string {
  let text = '';
  let skipDepth = 0;

  const parser = new Parser({
    onopentag(name) {
      if (skipDepth > 0 || SKIPPED_TEXT_TAGS.has(name)) skipDepth++;
      else if (name === 'br') text += '\n';
    },
    ontext(input) {
      if (skipDepth === 0) text += input;
    },
    onclosetag(name) {
      if (skipDepth > 0) skipDepth--;
      else if (BLOCK_TEXT_TAGS.has(name)) text += '\n';
    },
  });

  parser.write(html);
  parser.end();

  return text.replace(/\n{3,}/g, '\n\n').trim();
}
