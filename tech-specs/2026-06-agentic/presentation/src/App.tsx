import { Footer } from '@/components/Footer'
import { TopNav } from '@/components/TopNav'
import { Sheet } from '@/components/schematic/Sheet'
import { useHashRoute } from '@/hooks/useHashRoute'
import { ConsolePage } from '@/pages/ConsolePage'
import { LoopsPage } from '@/pages/LoopsPage'
import { TelegramPage } from '@/pages/TelegramPage'
import { DurableSection } from '@/sections/DurableSection'
import { GovernanceSection } from '@/sections/GovernanceSection'
import { Hero } from '@/sections/Hero'
import { OnboardingSection } from '@/sections/OnboardingSection'
import { ReactiveSection } from '@/sections/ReactiveSection'
import { SolvesSection } from '@/sections/SolvesSection'
import { SpawnSection } from '@/sections/SpawnSection'
import { SubstrateSection } from '@/sections/SubstrateSection'
import { SystemMapSection } from '@/sections/SystemMapSection'
import { TurnSection } from '@/sections/TurnSection'
import { UseCasesSection } from '@/sections/UseCasesSection'

function Home() {
  return (
    <main>
      <Hero />
      <SystemMapSection />
      <TurnSection />
      <SubstrateSection />
      <ReactiveSection />
      <DurableSection />
      <SpawnSection />
      <GovernanceSection />
      <UseCasesSection />
      <OnboardingSection />
      <SolvesSection />
    </main>
  )
}

export default function App() {
  const route = useHashRoute()

  return (
    <div className="@container min-h-screen">
      <Sheet>
        <TopNav route={route} />
        {route.kind === 'home' ? (
          <Home />
        ) : route.id === 'telegram' ? (
          <TelegramPage />
        ) : route.id === 'console' ? (
          <ConsolePage />
        ) : (
          <LoopsPage />
        )}
        <Footer />
      </Sheet>
    </div>
  )
}
