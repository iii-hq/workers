/**
 * Whether a freshly started browser session should pull the browser page
 * into the workspace. Sessions pointed at the console itself are tooling
 * (tests, self-inspection) — surfacing those would flip the user's
 * workspace under them for nothing they asked to watch.
 */
export function shouldOpenBrowserSession(
  url: string | undefined,
  consoleOrigin: string,
): boolean {
  if (!url) return false
  try {
    return new URL(url).origin !== consoleOrigin
  } catch {
    return true
  }
}
