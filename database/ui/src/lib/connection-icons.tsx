/**
 * The icons a connection can wear (`display.icon`, a Lucide name). A subset of
 * the iii app's environment glyphs, so a connection looks the same on the
 * phone and in the console. A name outside the set — written by another
 * client — still renders, as the database glyph, and is kept as stored.
 */

import {
  Atom,
  Bot,
  Box,
  Boxes,
  Bug,
  Building,
  Cloud,
  Code,
  Container,
  Database,
  Flag,
  Flame,
  FlaskConical,
  Gem,
  Globe,
  HardDrive,
  Heart,
  Hexagon,
  House,
  Layers,
  Leaf,
  Lock,
  type LucideIcon,
  Rocket,
  Server,
  Shield,
  Star,
  Target,
  Zap,
} from 'lucide-react'
import type { CSSProperties } from 'react'

const ICONS: Readonly<Record<string, LucideIcon>> = {
  database: Database,
  server: Server,
  cloud: Cloud,
  'hard-drive': HardDrive,
  container: Container,
  box: Box,
  boxes: Boxes,
  layers: Layers,
  rocket: Rocket,
  'flask-conical': FlaskConical,
  bug: Bug,
  code: Code,
  globe: Globe,
  house: House,
  building: Building,
  zap: Zap,
  flame: Flame,
  shield: Shield,
  lock: Lock,
  leaf: Leaf,
  atom: Atom,
  bot: Bot,
  star: Star,
  heart: Heart,
  gem: Gem,
  target: Target,
  flag: Flag,
  hexagon: Hexagon,
}

export const CONNECTION_ICONS: readonly string[] = Object.keys(ICONS)

export const DEFAULT_CONNECTION_ICON = 'database'

export function connectionIconComponent(name: string | undefined): LucideIcon {
  return (name && ICONS[name]) || Database
}

/** The connection's glyph, tinted with its colour when it has one. */
export function ConnectionIcon({ name, color, className }: { name?: string; color?: string; className?: string }) {
  const Icon = connectionIconComponent(name)
  const style: CSSProperties | undefined = color ? { color } : undefined
  return <Icon size={16} aria-hidden className={className} style={style} />
}
