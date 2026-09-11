import { describe, expect, it } from 'vitest'
import {
  acknowledgeUnavailableWorkingDirectory,
  acknowledgeValidatedWorkingDirectory,
  deepLinkRootTarget,
  isUnavailableWorkingDirectoryError,
  ownsRequestToken,
  ownsScopedRequestToken,
  rebasePathAfterValidation,
  rootValidationRetryDelay,
  validateRootTarget,
  workingDirectoryFollowRetryDelay,
  workingDirectoryNeedsFollow,
  workingDirectoryRetryMessage,
} from '../working-dir-sync'

describe('working directory synchronization', () => {
  it('retries after root resolves to the primary fallback without acknowledgment', () => {
    const workingDir = '/work/chat'
    let acknowledged: string | null = null

    // While root is null there is nothing to re-root yet. Once boot falls
    // back to the primary root, the unacknowledged chat directory is retried.
    const rootBeforeValidation: string | null = null
    expect(rootBeforeValidation).toBeNull()
    const rootAfterFailure = '/work/primary'
    expect(rootAfterFailure).not.toBe(workingDir)
    expect(workingDirectoryNeedsFollow(workingDir, acknowledged)).toBe(true)

    acknowledged = acknowledgeValidatedWorkingDirectory(acknowledged, workingDir, workingDir, true)
    expect(workingDirectoryNeedsFollow(workingDir, acknowledged)).toBe(false)
  })

  it('keeps a manual root sticky after acknowledging the unchanged chat directory', () => {
    const workingDir = '/work/chat'
    const acknowledged = acknowledgeValidatedWorkingDirectory(null, workingDir, workingDir, true)

    // The actual Shell root may now differ because the user selected it.
    expect('/work/manual').not.toBe(workingDir)
    expect(workingDirectoryNeedsFollow(workingDir, acknowledged)).toBe(false)
  })

  it('does not acknowledge a stale async validation result', () => {
    const acknowledged = acknowledgeValidatedWorkingDirectory('/work/old', '/work/first', '/work/second', true)

    expect(acknowledged).toBe('/work/old')
    expect(workingDirectoryNeedsFollow('/work/second', acknowledged)).toBe(true)
  })

  it('quietly stops following an unavailable chat directory', () => {
    const unavailable = acknowledgeUnavailableWorkingDirectory(
      '/work/old',
      '/private/tmp/deleted',
      '/private/tmp/deleted',
    )
    expect(unavailable).toBe('/private/tmp/deleted')
    expect(workingDirectoryNeedsFollow('/private/tmp/deleted', unavailable)).toBe(false)

    expect(acknowledgeUnavailableWorkingDirectory('/work/old', '/private/tmp/deleted', '/work/new')).toBe('/work/old')
  })

  it('does not permit destructive transition work before validation resolves', async () => {
    let resolveValidation: ((value: { path: string }) => void) | undefined
    const validation = new Promise<{ path: string }>((resolve) => {
      resolveValidation = resolve
    })
    let resetCount = 0
    const pending = validateRootTarget(
      () => validation,
      () => true,
    ).then((result) => {
      if (result.outcome === 'validated') resetCount += 1
      return result
    })

    await Promise.resolve()
    expect(resetCount).toBe(0)
    resolveValidation?.({ path: '/private/tmp/app' })

    await expect(pending).resolves.toEqual({
      outcome: 'validated',
      path: '/private/tmp/app',
    })
    expect(resetCount).toBe(1)
  })

  it('leaves transition state untouched when async validation fails', async () => {
    let resetCount = 0
    const result = await validateRootTarget(
      () => Promise.reject(new Error('temporary worker outage')),
      () => true,
    )
    if (result.outcome === 'validated') resetCount += 1

    expect(result.outcome).toBe('failed')
    expect(resetCount).toBe(0)
  })

  it('supersedes an in-flight chat result when manual navigation starts', async () => {
    let sequence = 1
    let resolveChat: ((value: { path: string }) => void) | undefined
    const chatValidation = new Promise<{ path: string }>((resolve) => {
      resolveChat = resolve
    })
    const pendingChat = validateRootTarget(
      () => chatValidation,
      () => sequence === 1,
    )

    // Manual navigation synchronously claims the next sequence.
    sequence = 2
    const manual = validateRootTarget(
      () => Promise.resolve({ path: '/work/manual' }),
      () => sequence === 2,
    )
    resolveChat?.({ path: '/work/chat' })

    await expect(pendingChat).resolves.toEqual({ outcome: 'superseded' })
    await expect(manual).resolves.toEqual({
      outcome: 'validated',
      path: '/work/manual',
    })
  })

  it('recovers a valid temp target within a bounded retry sequence', async () => {
    let attempts = 0
    let result = await validateRootTarget(
      async () => {
        attempts += 1
        throw new Error('worker not registered yet')
      },
      () => true,
    )

    let failures = 0
    while (result.outcome === 'failed') {
      expect(rootValidationRetryDelay(failures)).not.toBeNull()
      failures += 1
      result = await validateRootTarget(
        async () => {
          attempts += 1
          if (attempts < 3) throw new Error('worker not registered yet')
          return { path: '/private/tmp/harness-app' }
        },
        () => true,
      )
    }

    expect(result).toEqual({
      outcome: 'validated',
      path: '/private/tmp/harness-app',
    })
    expect(attempts).toBe(3)
    expect(rootValidationRetryDelay(5)).toBeNull()
  })

  it('rebases a pending temp file through the validated canonical root', () => {
    expect(
      rebasePathAfterValidation('/tmp/harness-app/src/app.ts', '/tmp/harness-app/src', '/private/tmp/harness-app/src'),
    ).toBe('/private/tmp/harness-app/src/app.ts')
  })

  it('roots a nested deep link at the chat working directory during recovery', () => {
    expect(deepLinkRootTarget('/tmp/harness-app', '/tmp/harness-app')).toBe('/tmp/harness-app')

    expect(deepLinkRootTarget('/tmp/harness-app/src/components/App.tsx', '/tmp/harness-app')).toBe('/tmp/harness-app')

    expect(
      rebasePathAfterValidation(
        '/tmp/harness-app/src/components/App.tsx',
        '/tmp/harness-app',
        '/private/tmp/harness-app',
      ),
    ).toBe('/private/tmp/harness-app/src/components/App.tsx')
  })

  it('uses the file parent when the deep link is outside the chat directory', () => {
    expect(deepLinkRootTarget('/work/another/src/App.tsx', '/work/app')).toBe('/work/another/src')
    expect(deepLinkRootTarget('/work/application/App.tsx', '/work/app')).toBe('/work/application')
  })

  it('exhausts and explicitly re-arms a bounded retry budget', () => {
    let failures = 0
    const delays: number[] = []
    for (;;) {
      const delay = rootValidationRetryDelay(failures)
      failures += 1
      if (delay === null) break
      delays.push(delay)
    }

    expect(delays).toEqual([250, 500, 1_000, 2_000, 4_000])
    expect(rootValidationRetryDelay(failures)).toBeNull()

    failures = 0
    expect(rootValidationRetryDelay(failures)).toBe(250)
  })

  it('acknowledges only definite missing paths after the initial retry burst', () => {
    expect(
      isUnavailableWorkingDirectoryError({
        message: 'handler error: {"code":"S211","message":"not found or not accessible"}',
      }),
    ).toBe(true)
    expect(isUnavailableWorkingDirectoryError(new Error('worker offline'))).toBe(false)
    expect(
      workingDirectoryFollowRetryDelay(5, {
        code: 'S212',
        message: 'not a directory',
      }),
    ).toBeNull()
    expect(workingDirectoryFollowRetryDelay(5, new Error('temporary worker outage'))).toBe(5_000)
  })

  it('keeps overlapping manual transitions owned by the newest request', () => {
    const manualOne = 1
    const manualTwo = 2
    let activeManual: number | null = manualTwo

    if (ownsRequestToken(activeManual, manualOne)) activeManual = null
    expect(activeManual).toBe(manualTwo)

    if (ownsRequestToken(activeManual, manualTwo)) activeManual = null
    expect(activeManual).toBeNull()
  })

  it('prevents an old deep-link callback from owning a newer capture', () => {
    const oldRequest = { scope: 1, request: 1 }
    const currentRequest = { scope: 2, request: 2 }

    expect(ownsScopedRequestToken(2, currentRequest, oldRequest)).toBe(false)
    expect(ownsScopedRequestToken(2, currentRequest, currentRequest)).toBe(true)
  })

  it('keeps missing chat roots quiet but surfaces a declined change', () => {
    expect(workingDirectoryRetryMessage('/private/tmp/app', 'failed', 250)).toBeNull()
    expect(workingDirectoryRetryMessage('/private/tmp/app', 'failed', null)).toBeNull()
    expect(workingDirectoryRetryMessage('/private/tmp/app', 'declined', null)).toBe(
      'working directory change paused for /private/tmp/app',
    )
  })
})
