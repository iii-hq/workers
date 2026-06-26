import type { ComponentType } from 'react'
import { Footer } from '@/components/Footer'
import { TopNav } from '@/components/TopNav'
import { Sheet } from '@/components/schematic/Sheet'
import { useHashRoute } from '@/hooks/useHashRoute'
import { EngineOverridesPage } from '@/pages/EngineOverridesPage'
import { RbacContractPage } from '@/pages/RbacContractPage'
import { AccessSection } from '@/sections/AccessSection'
import { CoexistenceSection } from '@/sections/CoexistenceSection'
import { FailClosedSection } from '@/sections/FailClosedSection'
import { Hero } from '@/sections/Hero'
import { LifecycleSection } from '@/sections/LifecycleSection'
import { MapSection } from '@/sections/MapSection'
import { OverridesSection } from '@/sections/OverridesSection'
import { PayoffSection } from '@/sections/PayoffSection'
import { WhySection } from '@/sections/WhySection'

/**
 * The ordered home-page sections. The first is the hero; the rest each carry a
 * DOM id matching a NAV entry in content/deck.ts for scroll-spy.
 */
const SECTIONS: ComponentType[] = [
  Hero,
  WhySection,
  LifecycleSection,
  MapSection,
  AccessSection,
  FailClosedSection,
  OverridesSection,
  CoexistenceSection,
  PayoffSection,
]

/** deep-dive pages, keyed by the `#/<slug>` route slug. */
const PAGES: Record<string, ComponentType> = {
  'engine-overrides': EngineOverridesPage,
  'rbac-contract': RbacContractPage,
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
