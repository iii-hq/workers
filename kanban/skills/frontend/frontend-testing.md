---
title: frontend-testing
description: Test frontend code at the right level — Vitest and Testing Library for behaviour, MSW for the network boundary, Playwright for critical journeys, and the mocking rules that keep tests honest.
type: how-to
---

# Frontend testing

A test exists to catch a regression you would otherwise ship. If it only restates the
implementation, it will fail whenever you refactor and pass whenever you break the app.

## Choose the level deliberately

| Level | Tool | Covers |
| --- | --- | --- |
| Types + lint | `tsc --noEmit`, eslint | contracts, dead code, a11y lint |
| Unit | Vitest | pure functions, formatters, reducers, query-key factories |
| Component | Vitest + Testing Library | rendering, user interaction, states, ARIA |
| Integration | RTL + MSW + a real router | one screen end to end against a fake network |
| Journey (e2e) | Playwright | login, checkout, the three flows the business depends on |

The common failure is **too many e2e tests and too few component tests**: e2e is slow, flaky,
and tells you "something broke", so it belongs on critical journeys only.

## Vitest setup

```ts
// vitest.config.ts
export default defineConfig({
  test: {
    environment: 'jsdom',
    setupFiles: ['./src/test/setup.ts'],
    restoreMocks: true,
    coverage: { provider: 'v8', reporter: ['text', 'lcov'] },
  },
})
```

- `environment: 'jsdom'` (or `happy-dom`, faster but less faithful for layout). Neither
  computes real layout: **anything that depends on measured sizes must be tested in a browser**
  (`vitest --browser` or Playwright).
- `setup.ts`: import `@testing-library/jest-dom`, start and stop the MSW server, and reset any
  global state.

## Testing Library: query the way a user finds things

The accessible name is the query, and that is the point: a missing label breaks the test, which
is a real bug.

```tsx
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'

it('filters the list by tag', async () => {
  const user = userEvent.setup()
  render(<PostList />)

  await user.type(screen.getByRole('searchbox', { name: /search/i }), 'vite')
  await user.click(screen.getByRole('button', { name: /apply/i }))

  expect(await screen.findByRole('heading', { name: 'Vite' })).toBeInTheDocument()
  expect(screen.queryByText('Loading')).not.toBeInTheDocument()
})
```

Rules:

- **Query priority:** role/name → label → text → test id. `getByTestId` is the last resort, for
  things with no accessible identity.
- `userEvent` over `fireEvent` — it models real input (focus, key sequences, pointer events)
  and will surface focus-management bugs that `fireEvent.click` hides.
- `await` every interaction and every `findBy*`. An unawaited `user.click` is the number one
  source of "works locally, flakes in CI".
- **Assert on what the user sees**, not on state, props, or class names.
- Test the states every screen has: loading, empty, error, success, and long content. Those are
  where the bug is.
- **No snapshots** for component output. They get blessed on failure and then assert nothing.
- Wrap in the real providers the component needs (QueryClient, router, iii client) via a local
  `renderWithProviders` — not by mocking the provider away.

## The network boundary: MSW, not module mocks

```ts
// src/test/handlers.ts
export const handlers = [
  http.get('/api/posts', () => HttpResponse.json([{ id: '1', title: 'Vite' }])),
]
```

- Mock the **protocol**, not the module you are testing. A test that mocks `usePosts` proves
  nothing about `usePosts` or about the screen that consumes it.
- Override per test with `server.use(...)`; the shared handlers stay the happy path.
- Same handlers can run in the browser during development, so the fake data is exercised twice.
- For an iii app, the boundary is the engine: wrap the `iii-browser-sdk` client in a thin
  adapter (`{ trigger, registerFunction }`) and inject a fake for tests. Mocking the SDK's
  internals makes your tests depend on a third party's module shape.

## Async, timers, and flake traps

- Never `await new Promise(r => setTimeout(r, 100))`. Use `findBy*`, `waitFor`, and
  `waitForElementToBeRemoved` — they poll until the assertion holds or the timeout expires.
- Fake timers and `userEvent` are a classic conflict: `userEvent.setup({ advanceTimers: vi.advanceTimersByTime })`,
  or avoid fake timers where real ones work.
- **A fresh `QueryClient` per test** with `retry: false` and `gcTime: 0`; a shared client leaks
  cache between tests and makes them order-dependent.
- React `act(...)` warnings are real: state updated outside `act` means an unawaited update.
  Fix the await, do not silence the warning.
- Clock- and timezone-dependent tests: pin them (`vi.setSystemTime`, `TZ=UTC` in CI).
- Stub randomness (`vi.spyOn(Math, 'random')`), never assert on a generated uuid.

## Playwright, for the flows that must not break

```ts
const test = base.extend({
  page: async ({ page }, use) => {
    await page.addInitScript(() => localStorage.setItem('seen-onboarding', '1'))
    await use(page)
  },
})
```

- `webServer` in `playwright.config.ts` so the suite starts the app itself; `reuseExistingServer`
  locally.
- `storageState` for a signed-in session instead of logging in through the UI in every test.
- Built-in auto-waiting `expect(locator).toBeVisible()` — no sleeps, ever.
- `trace: 'on-first-retry'`, `screenshot: 'only-on-failure'`: you will need them at 2 a.m.
- Run against the **production build** at least once in CI (`vite build && vite preview`) — dev
  servers hide production-only failures.
- One journey per spec, named after the user outcome.

## What not to test

- Third-party library behaviour.
- Exact CSS, class names, or DOM structure.
- Private functions you reached through a re-export.
- A component's internal state.
- The same logic at three levels; pick one level per rule and trust it.

## Definition of done for a UI change

1. `tsc --noEmit` and lint clean.
2. Unit/component tests cover the new states, including empty and error.
3. The primary flow passes in a real browser at a narrow and a wide viewport.
4. Keyboard-only pass of the new interaction.
5. The suite is green twice in a row (a flaky test is a red test).
