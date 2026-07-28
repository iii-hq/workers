import { Badge, type Host, Input } from '@iii-dev/console-ui'
import { useCallback, useState } from 'react'
import {
  type GithubSearchItems,
  type SearchKind,
  searchGithub,
  timeAgoIso,
} from './github-data'
import { type IconProps, Search } from './icons'
import { ModeToggle } from './ModeToggle'
import { PanelShell } from './PanelShell'
import { useGithubRead } from './useGithubRead'

const KIND_OPTIONS: { value: SearchKind; label: string }[] = [
  { value: 'repos', label: 'repos' },
  { value: 'issues', label: 'issues' },
  { value: 'prs', label: 'prs' },
  { value: 'code', label: 'code' },
]

const SearchIcon = (p: IconProps) => <Search size={28} {...p} />

interface SearchPanelProps {
  host: Host
  enabled: boolean
}

/**
 * Org-wide GitHub search. Repo scoping happens inside the query itself
 * (`repo:owner/name ...`), so this panel ignores the page's repo field.
 */
export function SearchPanel({ host, enabled }: SearchPanelProps) {
  const [kind, setKind] = useState<SearchKind>('repos')
  const [queryInput, setQueryInput] = useState('')
  const [query, setQuery] = useState('')

  const fetcher = useCallback(
    () => searchGithub(host, kind, query),
    [host, kind, query],
  )
  const active = enabled && query !== ''
  const { data, loading, error } = useGithubRead(active, fetcher)

  return (
    <div className="gh-panel">
      <div className="gh-search-bar">
        <Input
          value={queryInput}
          onChange={setQueryInput}
          onKeyDown={(e) => {
            if (e.key === 'Enter') setQuery(queryInput.trim())
          }}
          placeholder='search query, e.g. "repo:iii-hq/workers is:open"'
          preserveCase
          aria-label="search query"
          className="gh-search-input"
        />
        <ModeToggle<SearchKind>
          value={kind}
          onChange={setKind}
          options={KIND_OPTIONS}
          aria-label="search kind"
        />
      </div>
      {query === '' ? (
        <p className="gh-msg">
          type a query and press enter — github search syntax, qualifiers
          included
        </p>
      ) : (
        <PanelShell
          loading={loading}
          error={error}
          empty={isEmpty(data)}
          emptyIcon={SearchIcon}
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
      <ul className="gh-list">
        {results.items.map((repo) => (
          <li key={repo.fullName} className="gh-row">
            <a
              href={repo.url}
              target="_blank"
              rel="noreferrer"
              className="gh-row-tag"
            >
              {repo.fullName}
            </a>
            <span className="gh-row-title gh-row-title-quiet">
              {repo.description ?? ''}
            </span>
            <span className="gh-row-meta">
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
      <ul className="gh-list">
        {results.items.map((hit) => (
          <li
            key={`${hit.repository?.nameWithOwner ?? ''}/${hit.path}`}
            className="gh-row"
          >
            {hit.url ? (
              <a
                href={hit.url}
                target="_blank"
                rel="noreferrer"
                className="gh-row-title"
              >
                {hit.path}
              </a>
            ) : (
              <span className="gh-row-title">{hit.path}</span>
            )}
            <span className="gh-row-meta gh-trunc-64">
              {hit.repository?.nameWithOwner ?? ''}
            </span>
          </li>
        ))}
      </ul>
    )
  }
  return (
    <ul className="gh-list">
      {results.items.map((item) => (
        <li
          key={`${item.repository?.nameWithOwner ?? ''}#${item.number}`}
          className="gh-row"
        >
          <span className="gh-num">#{item.number}</span>
          <a
            href={item.url}
            target="_blank"
            rel="noreferrer"
            className="gh-row-title"
          >
            {item.title}
          </a>
          <span className="gh-row-meta gh-trunc-64">
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
