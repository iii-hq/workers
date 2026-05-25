/**
 * Shared URL normalisation for OpenAI-compatible local-server providers
 * (lmstudio, llamacpp). Both providers accept a base URL (`host:port`)
 * or a full chat-completions URL; we always POST to
 * `/v1/chat/completions`, so a missing path here means the request
 * lands on the server root and llama-server / LM Studio return 404.
 *
 * `resolveApiUrl` in each provider's `config.ts` historically applied
 * this auto-append only to the `*_BASE_URL` env var path -- the
 * config.yaml `default_api_url` was assumed to already be fully
 * qualified, and the runtime override stored under
 * `provider_config::set` was used verbatim. That left a sharp edge:
 * users who saved a base URL (e.g. `http://192.168.1.206:8080`) from
 * the Providers UI got 404s on every request. This helper closes the
 * gap so the override path normalises the same way as the env-var
 * path.
 */

/** Return `raw` with `/v1/chat/completions` appended if missing, or `null` if `raw` is not a usable http(s) URL. */
export function normalizeChatCompletionsUrl(raw: string): string | null {
  const trimmed = raw.trim();
  if (trimmed.length === 0) return null;
  // Reject inputs that don't have a usable origin before we go appending
  // a path -- `new URL("http://")` parses but with hostname "", which
  // would then concat into the absurd `http://v1/chat/completions`.
  let base: URL;
  try {
    base = new URL(trimmed);
  } catch {
    return null;
  }
  if (base.protocol !== 'http:' && base.protocol !== 'https:') return null;
  if (base.hostname.length === 0) return null;

  // Work in URL space — string concatenation on the raw input mishandles
  // query-string-only inputs. `http://host?foo=bar` would otherwise become
  // `http://host?foo=bar/v1/chat/completions`, with the path segment buried
  // inside the query string and every request hitting the server root.
  const pathNoTrailing = base.pathname.replace(/\/+$/, '');
  if (pathNoTrailing.endsWith('/chat/completions')) {
    base.pathname = pathNoTrailing;
  } else {
    base.pathname = `${pathNoTrailing}/v1/chat/completions`;
  }
  return base.toString();
}
