import { describe, expect, it } from 'vitest';
import {
  ACCEPT_LANGUAGE,
  BROWSER_USER_AGENT,
  acceptHeaderFor,
  convertHtmlToMarkdown,
  extractTextFromHtml,
  isImageMime,
} from '../../src/web/convert.js';

describe('convertHtmlToMarkdown', () => {
  it('converts headings and paragraphs to atx markdown', () => {
    const md = convertHtmlToMarkdown('<h1>Hi</h1><p>body text</p>');
    expect(md).toBe('# Hi\n\nbody text');
  });

  it('uses fenced code blocks', () => {
    const md = convertHtmlToMarkdown('<pre><code>const a = 1;</code></pre>');
    expect(md).toContain('```\nconst a = 1;\n```');
  });

  it('uses "-" as the bullet list marker', () => {
    const md = convertHtmlToMarkdown('<ul><li>one</li><li>two</li></ul>');
    // Turndown pads the marker to a 4-char gutter: "-   item".
    expect(md).toBe('-   one\n-   two');
  });

  it('removes script, style, meta, and link elements entirely', () => {
    const md = convertHtmlToMarkdown(
      '<head><meta charset="utf-8"><link rel="x" href="y"><style>.a{color:red}</style></head>' +
        '<body><script>alert(1)</script><p>kept</p></body>',
    );
    expect(md).toBe('kept');
  });

  it('uses * for emphasis', () => {
    const md = convertHtmlToMarkdown('<p><em>soft</em></p>');
    expect(md).toBe('*soft*');
  });

  it('converts empty html to empty markdown', () => {
    expect(convertHtmlToMarkdown('')).toBe('');
  });
});

describe('extractTextFromHtml', () => {
  it('separates block-level elements with newlines and trims the result', () => {
    const text = extractTextFromHtml('  <div><p>one</p><p>two</p></div>  ');
    expect(text).toBe('one\ntwo');
  });

  it('treats <br> as a line break and collapses 3+ newlines', () => {
    const text = extractTextFromHtml('<p>a<br>b</p><div></div><div></div><div></div><p>c</p>');
    expect(text).toBe('a\nb\n\nc');
  });

  it('returns empty string for empty input', () => {
    expect(extractTextFromHtml('')).toBe('');
  });

  it('returns empty string when all content is in skipped subtrees', () => {
    expect(extractTextFromHtml('<script>only hidden</script>')).toBe('');
  });

  it('skips script/style/noscript/iframe contents', () => {
    const text = extractTextFromHtml(
      '<script>var x = "hidden";</script><style>.a{}</style>' +
        '<noscript>also hidden</noscript><iframe>nested hidden</iframe><p>visible</p>',
    );
    expect(text).toBe('visible');
  });

  it('skips nested elements inside a skipped subtree', () => {
    const text = extractTextFromHtml('<script><span>deep hidden</span></script><b>shown</b>');
    expect(text).toBe('shown');
  });
});

describe('acceptHeaderFor', () => {
  it('prefers markdown for format=markdown', () => {
    expect(acceptHeaderFor('markdown')).toBe(
      'text/markdown;q=1.0, text/x-markdown;q=0.9, text/plain;q=0.8, text/html;q=0.7, */*;q=0.1',
    );
  });

  it('prefers plain text for format=text', () => {
    expect(acceptHeaderFor('text')).toBe(
      'text/plain;q=1.0, text/markdown;q=0.9, text/html;q=0.8, */*;q=0.1',
    );
  });

  it('prefers html for format=html', () => {
    expect(acceptHeaderFor('html')).toBe(
      'text/html;q=1.0, application/xhtml+xml;q=0.9, text/plain;q=0.8, text/markdown;q=0.7, */*;q=0.1',
    );
  });
});

describe('isImageMime', () => {
  it('matches image mimes', () => {
    expect(isImageMime('image/png')).toBe(true);
    expect(isImageMime('image/svg+xml')).toBe(true);
  });

  it('rejects non-image mimes', () => {
    expect(isImageMime('text/html')).toBe(false);
    expect(isImageMime('application/json')).toBe(false);
    expect(isImageMime('')).toBe(false);
  });
});

describe('browser identity constants', () => {
  it('looks like a mainstream Chrome UA', () => {
    expect(BROWSER_USER_AGENT).toMatch(/Mozilla\/5\.0/);
    expect(BROWSER_USER_AGENT).toMatch(/Chrome\//);
  });

  it('accept-language is en-US first', () => {
    expect(ACCEPT_LANGUAGE).toBe('en-US,en;q=0.9');
  });
});
