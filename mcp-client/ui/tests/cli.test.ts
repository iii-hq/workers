import { expect, it } from 'vitest'
import { asCliCommand } from '../src/cli'

it('writes the call as the Functions page does', () => {
  expect(asCliCommand('mcp::linear::list_issues', { team: 'ENG', assignee: 'me', limit: 5 })).toBe(
    'iii call mcp::linear::list_issues team=ENG assignee=me limit=5',
  )
  expect(asCliCommand('mcp::docs::resources::read', { uri: 'file:///README.md' })).toBe(
    'iii call mcp::docs::resources::read uri=file:///README.md',
  )
  expect(asCliCommand('mcp::docs::prompts::code_review', { language: 'rust' })).toBe(
    'iii call mcp::docs::prompts::code_review language=rust',
  )
  expect(asCliCommand('mcp::linear::whoami', {})).toBe('iii call mcp::linear::whoami')
  expect(asCliCommand('mcp::linear::save_issue', { title: 'Fix the bug', labels: ['Bug'] })).toBe(
    `iii call mcp::linear::save_issue title='Fix the bug' labels='["Bug"]'`,
  )
  expect(asCliCommand('mcp::linear::save_issue', { title: "it's" })).toBe(
    `iii call mcp::linear::save_issue title='it'\\''s'`,
  )
})
