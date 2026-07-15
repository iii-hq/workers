import overview from '../../README.md?raw'
import agentQuality from '../../agent-quality.md?raw'
import conformance from '../../conformance-e2e.md?raw'

export const SPEC_DOCS = [
  { id: 'overview', label: 'Overview', content: overview },
  { id: 'conformance', label: 'Conformance E2E', content: conformance },
  { id: 'agent-quality', label: 'Agent-quality E2E', content: agentQuality },
] as const
