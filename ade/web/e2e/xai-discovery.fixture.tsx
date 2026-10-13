import { useState } from 'react'
import { createRoot } from 'react-dom/client'
import { ModelPicker } from '../src/components/chat/ModelPicker'
import { TooltipProvider } from '../src/components/ui/Tooltip'
import type { DiscoveryStatus } from '../src/lib/models-catalog'
import { FixtureContext } from './xai-discovery-context'
import '../src/index.css'

const billing: DiscoveryStatus = {
  outcome: 'billing', http_status: 403, code: 'permission-denied', stale: false, checked_at_ms: 1,
}
function Fixture() {
  const [status, setStatus] = useState(billing)
  const [value, setValue] = useState('xai::grok-4')
  const [calls, setCalls] = useState<string[]>([])
  const [pending, setPending] = useState(false)
  const providers = [{ id: 'xai', display_name: 'xAI', configured: true, available: true, supports_model_listing: true, discovery: status }]
  async function refreshModels(id?: string) {
    setCalls((previous) => [...previous, id ?? 'all'])
    setPending(true)
    await new Promise((resolve) => setTimeout(resolve, 500))
    setStatus({ outcome: 'success', stale: false, checked_at_ms: 2 })
    setPending(false)
  }
  return (
    <FixtureContext.Provider value={{ presentProviders: providers, refreshModels, refreshingModels: pending }}>
      <TooltipProvider>
        <main className="p-4">
          <output aria-label="selection">{value}</output>
          <output aria-label="refresh calls">{calls.join(',')}</output>
          <ModelPicker value={value} options={[{ id: 'xai::grok-4', label: 'grok-4' }, { id: 'openai::gpt-5', label: 'gpt-5' }]} thinkingLevel="default" onChange={setValue} onThinkingLevelChange={() => {}} showRefresh={false} showProviderConfiguration={false} />
        </main>
      </TooltipProvider>
    </FixtureContext.Provider>
  )
}
const root = document.getElementById('root')
if (root) createRoot(root).render(<Fixture />)
