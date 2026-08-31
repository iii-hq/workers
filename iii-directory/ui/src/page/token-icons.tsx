/**
 * The nine harness `SubagentIcon` glyphs, inlined from lucide
 * (ISC license) so the agent form shows EXACTLY what the console's
 * session tree renders for each token — the console maps
 * agent→Bot, code→Code2, search→Search, terminal→Terminal,
 * database→Database, test→FlaskConical, review→ClipboardCheck,
 * docs→FileText, design→Palette (see console/web
 * ActiveSubagentChips.tsx). Path data only; rendered as stroke icons
 * with the standard lucide attributes.
 */

type IconNode = [string, Record<string, string | number>][]

export const TOKEN_ICON_NODES: Record<string, IconNode> = {
  agent: [["path", {"d": "M12 8V4H8", "key": "hb8ula"}], ["rect", {"width": "16", "height": "12", "x": "4", "y": "8", "rx": "2", "key": "enze0r"}], ["path", {"d": "M2 14h2", "key": "vft8re"}], ["path", {"d": "M20 14h2", "key": "4cs60a"}], ["path", {"d": "M15 13v2", "key": "1xurst"}], ["path", {"d": "M9 13v2", "key": "rq6x2g"}]],
  code: [["path", {"d": "m18 16 4-4-4-4", "key": "1inbqp"}], ["path", {"d": "m6 8-4 4 4 4", "key": "15zrgr"}], ["path", {"d": "m14.5 4-5 16", "key": "e7oirm"}]],
  search: [["path", {"d": "m21 21-4.34-4.34", "key": "14j7rj"}], ["circle", {"cx": "11", "cy": "11", "r": "8", "key": "4ej97u"}]],
  terminal: [["path", {"d": "M12 19h8", "key": "baeox8"}], ["path", {"d": "m4 17 6-6-6-6", "key": "1yngyt"}]],
  database: [["ellipse", {"cx": "12", "cy": "5", "rx": "9", "ry": "3", "key": "msslwz"}], ["path", {"d": "M3 5V19A9 3 0 0 0 21 19V5", "key": "1wlel7"}], ["path", {"d": "M3 12A9 3 0 0 0 21 12", "key": "mv7ke4"}]],
  test: [["path", {"d": "M14 2v6a2 2 0 0 0 .245.96l5.51 10.08A2 2 0 0 1 18 22H6a2 2 0 0 1-1.755-2.96l5.51-10.08A2 2 0 0 0 10 8V2", "key": "18mbvz"}], ["path", {"d": "M6.453 15h11.094", "key": "3shlmq"}], ["path", {"d": "M8.5 2h7", "key": "csnxdl"}]],
  review: [["rect", {"width": "8", "height": "4", "x": "8", "y": "2", "rx": "1", "ry": "1", "key": "tgr4d6"}], ["path", {"d": "M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2", "key": "116196"}], ["path", {"d": "m9 14 2 2 4-4", "key": "df797q"}]],
  docs: [["path", {"d": "M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z", "key": "1oefj6"}], ["path", {"d": "M14 2v5a1 1 0 0 0 1 1h5", "key": "wfsgrz"}], ["path", {"d": "M10 9H8", "key": "b1mrlr"}], ["path", {"d": "M16 13H8", "key": "t4e002"}], ["path", {"d": "M16 17H8", "key": "z1uh3a"}]],
  design: [["path", {"d": "M12 22a1 1 0 0 1 0-20 10 9 0 0 1 10 9 5 5 0 0 1-5 5h-2.25a1.75 1.75 0 0 0-1.4 2.8l.3.4a1.75 1.75 0 0 1-1.4 2.8z", "key": "e79jfc"}], ["circle", {"cx": "13.5", "cy": "6.5", "r": ".5", "fill": "currentColor", "key": "1okk4w"}], ["circle", {"cx": "17.5", "cy": "10.5", "r": ".5", "fill": "currentColor", "key": "f64h9f"}], ["circle", {"cx": "6.5", "cy": "12.5", "r": ".5", "fill": "currentColor", "key": "qy21gx"}], ["circle", {"cx": "8.5", "cy": "7.5", "r": ".5", "fill": "currentColor", "key": "fotxhn"}]],
}

export function TokenIcon({
  token,
  size = 16,
  className,
}: {
  token: string
  size?: number
  className?: string
}) {
  const node = TOKEN_ICON_NODES[token]
  if (!node) return null
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
      className={className}
    >
      {node.map(([tag, attrs], i) => {
        const Tag = tag as 'path'
        // biome-ignore lint/suspicious/noArrayIndexKey: static path list
        return <Tag key={i} {...attrs} />
      })}
    </svg>
  )
}
