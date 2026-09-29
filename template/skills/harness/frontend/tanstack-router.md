---
title: tanstack-router
description: Type-safe client-side routing with TanStack Router — file-based routes, beforeLoad and loader, URL search-param validation, pending/error states, code splitting, and TanStack Query integration.
type: how-to
---

# TanStack Router

The premise: **routes are data, not decorators.** A generated route tree makes `to`, `params`,
`search`, `loaderData`, and `context` type-checked end to end, so a renamed param or a
missing search field is a compile error rather than a runtime surprise.

Docs <https://tanstack.com/router/latest/docs/framework/react/overview>. Version-sensitive —
read the installed version's docs before using a feature you remember from a blog post.

## Setup that is not optional

```ts
// vite.config.ts
import { tanstackRouter } from '@tanstack/router-plugin/vite'

export default defineConfig({
  plugins: [tanstackRouter({ target: 'react', autoCodeSplitting: true }), react()],
})
```

- The plugin (or the CLI) generates `src/routeTree.gen.ts`. **Commit it, never hand-edit it**,
  and never import routes it does not know about. A stale generated tree is the single most
  common TanStack Router bug: types look wrong, links do not resolve, routes render nothing.
  When in doubt, restart the dev server so the plugin regenerates.
- `routeTree.gen.ts` belongs in `.gitignore` only if every consumer regenerates on install.
  Otherwise commit it.
- Register the router type once, at module scope:

```ts
// src/router.tsx
export const router = createRouter({
  routeTree,
  context: { queryClient },
  defaultPreload: 'intent',
  defaultPreloadStaleTime: 0,
  defaultPendingMs: 1000,
  defaultPendingMinMs: 500,
  scrollRestoration: true,
})

declare module '@tanstack/react-router' {
  interface Register { router: typeof router }
}
```

That `Register` block is what makes `Link`, `navigate`, and the hooks type-safe everywhere.
Omit it and everything silently degrades to `string`.

## File-based route conventions

| File | Means |
| --- | --- |
| `__root.tsx` | the root route — shell, providers, `notFoundComponent`, `errorComponent` |
| `index.tsx` | the index child of its directory |
| `about.tsx` | `/about` |
| `posts.$postId.tsx` | `/posts/$postId` — dots nest, exactly like directories |
| `posts/route.tsx` | layout route for the `/posts` segment (renders `<Outlet />`) |
| `posts/index.tsx` | `/posts/` |
| `_pathless.tsx` | layout route that adds no URL segment |
| `(group)/` | route group — organizes files, no URL segment |
| `$` | splat / catch-all |
| `posts.lazy.tsx` | lazy half of `posts.tsx` (when not using auto code splitting) |
| `-ignored.tsx` | ignored by the router |

Each route file exports one route:

```tsx
// src/routes/posts.$postId.tsx
export const Route = createFileRoute('/posts/$postId')({
  loader: ({ params }) => fetchPost(params.postId),
  component: PostPage,
})

function PostPage() {
  const { postId } = Route.useParams()
  const post = Route.useLoaderData()
  return <article><h1>{post.title}</h1></article>
}
```

The path string must match what the generated tree expects; let the plugin write it, or copy
the error the compiler gives you.

## `beforeLoad` vs `loader` vs `head`

- **`beforeLoad`** runs first, for every navigation, on every route in the match chain.
  Use it for authorization, redirects, and building `context` for the subtree. Returning a
  value is merged into `context` for that route and its children.

```tsx
export const Route = createFileRoute('/_authed')({
  beforeLoad: async ({ context, location }) => {
    const user = await context.auth.getUser()
    if (!user) throw redirect({ to: '/login', search: { redirect: location.href } })
    return { user } // typed into children as context.user
  },
})
```

  `redirect()` and `notFound()` are **thrown**, not returned. A `try/catch` that swallows them
  breaks navigation silently.
- **`loader`** fetches the data the route renders. It runs after `beforeLoad`, runs in
  parallel across the matched routes, and gets `{ params, deps, context, abortController,
  location, cause }`. Its return value is `useLoaderData()`.
- **`loaderDeps`** is what makes a loader react to the URL: without it a search-param change
  does not re-run the loader.

```tsx
export const Route = createFileRoute('/posts/')({
  validateSearch: (search) => ({ page: Number(search.page ?? 1) }),
  loaderDeps: ({ search }) => ({ page: search.page }),
  loader: ({ deps, context }) =>
    context.queryClient.ensureQueryData(postListOptions(deps.page)),
  component: PostList,
})
```

## Search params are the app's state machine

- Declare them with `validateSearch` and validate with the project's schema library:

```tsx
const searchSchema = z.object({
  q: z.string().catch(''),
  page: z.number().int().min(1).catch(1),
  tags: z.array(z.string()).default([]),
})

export const Route = createFileRoute('/posts/')({ validateSearch: searchSchema })
```

  Keep every field **primitive, serializable, and round-trippable** (`string`, `number`,
  `boolean`, arrays of those). A `Date` or an object identity in the URL is a lost reload.
- Read with `Route.useSearch()`; write with `navigate({ to: '/posts', search: (prev) => ({ ...prev, page: 2 }) })`
  or `<Link search={{ ... }}>`. The function form is what preserves the params you are not
  touching.
- `retainSearchParams` middleware carries params across navigations that would otherwise drop
  them — check the installed version's syntax before relying on it.
- **Default to the URL.** Tabs, filters, sort, pagination, and selection in component state do
  not survive back, reload, or a shared link.

## Links, active state, navigation

```tsx
<Link to="/posts/$postId" params={{ postId }} search={{ tab: 'comments' }} preload="intent"
      activeProps={{ className: 'is-active' }} activeOptions={{ exact: true }} />
```

- `to` + `params` + `search` are all type-checked; keep `to` literal-looking by using the
  generated union or `Link`'s own inference, and avoid `to={someString}`.
- `useNavigate()` for imperative navigation, `router.invalidate()` after a mutation that
  changes loader data, `useMatchRoute()` for conditional UI.
- `defaultPreload: 'intent'` (hover/focus) is nearly always the right call — it makes
  navigation feel instant. `'viewport'` preloads everything visible (mobile data cost),
  `'render'` preloads on mount.

## The three UI states you must not skip

```tsx
createRouter({
  routeTree,
  defaultPendingMs: 1000,   // reveal the pending UI only after this long
  defaultPendingMinMs: 500, // once revealed, keep it at least this long (no flash)
})
```

- `pendingComponent` per route for skeleton/loading UI. Data-less routes have nothing to wait
  for and should not show one.
- `errorComponent` on the root plus on any route whose failure should be contained. It
  receives `{ error, reset }` — render the reset affordance. Route errors do not reach a
  React error boundary above the router unless you rethrow.
- `notFoundComponent` on the root for unmatched URLs; call `notFound({ data })` from a loader
  when the record genuinely does not exist, and let the component render the 404 UI.

## Code splitting

- `autoCodeSplitting: true` in the plugin is the low-effort path: components get split out of
  the route file automatically.
- Otherwise split explicitly with `createLazyFileRoute` in a `.lazy.tsx` sibling. The loader
  and `beforeLoad` stay in the eager file (they must run for navigation) while the component
  moves to the lazy one.
- Lazy-load devtools too: `lazy(() => import('@tanstack/react-router-devtools'))` — they are
  large and must not ship.

## Pairing with TanStack Query (do not duplicate the cache)

The router owns *navigation*; the query cache owns *server data*. Wire them once:

```tsx
const queryClient = new QueryClient()
const router = createRouter({
  routeTree,
  context: { queryClient },
  defaultPreloadStaleTime: 0, // let Query decide staleness, not the router
})
```

- **Loaders seed, components read.** In the loader: `context.queryClient.ensureQueryData(options)`.
  In the component: `useSuspenseQuery(options)` with the *same* options object, from a shared
  `queryOptions()` factory. If the keys differ, you fetch twice.
- Because `defaultPreloadStaleTime: 0`, preloading a route with a warm query is free.
- After a mutation, either let Query handle it (`invalidateQueries`) or call
  `router.invalidate()` when non-Query loader data changed. Doing both is usually redundant but
  harmless; doing neither is the classic stale screen.
- With `ensureQueryData` the loader never throws on missing data — the component's
  `useSuspenseQuery` does, inside the route's error boundary.

## Trap list

- A hand-edited or stale `routeTree.gen.ts`; the plugin is not running.
- `redirect` / `notFound` returned instead of thrown.
- A loader that reads search params without a matching `loaderDeps` — it fetches once and
  never updates.
- `validateSearch` returning a new object every call in a way that breaks memoization; the
  schema object (and any `queryOptions`) should be module-scope constants.
- Non-serializable search values.
- Calling `Route.useParams()` from a component that is not inside that route — use
  `getRouteApi('/posts/$postId')` or the route's own component scope.
- `defaultPreloadStaleTime` left at its default while also using Query: "stale" preloads
  trigger loader re-runs and double fetches.
- Assuming `notFoundComponent` catches a thrown error — errors and not-found are different
  paths.
