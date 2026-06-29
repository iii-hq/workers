import type { ComponentType } from 'react'
import { Footer } from '@/components/Footer'
import { TopNav } from '@/components/TopNav'
import { Sheet } from '@/components/schematic/Sheet'
import { useHashRoute } from '@/hooks/useHashRoute'
import { HarnessConsumerPage } from '@/pages/HarnessConsumerPage'
import { Hero } from '@/sections/Hero'
import { WhySection } from '@/sections/WhySection'
import { RunItSection } from '@/sections/RunItSection'
import { SystemMapSection } from '@/sections/SystemMapSection'
import { SourceOfTruthSection } from '@/sections/SourceOfTruthSection'
import { SelectSection } from '@/sections/SelectSection'
import { LanguagesSection } from '@/sections/LanguagesSection'
import { HarnessSection } from '@/sections/HarnessSection'
import { PayoffSection } from '@/sections/PayoffSection'

/**
 * Ordered home-page sections. The first is the hero; the rest each carry a DOM
 * id matching a NAV entry in content/deck.ts for scroll-spy.
 */
const SECTIONS: ComponentType[] = [
  Hero,
  WhySection,
  RunItSection,
  SystemMapSection,
  SourceOfTruthSection,
  SelectSection,
  LanguagesSection,
  HarnessSection,
  PayoffSection,
]

/** deep-dive pages, keyed by the `#/<slug>` route slug. */
const PAGES: Record<string, ComponentType> = {
  'harness-consumer': HarnessConsumerPage,
}

function Home() {
  return (
    <main>
      {SECTIONS.map((Component, i) => (
        <Component key={i} />
      ))}
    </main>
  )
}

function NotFound() {
  return (
    <main className="px-4 py-24 @3xl:px-9">
      <p className="font-mono text-[14px] lowercase text-ink-faint">
        nothing here.{' '}
        <a href="#/" className="text-ink hover:text-accent transition-colors">
          ← back to the overview
        </a>
      </p>
    </main>
  )
}

export default function App() {
  const route = useHashRoute()
  const Page = route.kind === 'page' ? PAGES[route.slug] : undefined

  return (
    <div className="@container min-h-screen">
      <Sheet>
        <TopNav route={route} />
        {route.kind === 'home' ? <Home /> : Page ? <Page /> : <NotFound />}
        <Footer />
      </Sheet>
    </div>
  )
}
