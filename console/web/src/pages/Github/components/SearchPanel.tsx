import { Search } from 'lucide-react'
import { useCallback, useState } from 'react'
import { Badge } from '@/components/ui/Badge'
import { Input } from '@/components/ui/Input'
import { ModeToggle } from '@/components/ui/ModeToggle'
import {
  type GithubSearchItems,
  type SearchKind,
  searchGithub,
  timeAgoIso,
} from '@/lib/github'
import { useGithubQuery } from '../hooks/useGithubQuery'
import { PanelShell } from './PanelShell'

const KIND_OPTIONS: { value: SearchKind; label: string }[] = [
  { value: 'repos', label: 'repos' },
  { value: 'issues', label: 'issues' },
  { value: 'prs', label: 'prs' },
  { value: 'code', label: 'code' },
]

interface SearchPanelProps {
  enabled: boolean
}

/**
 * Org-wide GitHub search. Repo scoping happens inside the query itself
 * (`repo:owner/name ...`), so this panel ignores the page's repo field.
 */
export function SearchPanel({ enabled }: SearchPanelProps) {
  const [kind, setKind] = useState<SearchKind>('repos')
  const [queryInput, setQueryInput] = useState('')
  const [query, setQuery] = useState('')

  const fetcher = useCallback(() => searchGithub(kind, query), [kind, query])
  const active = enabled && query !== ''
  const { data, loading, error } = useGithubQuery(active, fetcher)

  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center gap-3">
        <Input
          value={queryInput}
          onChange={setQueryInput}
          onKeyDown={(e) => {
            if (e.key === 'Enter') setQuery(queryInput.trim())
          }}
          placeholder='search query, e.g. "repo:iii-hq/workers is:open"'
          preserveCase
          aria-label="search query"
          className="w-80 max-w-full"
        />
        <ModeToggle<SearchKind>
          value={kind}
          onChange={setKind}
          options={KIND_OPTIONS}
        />
      </div>
      {query === '' ? (
        <p className="font-mono text-[12px] lowercase text-ink-faint">
          type a query and press enter — github search syntax, qualifiers
          included
        </p>
      ) : (
        <PanelShell
          loading={loading}
          error={error}
          empty={isEmpty(data)}
          emptyIcon={Search}
          emptyTitle="no results"
          emptyDescription={`nothing matched "${query}" in ${kind}`}
        >
          {data ? <SearchResults results={data} /> : null}
        </PanelShell>
      )}
    </div>
  )
}

function isEmpty(results: GithubSearchItems | null): boolean {
  return !results || results.items.length === 0
}

function SearchResults({ results }: { results: GithubSearchItems }) {
  if (results.kind === 'repos') {
    return (
      <ul className="flex flex-col">
        {results.items.map((repo) => (
          <li
            key={repo.fullName}
            className="flex items-center gap-3 py-2 border-b border-rule last:border-b-0"
          >
            <a
              href={repo.url}
              target="_blank"
              rel="noreferrer"
              className="font-mono text-[13px] text-ink hover:text-accent transition-colors shrink-0"
            >
              {repo.fullName}
            </a>
            <span className="flex-1 min-w-0 truncate font-mono text-[12px] text-ink-faint">
              {repo.description ?? ''}
            </span>
            <span className="hidden sm:block font-mono text-[11px] lowercase text-ink-faint shrink-0">
              {[
                repo.language ?? undefined,
                repo.stargazersCount != null
                  ? `${repo.stargazersCount}★`
                  : undefined,
                timeAgoIso(repo.updatedAt) || undefined,
              ]
                .filter(Boolean)
                .join(' · ')}
            </span>
          </li>
        ))}
      </ul>
    )
  }
  if (results.kind === 'code') {
    return (
      <ul className="flex flex-col">
        {results.items.map((hit) => (
          <li
            key={`${hit.repository?.nameWithOwner ?? ''}/${hit.path}`}
            className="flex items-center gap-3 py-2 border-b border-rule last:border-b-0"
          >
            {hit.url ? (
              <a
                href={hit.url}
                target="_blank"
                rel="noreferrer"
                className="flex-1 min-w-0 truncate font-mono text-[12px] text-ink hover:text-accent transition-colors"
              >
                {hit.path}
              </a>
            ) : (
              <span className="flex-1 min-w-0 truncate font-mono text-[12px] text-ink">
                {hit.path}
              </span>
            )}
            <span className="hidden sm:block font-mono text-[11px] lowercase text-ink-faint truncate max-w-64 shrink-0">
              {hit.repository?.nameWithOwner ?? ''}
            </span>
          </li>
        ))}
      </ul>
    )
  }
  return (
    <ul className="flex flex-col">
      {results.items.map((item) => (
        <li
          key={`${item.repository?.nameWithOwner ?? ''}#${item.number}`}
          className="flex items-center gap-3 py-2 border-b border-rule last:border-b-0"
        >
          <span className="font-mono text-[11px] text-ink-faint shrink-0">
            #{item.number}
          </span>
          <a
            href={item.url}
            target="_blank"
            rel="noreferrer"
            className="flex-1 min-w-0 truncate font-mono text-[13px] text-ink hover:text-accent transition-colors"
          >
            {item.title}
          </a>
          <span className="hidden sm:block font-mono text-[11px] lowercase text-ink-faint truncate max-w-64 shrink-0">
            {[
              item.repository?.nameWithOwner ?? undefined,
              item.author?.login ?? undefined,
              timeAgoIso(item.updatedAt) || undefined,
            ]
              .filter(Boolean)
              .join(' · ')}
          </span>
          <Badge
            variant={item.state.toLowerCase() === 'open' ? 'accent' : 'default'}
          >
            {item.state.toLowerCase()}
          </Badge>
        </li>
      ))}
    </ul>
  )
}
