import { describe, expect, it } from 'vitest';
import { normalizeChatCompletionsUrl } from '../../src/runtime/openai-compat-url.js';

describe('normalizeChatCompletionsUrl', () => {
  // The regression this test exists for: a user who saved
  // `http://192.168.1.206:8080` from the Providers UI got 404 on every
  // chat request because the override URL was POSTed verbatim. The fix
  // routes the override through this helper.
  it('appends /v1/chat/completions to a bare base URL', () => {
    expect(normalizeChatCompletionsUrl('http://192.168.1.206:8080')).toBe(
      'http://192.168.1.206:8080/v1/chat/completions',
    );
  });

  it('appends /v1/chat/completions to a base URL with a trailing slash', () => {
    expect(normalizeChatCompletionsUrl('http://localhost:8080/')).toBe(
      'http://localhost:8080/v1/chat/completions',
    );
  });

  // Regression: substring matching on the raw input previously concatenated
  // the path into the query string, producing `?foo=bar/v1/chat/completions`
  // and 404s on every request. Routing through the parsed URL fixes the
  // path placement and preserves the query.
  it('places /v1/chat/completions on the path even when the input has only a query string', () => {
    expect(normalizeChatCompletionsUrl('http://localhost:8080?foo=bar')).toBe(
      'http://localhost:8080/v1/chat/completions?foo=bar',
    );
  });

  it('leaves a fully-qualified URL unchanged', () => {
    expect(
      normalizeChatCompletionsUrl('http://localhost:8080/v1/chat/completions'),
    ).toBe('http://localhost:8080/v1/chat/completions');
  });

  it('treats any path containing /chat/completions as already qualified', () => {
    // Some forks of llama-server / LM Studio expose the endpoint at a
    // non-/v1 path. Don't double-append.
    expect(normalizeChatCompletionsUrl('http://host:8080/api/v2/chat/completions')).toBe(
      'http://host:8080/api/v2/chat/completions',
    );
  });

  it('accepts https URLs', () => {
    expect(normalizeChatCompletionsUrl('https://tunnel.ngrok.io')).toBe(
      'https://tunnel.ngrok.io/v1/chat/completions',
    );
  });

  it('returns null for empty / whitespace strings', () => {
    expect(normalizeChatCompletionsUrl('')).toBeNull();
    expect(normalizeChatCompletionsUrl('   ')).toBeNull();
  });

  it('returns null for non-http(s) schemes', () => {
    expect(normalizeChatCompletionsUrl('file:///etc/passwd')).toBeNull();
    expect(normalizeChatCompletionsUrl('javascript:alert(1)')).toBeNull();
  });

  it('returns null for unparseable URLs', () => {
    expect(normalizeChatCompletionsUrl('not a url')).toBeNull();
    expect(normalizeChatCompletionsUrl('http://')).toBeNull();
  });
});
