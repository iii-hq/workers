import { describe, expect, it } from 'vitest'
import type { OpEcho, UpdateOp } from '../parsers'
import {
  contentLineCount,
  echoRows,
  firstEchoLine,
  groupEchoesByOp,
} from '../UpdateFileView'

function lineEcho(overrides: Partial<OpEcho> = {}): OpEcho {
  return {
    op_index: 0,
    from_line: 1,
    lines: ['pub mod utils;'],
    ...overrides,
  }
}

describe('groupEchoesByOp', () => {
  it('returns no groups for a failed file (echoes always [])', () => {
    expect(groupEchoesByOp([])).toEqual([])
  })

  it('keeps one group per line op in wire order', () => {
    const groups = groupEchoesByOp([
      lineEcho({ op_index: 0, from_line: 1 }),
      lineEcho({ op_index: 1, from_line: 10 }),
    ])
    expect(groups.map((g) => g.opIndex)).toEqual([0, 1])
    expect(groups[0]?.echoes).toHaveLength(1)
  })

  it('groups replace sites sharing an op_index, sites in match order', () => {
    const groups = groupEchoesByOp([
      lineEcho({ op_index: 0, from_line: 1 }),
      lineEcho({ op_index: 2, from_line: 10, total_replacements: 7 }),
      lineEcho({ op_index: 2, from_line: 55, total_replacements: 7 }),
    ])
    expect(groups).toHaveLength(2)
    expect(groups[1]?.opIndex).toBe(2)
    expect(groups[1]?.echoes.map((e) => e.from_line)).toEqual([10, 55])
  })
})

describe('contentLineCount (parity with update_file.rs split_content)', () => {
  it('mirrors Rust str::lines() — trailing \\n adds no line', () => {
    expect(contentLineCount('')).toBe(0)
    expect(contentLineCount('a')).toBe(1)
    expect(contentLineCount('a\n')).toBe(1)
    expect(contentLineCount('a\nb')).toBe(2)
    expect(contentLineCount('a\nb\n')).toBe(2)
    expect(contentLineCount('\n')).toBe(1)
    expect(contentLineCount('a\n\n')).toBe(2)
  })

  it('CRLF \\r never affects the count', () => {
    expect(contentLineCount('a\r\nb\r\n')).toBe(2)
    expect(contentLineCount('\r\n')).toBe(1)
  })
})

describe('echoRows (no op — neutral rows, wire numbering)', () => {
  it('numbers lines sequentially from from_line when nothing is elided', () => {
    const rows = echoRows(lineEcho({ from_line: 10, lines: ['a', 'b', 'c'] }))
    expect(rows).toEqual([
      { kind: 'line', lineNo: 10, text: 'a', added: false },
      { kind: 'line', lineNo: 11, text: 'b', added: false },
      { kind: 'line', lineNo: 12, text: 'c', added: false },
    ])
  })

  it('places the replace-site divider between first and last region line', () => {
    // build_site_echo: region 10..=15 echoes first + last, elided = 4.
    const rows = echoRows(
      lineEcho({
        from_line: 10,
        lines: ['fn first() {', '}'],
        elided: 4,
        total_replacements: 7,
      }),
    )
    expect(rows).toEqual([
      { kind: 'line', lineNo: 10, text: 'fn first() {', added: false },
      { kind: 'elision', count: 4 },
      // Tail resumes after the 4 elided inner lines: region ends at L15.
      { kind: 'line', lineNo: 15, text: '}', added: false },
    ])
  })

  it('resumes line-op tail numbering after the elided middle (8+8 split)', () => {
    // build_line_echo: 26-line region keeps first/last ECHO_HEAD_TAIL (8).
    const lines = Array.from({ length: 16 }, (_, i) => `l${i}`)
    const rows = echoRows(lineEcho({ from_line: 100, lines, elided: 10 }))
    expect(rows).toHaveLength(17)
    expect(rows[7]).toEqual({
      kind: 'line',
      lineNo: 107,
      text: 'l7',
      added: false,
    })
    expect(rows[8]).toEqual({ kind: 'elision', count: 10 })
    expect(rows[9]).toEqual({
      kind: 'line',
      lineNo: 118,
      text: 'l8',
      added: false,
    })
    expect(rows[16]).toEqual({
      kind: 'line',
      lineNo: 125,
      text: 'l15',
      added: false,
    })
  })

  it('degrades an elided echo with no lines to just the divider', () => {
    const rows = echoRows(lineEcho({ from_line: 1, lines: [], elided: 3 }))
    expect(rows).toEqual([{ kind: 'elision', count: 3 }])
  })
})

describe('echoRows — diff tagging (half-diff)', () => {
  it('insert: tags [at_line, at_line+K-1] added with trailing-\\n content, no stub', () => {
    // K = 2 ("x\ny\n" — trailing newline must NOT count a third line, or
    // the trailing context line L12 would be mistagged as added).
    const op: UpdateOp = { op: 'insert', at_line: 10, content: 'x\ny\n' }
    // build_line_echo: region [10,11] ±2 context → L8..L13.
    const echo = lineEcho({
      from_line: 8,
      lines: ['c1', 'c2', 'x', 'y', 'c3', 'c4'],
    })
    expect(echoRows(echo, op)).toEqual([
      { kind: 'line', lineNo: 8, text: 'c1', added: false },
      { kind: 'line', lineNo: 9, text: 'c2', added: false },
      { kind: 'line', lineNo: 10, text: 'x', added: true },
      { kind: 'line', lineNo: 11, text: 'y', added: true },
      { kind: 'line', lineNo: 12, text: 'c3', added: false },
      { kind: 'line', lineNo: 13, text: 'c4', added: false },
    ])
  })

  it('update_lines: tags [from_line, from_line+K-1] and stubs at the seam', () => {
    // K = 3 from "a\nb\nc\n" — replaces original L5–L6 (2 lines → 3).
    const op: UpdateOp = {
      op: 'update_lines',
      from_line: 5,
      to_line: 6,
      content: 'a\nb\nc\n',
    }
    // Post-apply region [5,7] ±2 context → L3..L9.
    const echo = lineEcho({
      from_line: 3,
      lines: ['c1', 'c2', 'a', 'b', 'c', 'c3', 'c4'],
    })
    expect(echoRows(echo, op)).toEqual([
      { kind: 'line', lineNo: 3, text: 'c1', added: false },
      { kind: 'line', lineNo: 4, text: 'c2', added: false },
      // Stub sits at the seam: after leading context, before additions.
      { kind: 'stub', verb: 'replaced', label: 'replaced original L5–L6' },
      { kind: 'line', lineNo: 5, text: 'a', added: true },
      { kind: 'line', lineNo: 6, text: 'b', added: true },
      { kind: 'line', lineNo: 7, text: 'c', added: true },
      { kind: 'line', lineNo: 8, text: 'c3', added: false },
      { kind: 'line', lineNo: 9, text: 'c4', added: false },
    ])
  })

  it('collapses the stub range to a single line number when from==to', () => {
    const op: UpdateOp = {
      op: 'update_lines',
      from_line: 7,
      to_line: 7,
      content: 'z',
    }
    const echo = lineEcho({ from_line: 5, lines: ['c1', 'c2', 'z', 'c3'] })
    const stub = echoRows(echo, op).find((r) => r.kind === 'stub')
    expect(stub).toEqual({
      kind: 'stub',
      verb: 'replaced',
      label: 'replaced original L7',
    })
  })

  it('tags an addition range spanning the elided middle (head AND tail)', () => {
    // update_lines L100–L101 with 20 new lines → post-apply region
    // [100,119]; ±2 context → [98,121] = 24 lines > ECHO_MAX_LINES(20)
    // → first 8 (L98..105) + last 8 (L114..121), elided 8.
    const content = `${Array.from({ length: 20 }, (_, i) => `n${i}`).join('\n')}\n`
    const op: UpdateOp = {
      op: 'update_lines',
      from_line: 100,
      to_line: 101,
      content,
    }
    // Wire keeps head 8 (L98..L105 = c1,c2,n0..n5) + tail 8
    // (L114..L121 = n14..n19,c3,c4); L106..L113 (n6..n13) elided.
    const echo = lineEcho({
      from_line: 98,
      lines: [
        'c1',
        'c2',
        'n0',
        'n1',
        'n2',
        'n3',
        'n4',
        'n5',
        'n14',
        'n15',
        'n16',
        'n17',
        'n18',
        'n19',
        'c3',
        'c4',
      ],
      elided: 8,
    })
    const rows = echoRows(echo, op)
    expect(rows).toHaveLength(18)
    // Leading context, then the stub at the seam.
    expect(rows[0]).toEqual({
      kind: 'line',
      lineNo: 98,
      text: 'c1',
      added: false,
    })
    expect(rows[1]).toEqual({
      kind: 'line',
      lineNo: 99,
      text: 'c2',
      added: false,
    })
    expect(rows[2]).toEqual({
      kind: 'stub',
      verb: 'replaced',
      label: 'replaced original L100–L101',
    })
    // Head additions L100..L105 — every one tagged.
    expect(rows[3]).toEqual({
      kind: 'line',
      lineNo: 100,
      text: 'n0',
      added: true,
    })
    expect(rows[8]).toEqual({
      kind: 'line',
      lineNo: 105,
      text: 'n5',
      added: true,
    })
    expect(rows[9]).toEqual({ kind: 'elision', count: 8 })
    // Tail resumes INSIDE the addition range: L114..L119 still added.
    expect(rows[10]).toEqual({
      kind: 'line',
      lineNo: 114,
      text: 'n14',
      added: true,
    })
    expect(rows[15]).toEqual({
      kind: 'line',
      lineNo: 119,
      text: 'n19',
      added: true,
    })
    // Trailing context past the range end (119) stays neutral.
    expect(rows[16]).toEqual({
      kind: 'line',
      lineNo: 120,
      text: 'c3',
      added: false,
    })
    expect(rows[17]).toEqual({
      kind: 'line',
      lineNo: 121,
      text: 'c4',
      added: false,
    })
  })

  it('remove: every echoed line stays neutral, stub marks the seam', () => {
    const op: UpdateOp = { op: 'remove', from_line: 10, to_line: 12 }
    // Remove anchors at from_line-1 = L9; ±2 context → L7..L11.
    const echo = lineEcho({
      from_line: 7,
      lines: ['c1', 'c2', 'c3', 'c4', 'c5'],
    })
    expect(echoRows(echo, op)).toEqual([
      { kind: 'line', lineNo: 7, text: 'c1', added: false },
      { kind: 'line', lineNo: 8, text: 'c2', added: false },
      { kind: 'line', lineNo: 9, text: 'c3', added: false },
      // The removal seam: original L10–L12 are gone; final L10 is the
      // line that used to be L13.
      { kind: 'stub', verb: 'removed', label: 'removed original L10–L12' },
      { kind: 'line', lineNo: 10, text: 'c4', added: false },
      { kind: 'line', lineNo: 11, text: 'c5', added: false },
    ])
  })

  it('remove at EOF: stub appends when no echoed line is at/after from_line', () => {
    const op: UpdateOp = { op: 'remove', from_line: 40, to_line: 42 }
    // Tail removal: echo is only the surviving lines above the cut.
    const echo = lineEcho({ from_line: 37, lines: ['c1', 'c2', 'c3'] })
    const rows = echoRows(echo, op)
    expect(rows[3]).toEqual({
      kind: 'stub',
      verb: 'removed',
      label: 'removed original L40–L42',
    })
  })

  it('replace: every site line is added (zero-context echo), no stub', () => {
    const op: UpdateOp = {
      op: 'replace',
      pattern: 'foo',
      replacement: 'bar',
    }
    const echo = lineEcho({
      from_line: 40,
      lines: ['const a = bar', 'const b = bar'],
      total_replacements: 3,
    })
    expect(echoRows(echo, op)).toEqual([
      { kind: 'line', lineNo: 40, text: 'const a = bar', added: true },
      { kind: 'line', lineNo: 41, text: 'const b = bar', added: true },
    ])
  })

  it('replace: elided multi-line site tags first AND last line added', () => {
    const op: UpdateOp = {
      op: 'replace',
      pattern: 'fn first\\(.*?\\n\\}',
      replacement: 'fn first() {\n…\n}',
      dot_matches_newline: true,
    }
    const echo = lineEcho({
      from_line: 10,
      lines: ['fn first() {', '}'],
      elided: 4,
      total_replacements: 1,
    })
    expect(echoRows(echo, op)).toEqual([
      { kind: 'line', lineNo: 10, text: 'fn first() {', added: true },
      { kind: 'elision', count: 4 },
      { kind: 'line', lineNo: 15, text: '}', added: true },
    ])
  })

  it('insert with empty content tags nothing (K = 0, like Rust)', () => {
    const op: UpdateOp = { op: 'insert', at_line: 5, content: '' }
    const echo = lineEcho({ from_line: 3, lines: ['c1', 'c2', 'c3'] })
    expect(echoRows(echo, op).every((r) => r.kind === 'line' && !r.added)).toBe(
      true,
    )
  })
})

describe('echoRows — multi-op anchor reconstruction (post-apply coords)', () => {
  it('shifted update_lines: tags the post-apply line, not the original anchor (Rust echo_two_line_ops_offset_correctness)', () => {
    // File 1..10; op 0 inserts "X" at L2 (+1), op 1 updates original L5 →
    // "FIVE" now sits at POST-APPLY L6. Wire echo for op 1 (pinned from the
    // Rust test): from_line 4 = post lines 4..8. The anchor reconstructs as
    // from_line + ECHO_CONTEXT = 6; original-coords math would mistag the
    // context line "4" (post L5) and misplace the stub before it.
    const op: UpdateOp = {
      op: 'update_lines',
      from_line: 5,
      to_line: 5,
      content: 'FIVE',
    }
    const echo = lineEcho({
      op_index: 1,
      from_line: 4,
      lines: ['3', '4', 'FIVE', '6', '7'],
    })
    expect(echoRows(echo, op)).toEqual([
      { kind: 'line', lineNo: 4, text: '3', added: false },
      { kind: 'line', lineNo: 5, text: '4', added: false },
      { kind: 'stub', verb: 'replaced', label: 'replaced original L5' },
      { kind: 'line', lineNo: 6, text: 'FIVE', added: true },
      { kind: 'line', lineNo: 7, text: '6', added: false },
      { kind: 'line', lineNo: 8, text: '7', added: false },
    ])
  })

  it('head-clamped echo (from_line === 1) without fileOps falls back to original coords', () => {
    // Op 0 of the same Rust test: its own anchor is unshifted (the other
    // op sits strictly below), but build_line_echo's head clamp collapses
    // from_line to 1, so wire reconstruction is impossible. Without the
    // request's op list the original at_line is the only anchor — exact
    // here, and "X" (post L2) tags added.
    const op: UpdateOp = { op: 'insert', at_line: 2, content: 'X' }
    const echo = lineEcho({
      op_index: 0,
      from_line: 1,
      lines: ['1', 'X', '2', '3'],
    })
    expect(echoRows(echo, op)).toEqual([
      { kind: 'line', lineNo: 1, text: '1', added: false },
      { kind: 'line', lineNo: 2, text: 'X', added: true },
      { kind: 'line', lineNo: 3, text: '2', added: false },
      { kind: 'line', lineNo: 4, text: '3', added: false },
    ])
  })

  it('shifted remove: stub seam follows the post-apply anchor', () => {
    // File 1..10; insert "X" at L2 (+1), then remove original L8–L9.
    // Rust anchors remove at from_line−1 = 7, mapped +1 → post L8 ("7");
    // echo from_line 8−2 = 6, post lines 6..9 = 5,6,7,10. The seam is
    // post L9: stub lands between "7" and "10", not before post L8.
    const op: UpdateOp = { op: 'remove', from_line: 8, to_line: 9 }
    const echo = lineEcho({
      op_index: 1,
      from_line: 6,
      lines: ['5', '6', '7', '10'],
    })
    expect(echoRows(echo, op)).toEqual([
      { kind: 'line', lineNo: 6, text: '5', added: false },
      { kind: 'line', lineNo: 7, text: '6', added: false },
      { kind: 'line', lineNo: 8, text: '7', added: false },
      { kind: 'stub', verb: 'removed', label: 'removed original L8–L9' },
      { kind: 'line', lineNo: 9, text: '10', added: false },
    ])
  })

  it('head-clamp + line-op-only batch: maps the anchor through sibling deltas (request disambiguates the clamp)', () => {
    // Insert "Z" at L1 (+1) + update_lines original L2 → post L3 "TWO".
    // The echo head-clamps (from_line 1) so wire reconstruction is out,
    // but the batch is line-op-only and the view holds the full request:
    // post anchor = 2 + (+1 from the smaller-anchor insert) = 3 — "TWO"
    // tags added and the stub lands at the true seam, not before "1".
    const insertOp: UpdateOp = { op: 'insert', at_line: 1, content: 'Z' }
    const op: UpdateOp = {
      op: 'update_lines',
      from_line: 2,
      to_line: 2,
      content: 'TWO',
    }
    const echo = lineEcho({
      op_index: 1,
      from_line: 1,
      lines: ['Z', '1', 'TWO', '3', '4'],
    })
    expect(echoRows(echo, op, [insertOp, op])).toEqual([
      { kind: 'line', lineNo: 1, text: 'Z', added: false },
      { kind: 'line', lineNo: 2, text: '1', added: false },
      { kind: 'stub', verb: 'replaced', label: 'replaced original L2' },
      { kind: 'line', lineNo: 3, text: 'TWO', added: true },
      { kind: 'line', lineNo: 4, text: '3', added: false },
      { kind: 'line', lineNo: 5, text: '4', added: false },
    ])
  })

  it('head-clamp mapping ignores siblings at larger anchors (they apply first, strictly below)', () => {
    // record_line_op_events applies bottom-up: the remove (anchor 5) runs
    // BEFORE the insert (anchor 1), so it is not a "later event" and must
    // not shift the insert's region. File 1..6: insert "X" at L1 (+1),
    // remove original L5–L6 → final X,1,2,3,4. Echo region [1,1] +ctx.
    const op: UpdateOp = { op: 'insert', at_line: 1, content: 'X' }
    const below: UpdateOp = { op: 'remove', from_line: 5, to_line: 6 }
    const echo = lineEcho({
      op_index: 0,
      from_line: 1,
      lines: ['X', '1', '2'],
    })
    const added = echoRows(echo, op, [op, below])
      .filter((r) => r.kind === 'line' && r.added)
      .map((r) => (r.kind === 'line' ? r.text : ''))
    expect(added).toEqual(['X'])
  })

  it('head-clamp + remove: the stub seam maps through the sibling insert', () => {
    // File 1..6: insert "X" at L1 (+1), remove original L3–L4 → final
    // X,1,2,5,6. Remove anchors at from_line−1 = 2, mapped +1 → post L3
    // ("2"); echo from_line 3−2 = 1 head-clamps. The seam is post L4:
    // stub between "2" and "5" — original coords would misplace it
    // before "2".
    const insertOp: UpdateOp = { op: 'insert', at_line: 1, content: 'X' }
    const op: UpdateOp = { op: 'remove', from_line: 3, to_line: 4 }
    const echo = lineEcho({
      op_index: 1,
      from_line: 1,
      lines: ['X', '1', '2', '5', '6'],
    })
    expect(echoRows(echo, op, [insertOp, op])).toEqual([
      { kind: 'line', lineNo: 1, text: 'X', added: false },
      { kind: 'line', lineNo: 2, text: '1', added: false },
      { kind: 'line', lineNo: 3, text: '2', added: false },
      { kind: 'stub', verb: 'removed', label: 'removed original L3–L4' },
      { kind: 'line', lineNo: 4, text: '5', added: false },
      { kind: 'line', lineNo: 5, text: '6', added: false },
    ])
  })

  it('ACCEPTED RESIDUAL: a newline-adding replace above a head-of-file region defeats the mapping', () => {
    // File 1..5: replace "1" → "Z\n1" (+1, matches post-line-op L1) +
    // update_lines original L2 → post L3 "TWO". Regex ops run AFTER all
    // line ops and their match positions (thus deltas) exist only
    // server-side, so any replace in the batch forces the original-anchor
    // fallback: post L2 ("1", context) mistags instead of "TWO". Bounded
    // to the clamp window (first ECHO_CONTEXT+1 post-apply lines) —
    // pinned so the tradeoff stays visible; see postRegionFirst.
    const replaceOp: UpdateOp = {
      op: 'replace',
      pattern: '1',
      replacement: 'Z\n1',
    }
    const op: UpdateOp = {
      op: 'update_lines',
      from_line: 2,
      to_line: 2,
      content: 'TWO',
    }
    const echo = lineEcho({
      op_index: 1,
      from_line: 1,
      lines: ['Z', '1', 'TWO', '3', '4'],
    })
    const added = echoRows(echo, op, [replaceOp, op])
      .filter((r) => r.kind === 'line' && r.added)
      .map((r) => (r.kind === 'line' ? r.text : ''))
    expect(added).toEqual(['1']) // known off-by-shift; ideal would be ['TWO']
  })
})

describe('firstEchoLine (open-in-editor anchor)', () => {
  const base = {
    path: '/w/a.ts',
    success: true,
    applied: 1,
    new_line_count: 10,
    echoes_truncated: false,
  }

  it('returns the first echo from_line (wire order, first op group)', () => {
    expect(
      firstEchoLine({
        ...base,
        echoes: [
          lineEcho({ op_index: 0, from_line: 7 }),
          lineEcho({ op_index: 1, from_line: 30 }),
        ],
      }),
    ).toBe(7)
  })

  it('returns undefined when there are no echoes (failed or echo-less file)', () => {
    expect(firstEchoLine({ ...base, echoes: [] })).toBeUndefined()
  })
})
