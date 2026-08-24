import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { SandboxErrorView } from './ErrorView'
import type { SandboxErrorDisplay } from './parsers'

const wireError: SandboxErrorDisplay = {
  variant: 'wire',
  error: {
    type: 'sandbox_error',
    code: 'S200',
    message: 'Command timed out after 100ms.',
    docs_url: 'https://docs.example/s200',
    retryable: true,
    fix_note: 'Increase timeout_ms or simplify the command.',
    fix: {
      stdout: 'partial output\n',
      stderr: '',
      exit_code: null,
      timed_out: true,
      duration_ms: 100,
      success: false,
    },
  },
}

describe('SandboxErrorView', () => {
  it('renders wire errors through the shared card recipe', () => {
    const html = renderToStaticMarkup(<SandboxErrorView display={wireError} />)

    expect(html).toContain('data-error-card=""')
    expect(html).toContain('class="iii-ui-card @container"')
    expect(html).toContain('role="alert"')
    expect(html).toContain('S200')
    expect(html).toContain('data-error-retryable="true"')
    expect(html).toContain('Suggested fix')
    expect(html).toContain('Partial output')
    expect(html).toContain('partial output')
    expect(html).toContain('Open documentation')
  })

  it('keeps secondary invocation diagnostics behind technical details', () => {
    const display: SandboxErrorDisplay = {
      variant: 'invocation',
      error: {
        title: 'Gate unavailable',
        message: 'The approval gate could not be reached.',
        functionId: 'sandbox::fs::write',
        deniedBy: 'gate_unavailable',
        detailText: 'connection refused on approval::resolve',
      },
    }
    const html = renderToStaticMarkup(<SandboxErrorView display={display} />)

    expect(html).toContain('Gate unavailable')
    expect(html).toContain('sandbox::fs::write')
    expect(html).toContain('gate_unavailable')
    expect(html).toContain('<details')
    expect(html).toContain('Technical details')
    expect(html).toContain('connection refused on approval::resolve')
  })

  it('turns dispatch denials into actionable guidance', () => {
    const display: SandboxErrorDisplay = {
      variant: 'dispatch-denied',
      error: {
        functionId: 'web::fetch',
        namespace: 'web',
        message:
          'function web::fetch is not permitted by this agent’s dispatch policy',
      },
    }
    const html = renderToStaticMarkup(<SandboxErrorView display={display} />)

    expect(html).toContain('Function blocked by policy')
    expect(html).toContain('Blocked function')
    expect(html).toContain('web::fetch')
    expect(html).toContain('How to resolve')
    expect(html).toContain('agent.functions')
    expect(html).toContain('options.functions.allow')
  })
})
