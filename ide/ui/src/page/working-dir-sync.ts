export function workingDirectoryNeedsFollow(workingDir: string | null, acknowledgedWorkingDir: string | null): boolean {
  return workingDir !== null && workingDir !== acknowledgedWorkingDir
}

/** Ignore successful validations that settle after Harness has selected a
    different directory. */
export function acknowledgeValidatedWorkingDirectory(
  acknowledgedWorkingDir: string | null,
  requestedWorkingDir: string,
  currentWorkingDir: string | null,
  validated: boolean,
): string | null {
  return validated && requestedWorkingDir === currentWorkingDir ? requestedWorkingDir : acknowledgedWorkingDir
}

/** Stop following an unavailable chat root only when it is still current. */
export function acknowledgeUnavailableWorkingDirectory(
  acknowledgedWorkingDir: string | null,
  requestedWorkingDir: string,
  currentWorkingDir: string | null,
): string | null {
  return requestedWorkingDir === currentWorkingDir ? requestedWorkingDir : acknowledgedWorkingDir
}

export type RootTargetValidation =
  | { outcome: 'validated'; path: string }
  | { outcome: 'failed'; error: unknown }
  | { outcome: 'superseded' }

/** Validate before the caller tears down any root-owned UI. The sequence
    predicate also prevents a late result from committing stale navigation. */
export async function validateRootTarget(
  validate: () => Promise<{ path: string }>,
  isCurrent: () => boolean,
): Promise<RootTargetValidation> {
  try {
    const { path } = await validate()
    return isCurrent() ? { outcome: 'validated', path } : { outcome: 'superseded' }
  } catch (error) {
    return isCurrent() ? { outcome: 'failed', error } : { outcome: 'superseded' }
  }
}

const ROOT_VALIDATION_RETRY_DELAYS = [250, 500, 1_000, 2_000, 4_000] as const
const TRANSIENT_WORKING_DIR_RETRY_MS = 5_000

/** Delay after the given consecutive failure, or null once retries are
    exhausted. A later directory selection starts a new bounded sequence. */
export function rootValidationRetryDelay(failures: number): number | null {
  return ROOT_VALIDATION_RETRY_DELAYS[failures] ?? null
}

export function isUnavailableWorkingDirectoryError(error: unknown): boolean {
  const raw =
    error instanceof Error
      ? error.message
      : error && typeof error === 'object' && 'message' in error
        ? String((error as { message: unknown }).message)
        : String(error)
  const directCode =
    error && typeof error === 'object' && 'code' in error ? String((error as { code: unknown }).code) : undefined
  const nestedCodes = [...raw.matchAll(/"code"\s*:\s*"([A-Z]\d{3})"/g)]
  const code = nestedCodes.at(-1)?.[1] ?? directCode
  return code === 'S211' || code === 'S212' || /not found or not accessible|not a directory/i.test(raw)
}

export function workingDirectoryFollowRetryDelay(failures: number, error: unknown): number | null {
  const initialDelay = rootValidationRetryDelay(failures)
  if (initialDelay !== null) return initialDelay
  return isUnavailableWorkingDirectoryError(error) ? null : TRANSIENT_WORKING_DIR_RETRY_MS
}

export function rebasePathAfterValidation(absolutePath: string, requestedRoot: string, validatedRoot: string): string {
  if (absolutePath === requestedRoot) return validatedRoot
  const prefix = requestedRoot.endsWith('/') ? requestedRoot : `${requestedRoot}/`
  if (!absolutePath.startsWith(prefix)) return absolutePath
  const base = validatedRoot.endsWith('/') ? validatedRoot : `${validatedRoot}/`
  return `${base}${absolutePath.slice(prefix.length)}`
}

export function deepLinkRootTarget(absolutePath: string, workingDir: string | null): string {
  if (workingDir !== null) {
    const prefix = workingDir.endsWith('/') ? workingDir : `${workingDir}/`
    if (absolutePath === workingDir || absolutePath.startsWith(prefix)) {
      return workingDir
    }
  }
  const cut = absolutePath.lastIndexOf('/')
  return cut > 0 ? absolutePath.slice(0, cut) : '/'
}

export function ownsRequestToken(activeRequest: number | null, request: number): boolean {
  return activeRequest === request
}

export interface ScopedRequestToken {
  scope: number
  request: number
}

export function ownsScopedRequestToken(
  currentScope: number,
  activeRequest: ScopedRequestToken | null,
  request: ScopedRequestToken,
): boolean {
  return (
    currentScope === request.scope &&
    activeRequest?.scope === request.scope &&
    activeRequest.request === request.request
  )
}

export function workingDirectoryRetryMessage(
  path: string,
  outcome: 'failed' | 'declined',
  _retryDelay: number | null,
): string | null {
  if (outcome === 'declined') return `working directory change paused for ${path}`
  return null
}
