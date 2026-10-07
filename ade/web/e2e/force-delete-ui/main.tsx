import { createRoot } from 'react-dom/client'
import { ChatPanel } from '../../src/components/chat/ChatPanel'
import { TooltipProvider } from '../../src/components/ui/Tooltip'
import './fixture.css'

const query = new URLSearchParams(location.search)
document.documentElement.dataset.theme =
  query.get('theme') === 'dark' ? 'dark' : 'light'
const root = document.getElementById('root')
if (!root) throw new Error('fixture root missing')
createRoot(root).render(
  <TooltipProvider>
    <div
      style={{
        display: 'flex',
        width: Number(query.get('pane') ?? 1280),
        maxWidth: '100vw',
        height: '100dvh',
      }}
    >
      <ChatPanel />
    </div>
  </TooltipProvider>,
)
