---
title: tanstack-query
description: Server state with TanStack Query — query keys and options factories, stale vs fresh, mutations and optimistic updates, invalidation, suspense, pagination, and error handling.
type: how-to
---

# TanStack Query

Treat the cache as **the only copy of server state**. Components read from it; nothing mirrors
it into `useState`. Everything else in this skill follows from that.

Docs <https://tanstack.com/query/latest/docs/framework/react/overview>.

## Setup and defaults that matter

```tsx
const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 30_000,
      gcTime: 5 * 60_000,
      retry: (count, error) => !isClientError(error) && count < 2,
      refetchOnWindowFocus: true,
    },
    mutations: { retry: 0 },
  },
})
```

- **`staleTime` is the single most important knob.** It is how long data is *trusted*. `0`
  (the default) means every mount, focus, and remount refetches — a request storm. Set it
  deliberately per resource: seconds for feeds, minutes for reference data, `Infinity` for
  data that only changes through a mutation you control.
- `gcTime` (formerly `cacheTime`) is how long an unused cache entry survives; it is unrelated
  to freshness. Do not use it to control refetching.
- Queries retry with exponential backoff by default; **mutations do not retry**, and should
  not. Never retry a 4xx — that is a bug in the request, not a blip.
- `refetchOnWindowFocus` is valuable for dashboards and infuriating for a form-heavy editor.
  Decide per app, and override per query.

## Keys and options: the discipline

```ts
// src/queries/posts.ts
export const postKeys = {
  all: ['posts'] as const,
  lists: () => [...postKeys.all, 'list'] as const,
  list: (filters: PostFilters) => [...postKeys.lists(), filters] as const,
  details: () => [...postKeys.all, 'detail'] as const,
  detail: (id: string) => [...postKeys.details(), id] as const,
}

export const postListOptions = (filters: PostFilters) =>
  queryOptions({ queryKey: postKeys.list(filters), queryFn: () => fetchPosts(filters) })
```

- Keys are **arrays, serializable, and hierarchical** — ordered from general to specific. That
  hierarchy is what lets `invalidateQueries({ queryKey: postKeys.lists() })` hit every list
  without touching a detail.
- **Every input the `queryFn` reads must be in the key.** A missing filter is the classic
  "it shows the wrong user's data" bug, and no amount of `refetch` fixes it.
- Export `queryOptions()` factories, not bare keys. One factory then serves `useQuery`,
  `useSuspenseQuery`, `prefetchQuery`, `ensureQueryData`, and the router loader — with the
  same key and the same function, which is the only way they stay in sync.

## Reading

```tsx
const { data, isPending, isFetching, error, refetch } = useQuery(postListOptions(filters))
```

- Branch on `isPending` (no data yet) separately from `isFetching` (data shown, refreshing).
  Rendering a spinner over data you already have is a UX regression.
- `useSuspenseQuery` for routes covered by a `Suspense` boundary: it suspends instead of
  returning a pending state, and `data` is non-nullable. Prefer it where the router already has
  a pending/error UI system.
- `enabled: Boolean(id)` to defer a query whose inputs are not ready; dependent queries chain
  naturally because a disabled query is simply not fetched.
- `useQueries` for a dynamic set of parallel queries; `useInfiniteQuery` with
  `initialPageParam` and `getNextPageParam` for cursors, plus `maxPages` to bound memory.
- `placeholderData: keepPreviousData` to keep the previous page visible while the next loads —
  not `initialData`, which pollutes the cache and suppresses `isPending`.
- `select` to narrow or transform cached data; the transform is memoized per observer, so it is
  safe for sorting and filtering derived views.

## Writing

```tsx
const mutation = useMutation({
  mutationFn: updatePost,
  onMutate: async (next) => {
    await queryClient.cancelQueries({ queryKey: postKeys.detail(next.id) })
    const previous = queryClient.getQueryData(postKeys.detail(next.id))
    queryClient.setQueryData(postKeys.detail(next.id), (old) => ({ ...old, ...next }))
    return { previous } // becomes the context argument
  },
  onError: (_err, next, ctx) => {
    if (ctx) queryClient.setQueryData(postKeys.detail(next.id), ctx.previous)
  },
  onSettled: (_data, _err, next) => {
    void queryClient.invalidateQueries({ queryKey: postKeys.detail(next.id) })
  },
})
```

- **Optimistic update = cancel, snapshot, write, roll back on error, reconcile on settle.**
  All five steps. Skipping `cancelQueries` means an in-flight refetch overwrites the optimistic
  value and then the rollback restores the wrong thing.
- `invalidateQueries` marks entries stale and refetches the *active* ones. Use it when the
  server is the source of truth for the shape of the change.
- `setQueryData` when the mutation response already contains the new value — no round trip.
  Together with an update to the *list* entry, or the list will disagree with the detail.
- A mutation that affects many queries is better expressed as a small set of targeted
  invalidations than one broad `invalidateQueries()`; broad invalidation is correct but slow.
- `useMutation` returned `mutateAsync` rejects; `mutate` does not. Use `mutateAsync` inside a
  handler with `try/catch`, `mutate` for fire-and-forget with callbacks.

## Errors

- Per-query `throwOnError: true` (or `throwOnError: (error) => isServerError(error)`) routes
  failures to the nearest error boundary — which pairs cleanly with the router's
  `errorComponent`. Otherwise handle `error` inline.
- A global `QueryCache` `onError` (and `MutationCache` `onError`) is the right place for
  logging, a toast, or auth-expiry redirects, so no call site repeats it.
- `retry` should reflect the error: retry network and 5xx, never 4xx validation failures.

## Integration points

- **Router (or any loader):** `await queryClient.ensureQueryData(options)` in the loader, then
  `useSuspenseQuery(options)` in the component. Same factory, one fetch.
- **Prefetch:** `queryClient.prefetchQuery(options)` on hover/intent makes navigation
  instant and costs one request.
- **Hydration/SSR:** `dehydrate` / `hydrate` on the client; the same keys must exist on both
  sides or the cache thrashes.
- **Offline:** `persistQueryClient` + the broadcast/sync plugins, with a `maxAge` and a
  buster that changes when the cache shape changes.
- **Devtools:** `@tanstack/react-query-devtools`, mounted behind a dev-only import.

## Testing

```tsx
const wrapper = ({ children }: { children: ReactNode }) => (
  <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
    {children}
  </QueryClientProvider>
)
```

- **A fresh `QueryClient` per test**, with `retry: false` and `gcTime: 0`. A shared client leaks
  cache between tests and makes them order-dependent.
- Drive real handlers through MSW rather than mocking the query hooks — testing the hook mock
  tests the mock.

## Anti-patterns to refuse

- `useEffect` + `useState` for fetching when a query would do.
- Copying query data into component state (it drifts immediately).
- Keys that omit inputs, or a single `['data']` key for the whole app.
- `staleTime: 0` on every list, then wondering why the network panel is busy.
- `invalidateQueries()` with no key after every mutation.
- Optimistic UI without a rollback path.
- Using `initialData` where `placeholderData` is meant.
