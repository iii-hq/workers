import { describe, expect, it } from 'vitest'
import {
  applyPatchRequestSchema,
  applyPatchResponseSchema,
  CODER_FUNCTION_IDS,
  CODER_MUTATE_FUNCTION_IDS,
  checkResultSchema,
  contentMatchSchema,
  contextRequestSchema,
  contextResponseSchema,
  createFileRequestSchema,
  createFileResponseSchema,
  deleteFileRequestSchema,
  deleteFileResponseSchema,
  formatUpdateOp,
  infoRequestSchema,
  infoResponseSchema,
  isCoderFunction,
  isCoderMutateFunction,
  joinEntryPath,
  listFolderRequestSchema,
  listFolderResponseSchema,
  moveFileRequestSchema,
  moveFileResponseSchema,
  readFileRequestSchema,
  readFileResponseSchema,
  safeParseRequest,
  safeParseResponse,
  searchRequestSchema,
  searchResponseSchema,
  treeRequestSchema,
  treeResponseSchema,
  truncateInline,
  unwrapEnvelope,
  updateFileRequestSchema,
  updateFileResponseSchema,
  updateOpReplaceSchema,
  wireErrorSchema,
  worktreeAddRequestSchema,
  worktreeAddResponseSchema,
  worktreeRemoveRequestSchema,
  worktreeRemoveResponseSchema,
} from '../parsers'

function wrap<T>(details: T) {
  return {
    content: [{ type: 'text', text: JSON.stringify(details) }],
    details,
    terminate: true,
  }
}

describe('isCoderFunction', () => {
  it('matches every id in the explicit allowlist', () => {
    for (const id of CODER_FUNCTION_IDS) {
      expect(isCoderFunction(id)).toBe(true)
    }
    expect(CODER_FUNCTION_IDS).toHaveLength(13)
  })

  it('rejects same-prefix non-members and other families', () => {
    expect(isCoderFunction('coder::nonexistent')).toBe(false)
    expect(isCoderFunction('sandbox::exec')).toBe(false)
  })
})

describe('isCoderMutateFunction', () => {
  it('matches every id in the explicit allowlist, including move', () => {
    for (const id of CODER_MUTATE_FUNCTION_IDS) {
      expect(isCoderMutateFunction(id)).toBe(true)
    }
    expect(isCoderMutateFunction('coder::move')).toBe(true)
    expect(isCoderMutateFunction('coder::apply-patch')).toBe(true)
  })

  it('rejects read-only coder ids and other families', () => {
    expect(isCoderMutateFunction('coder::read-file')).toBe(false)
    expect(isCoderMutateFunction('coder::search')).toBe(false)
    expect(isCoderMutateFunction('coder::context')).toBe(false)
    expect(isCoderMutateFunction('coder::worktree-add')).toBe(false)
    expect(isCoderMutateFunction('sandbox::exec')).toBe(false)
  })
})

describe('wireErrorSchema', () => {
  it('accepts a structured {code, message} error', () => {
    const r = wireErrorSchema.safeParse({
      code: 'C217',
      message: 'already exists — pass overwrite: true',
    })
    expect(r.success).toBe(true)
  })

  it('rejects entries missing the message', () => {
    expect(wireErrorSchema.safeParse({ code: 'C211' }).success).toBe(false)
  })
})

describe('infoRequestSchema / infoResponseSchema', () => {
  it('accepts an empty discovery request (null coerced to {})', () => {
    expect(safeParseRequest(infoRequestSchema, undefined)).toEqual({})
  })

  it('parses a wrapped full config payload', () => {
    const r = safeParseResponse(
      infoResponseSchema,
      wrap({
        base_paths: ['/work/project', '/tmp/coder-cache'],
        primary_root: '/work/project',
        batch_read_budget_bytes: 1048576,
        max_output_bytes: 131072,
        max_read_bytes: 2097152,
        max_write_bytes: 2097152,
        default_exclude_globs: ['node_modules/**', '.git/**'],
        non_accessible_globs: ['.env', 'secrets/**'],
        list_default_page_size: 100,
        list_max_page_size: 500,
        search_default_max_line_bytes: 512,
        search_default_max_matches: 100,
        search_response_budget_bytes: 65536,
        tree_default_depth: 3,
        tree_per_folder_limit: 50,
        version: '0.4.1',
      }),
    )
    expect(r?.primary_root).toBe('/work/project')
    expect(r?.base_paths[0]).toBe(r?.primary_root)
    expect(r?.non_accessible_globs).toContain('.env')
  })

  it('rejects a response missing a required budget field', () => {
    expect(
      safeParseResponse(infoResponseSchema, { version: '0.4.1' }),
    ).toBeNull()
  })
})

describe('createFileRequestSchema', () => {
  it('accepts the golden batched request', () => {
    const r = safeParseRequest(createFileRequestSchema, {
      files: [
        { content: 'pub mod utils;\n', overwrite: false, path: 'src/lib.rs' },
        {
          content: '# scratch notes\n',
          overwrite: true,
          path: '/tmp/scratch/notes.md',
        },
      ],
    })
    expect(r?.files).toHaveLength(2)
    expect(r?.files[0]?.path).toBe('src/lib.rs')
  })

  it('accepts an empty files array (no minItems in the golden; runtime rejects)', () => {
    const r = safeParseRequest(createFileRequestSchema, { files: [] })
    expect(r?.files).toHaveLength(0)
  })
})

describe('createFileResponseSchema', () => {
  it('parses raw success with error omitted (not null)', () => {
    const r = safeParseResponse(createFileResponseSchema, {
      results: [{ path: '/work/a.txt', success: true, bytes_written: 5 }],
    })
    expect(r?.results[0]?.bytes_written).toBe(5)
    expect(r?.results[0]?.error).toBeUndefined()
  })

  it('parses wrapped partial failure with a structured WireError', () => {
    const r = safeParseResponse(
      createFileResponseSchema,
      wrap({
        results: [
          {
            path: '.env',
            success: false,
            bytes_written: 0,
            error: {
              code: 'C211',
              message: 'not accessible — matches non_accessible_globs',
            },
          },
          { path: '/work/ok.txt', success: true, bytes_written: 1 },
        ],
      }),
    )
    expect(r?.results).toHaveLength(2)
    expect(r?.results[0]?.success).toBe(false)
    expect(r?.results[0]?.error?.code).toBe('C211')
    expect(r?.results[1]?.error).toBeUndefined()
  })
})

describe('updateFileRequestSchema', () => {
  it('accepts the golden request with all four op kinds', () => {
    const r = safeParseRequest(updateFileRequestSchema, {
      files: [
        {
          path: 'src/lib.rs',
          ops: [
            { op: 'insert', at_line: 1, content: '// generated by coder\n' },
            { op: 'remove', from_line: 10, to_line: 12 },
            {
              op: 'update_lines',
              from_line: 5,
              to_line: 7,
              content: 'pub fn hello() {\n    println!("hello");\n}\n',
            },
            {
              op: 'replace',
              pattern: '// BEGIN legacy.*?// END legacy',
              replacement: '// removed',
              dot_matches_newline: true,
              expect_matches: 1,
            },
          ],
        },
      ],
    })
    expect(r?.files[0]?.ops).toHaveLength(4)
    const ops = r?.files[0]?.ops
    if (!ops?.[0] || !ops[3]) throw new Error('expected ops')
    expect(formatUpdateOp(ops[0])).toBe('insert @ L1')
    expect(formatUpdateOp(ops[3])).toContain('replace')
    expect(formatUpdateOp(ops[3])).toContain('(expect 1)')
  })

  it('accepts a null expect_matches (replace all unconditionally)', () => {
    const op = updateOpReplaceSchema.safeParse({
      op: 'replace',
      pattern: 'foo',
      replacement: 'bar',
      expect_matches: null,
    })
    expect(op.success).toBe(true)
    if (op.success) expect(formatUpdateOp(op.data)).not.toContain('expect')
  })

  // Golden-valid: no minItems on ops. The runtime answers with a per-file
  // C210 ('ops must not be empty'), so the request must parse for the
  // structured per-file failure view to render.
  it('accepts a file spec with empty ops (runtime rejects per-file with C210)', () => {
    const r = safeParseRequest(updateFileRequestSchema, {
      files: [{ path: 'src/lib.rs', ops: [] }],
    })
    expect(r?.files[0]?.ops).toHaveLength(0)
  })
})

describe('updateFileResponseSchema', () => {
  it('unwraps the harness envelope and parses echoes', () => {
    const payload = {
      results: [
        {
          path: '/work/src/lib.rs',
          success: true,
          applied: 2,
          new_line_count: 42,
          echoes: [
            {
              op_index: 0,
              from_line: 1,
              lines: ['// generated by coder', 'pub mod utils;'],
            },
          ],
          echoes_truncated: false,
        },
      ],
    }
    expect(unwrapEnvelope(wrap(payload))).toEqual(payload)
    const r = safeParseResponse(updateFileResponseSchema, wrap(payload))
    expect(r?.results[0]?.applied).toBe(2)
    expect(r?.results[0]?.echoes[0]?.lines).toHaveLength(2)
    expect(r?.results[0]?.echoes[0]?.elided).toBeUndefined()
  })

  it('parses replace-site echoes with elided + total_replacements', () => {
    const r = safeParseResponse(updateFileResponseSchema, {
      results: [
        {
          path: '/work/src/lib.rs',
          success: true,
          applied: 1,
          new_line_count: 120,
          echoes: [
            {
              op_index: 0,
              from_line: 10,
              lines: ['fn first() {', '}'],
              elided: 4,
              total_replacements: 7,
            },
            {
              op_index: 0,
              from_line: 55,
              lines: ['fn second() {}'],
              total_replacements: 7,
            },
          ],
          echoes_truncated: true,
        },
      ],
    })
    expect(r?.results[0]?.echoes[0]?.elided).toBe(4)
    expect(r?.results[0]?.echoes[0]?.total_replacements).toBe(7)
    expect(r?.results[0]?.echoes[1]?.elided).toBeUndefined()
    expect(r?.results[0]?.echoes_truncated).toBe(true)
  })

  it('parses a failed file with empty echoes and a WireError', () => {
    const r = safeParseResponse(updateFileResponseSchema, {
      results: [
        {
          path: '/work/src/gone.rs',
          success: false,
          applied: 0,
          new_line_count: 0,
          echoes: [],
          echoes_truncated: false,
          error: {
            code: 'C210',
            message: 'expect_matches: 1 but pattern matched 0 times',
          },
        },
      ],
    })
    expect(r?.results[0]?.echoes).toEqual([])
    expect(r?.results[0]?.error?.code).toBe('C210')
  })

  it('parses top-level checks alongside the results', () => {
    const r = safeParseResponse(updateFileResponseSchema, {
      results: [
        {
          path: '/work/src/lib.rs',
          success: true,
          applied: 1,
          new_line_count: 10,
          echoes: [],
          echoes_truncated: false,
        },
      ],
      checks: [
        { command: 'cargo check', exit_code: 0, output: '', truncated: false },
      ],
    })
    expect(r?.checks).toHaveLength(1)
    expect(r?.checks?.[0]?.exit_code).toBe(0)
  })

  it('rejects results missing the always-present echoes fields', () => {
    expect(
      safeParseResponse(updateFileResponseSchema, {
        results: [
          { path: 'a.txt', success: true, applied: 1, new_line_count: 3 },
        ],
      }),
    ).toBeNull()
  })
})

describe('deleteFileRequestSchema', () => {
  it('accepts the golden recursive batch delete', () => {
    const r = safeParseRequest(deleteFileRequestSchema, {
      paths: ['src/old_module.rs', 'build/artifacts'],
      recursive: true,
    })
    expect(r?.recursive).toBe(true)
    expect(r?.paths).toHaveLength(2)
  })

  it('accepts an empty paths array (no minItems in the golden; runtime rejects)', () => {
    const r = safeParseRequest(deleteFileRequestSchema, { paths: [] })
    expect(r?.paths).toHaveLength(0)
  })
})

describe('deleteFileResponseSchema', () => {
  it('parses idempotent miss (already absent, not a deletion)', () => {
    const r = safeParseResponse(deleteFileResponseSchema, {
      results: [{ path: '/work/gone.txt', success: true, removed: false }],
    })
    expect(r?.results[0]?.success).toBe(true)
    expect(r?.results[0]?.removed).toBe(false)
    expect(r?.results[0]?.error).toBeUndefined()
  })

  it('parses a C210 refusal to delete an allowed root', () => {
    const r = safeParseResponse(deleteFileResponseSchema, {
      results: [
        {
          path: '/work/project',
          success: false,
          removed: false,
          error: {
            code: 'C210',
            message: 'refusing to delete an allowed root',
          },
        },
      ],
    })
    expect(r?.results[0]?.error?.code).toBe('C210')
  })
})

describe('moveFileRequestSchema', () => {
  it('accepts the golden batched move', () => {
    const r = safeParseRequest(moveFileRequestSchema, {
      files: [
        { from: 'src/old_name.rs', to: 'src/new_name.rs' },
        {
          from: 'build/output.bin',
          overwrite: true,
          to: '/tmp/coder-cache/output.bin',
        },
      ],
    })
    expect(r?.files).toHaveLength(2)
    expect(r?.files[1]?.overwrite).toBe(true)
  })

  it('accepts an empty files array (no minItems in the golden; runtime rejects)', () => {
    const r = safeParseRequest(moveFileRequestSchema, { files: [] })
    expect(r?.files).toHaveLength(0)
  })
})

describe('moveFileResponseSchema', () => {
  it('parses a wrapped mixed batch: moved, no-op self-move, C217 failure', () => {
    const r = safeParseResponse(
      moveFileResponseSchema,
      wrap({
        results: [
          {
            from: '/work/src/old_name.rs',
            to: '/work/src/new_name.rs',
            success: true,
            moved: true,
          },
          {
            from: '/work/same.rs',
            to: '/work/same.rs',
            success: true,
            moved: false,
          },
          {
            from: '/work/build/output.bin',
            to: '/tmp/coder-cache/output.bin',
            success: false,
            moved: false,
            error: {
              code: 'C217',
              message: 'destination exists — pass overwrite: true',
            },
          },
        ],
      }),
    )
    expect(r?.results[0]?.moved).toBe(true)
    // success + !moved = no-op self-move, render as "unchanged".
    expect(r?.results[1]?.success).toBe(true)
    expect(r?.results[1]?.moved).toBe(false)
    expect(r?.results[1]?.error).toBeUndefined()
    expect(r?.results[2]?.error?.code).toBe('C217')
  })
})

describe('readFileRequestSchema', () => {
  it('accepts the golden single-path windowed request (numbered)', () => {
    const r = safeParseRequest(readFileRequestSchema, {
      line_from: 10,
      line_to: 50,
      path: 'src/main.rs',
      numbered: true,
    })
    expect(r?.path).toBe('src/main.rs')
    expect(r?.line_from).toBe(10)
    expect(r?.numbered).toBe(true)
  })

  it('accepts the golden batch with mixed string and object targets', () => {
    const r = safeParseRequest(readFileRequestSchema, {
      paths: [
        'src/lib.rs',
        { line_from: 1, line_to: 30, path: 'src/config.rs' },
        { path: 'assets/logo.bin', stat: true },
      ],
    })
    expect(r?.paths).toHaveLength(3)
    expect(r?.paths?.[0]).toBe('src/lib.rs')
  })
})

describe('readFileResponseSchema', () => {
  it('parses a single-path full read (scalars set, results omitted)', () => {
    const r = safeParseResponse(
      readFileResponseSchema,
      wrap({
        path: '/work/src/main.rs',
        content: 'fn main() {}\n',
        is_utf8: true,
        lines_returned: 1,
        total_lines: 1,
        more_lines: false,
        size: 13,
        mode: 420,
        mtime: 1760000000,
      }),
    )
    expect(r?.content).toBe('fn main() {}\n')
    expect(r?.results).toBeUndefined()
  })

  it('distinguishes absent total_lines (not traversed) from zero', () => {
    const r = safeParseResponse(readFileResponseSchema, {
      path: '/work/big.log',
      content: 'line 40\nline 41\n',
      is_utf8: true,
      lines_returned: 2,
      more_lines: true,
      size: 9999999,
    })
    expect(r?.more_lines).toBe(true)
    expect(r?.total_lines).toBeUndefined()
  })

  it('parses a stat-only probe on a file too large to line-count', () => {
    // total_lines / is_utf8 stay absent beyond max_read_bytes; stat SUCCEEDS.
    const r = safeParseResponse(readFileResponseSchema, {
      path: '/work/huge.bin',
      lines_returned: 0,
      more_lines: false,
      size: 50000000,
      mode: 420,
      mtime: 1760000000,
    })
    expect(r?.content).toBeUndefined()
    expect(r?.lines_returned).toBe(0)
    expect(r?.is_utf8).toBeUndefined()
    expect(r?.total_lines).toBeUndefined()
  })

  it('tolerates explicit nulls for optional fields', () => {
    const r = safeParseResponse(readFileResponseSchema, {
      path: '/work/a.txt',
      content: null,
      total_lines: null,
    })
    expect(r?.content).toBeNull()
  })

  it('parses a batch with a per-entry C213 budget failure mid-stream', () => {
    const r = safeParseResponse(readFileResponseSchema, {
      results: [
        {
          path: '/work/src/lib.rs',
          success: true,
          content: 'pub mod utils;\n',
          is_utf8: true,
          lines_returned: 1,
          total_lines: 1,
          more_lines: false,
          size: 15,
          mode: 420,
          mtime: 1760000000,
        },
        {
          path: '/work/src/big.rs',
          success: false,
          error: {
            code: 'C213',
            message:
              'batch_read_budget_bytes (1048576) exhausted after 1048561 bytes — read this file individually',
          },
        },
      ],
    })
    expect(r?.results).toHaveLength(2)
    expect(r?.results?.[0]?.success).toBe(true)
    expect(r?.results?.[1]?.error?.code).toBe('C213')
    // size omitted when the budget died before the file was opened.
    expect(r?.results?.[1]?.size).toBeUndefined()
  })
})

describe('searchRequestSchema', () => {
  it('accepts the golden scoped content search with context lines', () => {
    const r = safeParseRequest(searchRequestSchema, {
      context_lines_after: 2,
      context_lines_before: 2,
      include_globs: ['**/*.rs'],
      path: 'src',
      query: 'fn handle',
      search_content: true,
      search_paths: false,
    })
    expect(r?.query).toBe('fn handle')
    expect(r?.context_lines_before).toBe(2)
  })

  it('accepts a minimal query-only request', () => {
    const r = safeParseRequest(searchRequestSchema, { query: 'TODO' })
    expect(r?.query).toBe('TODO')
    expect(r?.regex).toBeUndefined()
  })

  it('rejects a request without query', () => {
    expect(safeParseRequest(searchRequestSchema, { path: 'src' })).toBeNull()
  })
})

describe('searchResponseSchema', () => {
  it('parses content matches with and without context arrays', () => {
    const r = safeParseResponse(
      searchResponseSchema,
      wrap({
        content_matches: [
          {
            path: '/work/src/main.rs',
            line: 12,
            column: 4,
            text: 'fn handle_request() {',
            before: ['// router entry', ''],
            after: ['    let body = read();', '    respond(body)'],
          },
          {
            path: '/work/src/lib.rs',
            line: 1,
            column: 1,
            text: 'fn handle() {}',
          },
        ],
        path_matches: [{ path: '/work/src/handlers.rs' }],
        truncated: true,
      }),
    )
    expect(r?.content_matches[0]?.before).toHaveLength(2)
    // Omitted context = zero context, not an empty array.
    expect(r?.content_matches[1]?.before).toBeUndefined()
    expect(r?.path_matches[0]?.path).toBe('/work/src/handlers.rs')
    expect(r?.truncated).toBe(true)
  })

  it('rejects a content match missing the column', () => {
    expect(
      contentMatchSchema.safeParse({ path: 'a.rs', line: 1, text: 'x' })
        .success,
    ).toBe(false)
  })
})

describe('treeRequestSchema', () => {
  it('accepts the golden depth-limited request and an empty request', () => {
    const r = safeParseRequest(treeRequestSchema, { max_depth: 3, path: '.' })
    expect(r?.max_depth).toBe(3)
    expect(safeParseRequest(treeRequestSchema, {})).toEqual({})
  })
})

describe('treeResponseSchema', () => {
  it('parses a nested tree with truncation stubs and wire omissions', () => {
    const r = safeParseResponse(
      treeResponseSchema,
      wrap({
        path: '/work/project',
        root: {
          name: 'project',
          kind: 'dir',
          size: 0,
          mtime: 1760000000,
          children: [
            // File node: children/truncated omitted, non_accessible omitted.
            { name: 'README.md', kind: 'file', size: 1024, mtime: 1760000000 },
            {
              name: 'src',
              kind: 'dir',
              size: 0,
              mtime: 1760000000,
              children: [
                {
                  name: 'main.rs',
                  kind: 'file',
                  size: 2048,
                  mtime: 1760000000,
                },
              ],
              truncated: {
                reason: 'per_folder_limit',
                shown: 1,
                total: 73,
                hint: 'use coder::list-folder to paginate src',
              },
            },
            // Default-exclude stub: childless dir, no total.
            {
              name: 'node_modules',
              kind: 'dir',
              size: 0,
              mtime: 1760000000,
              truncated: {
                reason: 'default_exclude',
                shown: 0,
                hint: 'pass use_default_excludes: false to descend',
              },
            },
            {
              name: '.env',
              kind: 'file',
              size: 64,
              mtime: 1760000000,
              non_accessible: true,
            },
          ],
        },
      }),
    )
    expect(r?.root.name).toBe('project')
    // non_accessible omitted on the wire when false — defaulted by the schema.
    expect(r?.root.non_accessible).toBe(false)
    expect(r?.root.children?.[0]?.non_accessible).toBe(false)
    expect(r?.root.children?.[0]?.children).toBeUndefined()
    expect(r?.root.children?.[1]?.truncated?.reason).toBe('per_folder_limit')
    expect(r?.root.children?.[1]?.truncated?.total).toBe(73)
    expect(r?.root.children?.[2]?.truncated?.total).toBeUndefined()
    expect(r?.root.children?.[3]?.non_accessible).toBe(true)
  })
})

describe('listFolderRequestSchema', () => {
  it('accepts the golden paged request and an empty request', () => {
    const r = safeParseRequest(listFolderRequestSchema, {
      page: 1,
      page_size: 50,
      path: 'src',
    })
    expect(r?.page_size).toBe(50)
    expect(safeParseRequest(listFolderRequestSchema, {})).toEqual({})
  })

  it('accepts a null page_size (config default applies)', () => {
    const r = safeParseRequest(listFolderRequestSchema, { page_size: null })
    expect(r?.page_size).toBeNull()
  })
})

describe('listFolderResponseSchema', () => {
  it('parses a page with non_accessible always present per entry', () => {
    const r = safeParseResponse(
      listFolderResponseSchema,
      wrap({
        path: '/work/project/src',
        entries: [
          {
            name: 'main.rs',
            kind: 'file',
            size: 2048,
            mtime: 1760000000,
            non_accessible: false,
          },
          {
            name: 'secrets',
            kind: 'dir',
            size: 0,
            mtime: 1760000000,
            non_accessible: true,
          },
        ],
        page: 1,
        page_size: 50,
        total: 73,
        has_more: true,
      }),
    )
    expect(r?.entries).toHaveLength(2)
    expect(r?.entries[1]?.non_accessible).toBe(true)
    expect(r?.has_more).toBe(true)
    // Entries carry basenames only — full path is derived.
    const first = r?.entries[0]
    if (!r || !first) throw new Error('expected entry')
    expect(joinEntryPath(r.path, first.name)).toBe('/work/project/src/main.rs')
  })

  it('rejects entries missing the always-present non_accessible', () => {
    expect(
      safeParseResponse(listFolderResponseSchema, {
        path: '/work',
        entries: [{ name: 'a', kind: 'file', size: 1, mtime: 1 }],
        page: 1,
        page_size: 50,
        total: 1,
        has_more: false,
      }),
    ).toBeNull()
  })
})

describe('checkResultSchema', () => {
  it('parses an errored check with the exit code omitted', () => {
    const r = checkResultSchema.safeParse({
      command: 'cargo test',
      output: '',
      truncated: true,
      error: 'timed out after 120s',
    })
    expect(r.success).toBe(true)
    if (r.success) {
      expect(r.data.exit_code).toBeUndefined()
      expect(r.data.error).toBe('timed out after 120s')
    }
  })

  it('rejects a check missing the command', () => {
    expect(
      checkResultSchema.safeParse({ output: '', truncated: false }).success,
    ).toBe(false)
  })
})

describe('applyPatchRequestSchema / applyPatchResponseSchema', () => {
  it('accepts the opaque patch string request', () => {
    const r = safeParseRequest(applyPatchRequestSchema, {
      patch: '*** Begin Patch\n*** Delete File: old.rs\n*** End Patch\n',
    })
    expect(r?.patch).toContain('*** Delete File: old.rs')
  })

  it('rejects a request without patch text', () => {
    expect(safeParseRequest(applyPatchRequestSchema, {})).toBeNull()
  })

  it('parses a wrapped mixed-kind response with echo and checks', () => {
    const r = safeParseResponse(
      applyPatchResponseSchema,
      wrap({
        results: [
          {
            path: '/work/src/main.rs',
            kind: 'moved',
            new_line_count: 30,
            echo: { from_line: 9, lines: ["  'demo::sum',"] },
          },
          { path: '/work/src/lib/log.rs', kind: 'added', new_line_count: 3 },
          { path: '/work/src/adapters.rs', kind: 'deleted' },
        ],
        checks: [
          {
            command: 'cargo check',
            exit_code: 101,
            output: 'error[E0425]: cannot find value `vm`\n',
            truncated: false,
          },
        ],
      }),
    )
    expect(r?.results).toHaveLength(3)
    expect(r?.results[0]?.kind).toBe('moved')
    expect(r?.results[0]?.echo?.from_line).toBe(9)
    // Deleted files carry no line count or echo.
    expect(r?.results[2]?.new_line_count).toBeUndefined()
    expect(r?.results[2]?.echo).toBeUndefined()
    expect(r?.checks?.[0]?.exit_code).toBe(101)
  })

  it('rejects a result with an unknown kind', () => {
    expect(
      safeParseResponse(applyPatchResponseSchema, {
        results: [{ path: '/work/a.rs', kind: 'renamed' }],
      }),
    ).toBeNull()
  })
})

describe('contextRequestSchema / contextResponseSchema', () => {
  it('accepts an empty discovery request (null coerced to {})', () => {
    expect(safeParseRequest(contextRequestSchema, undefined)).toEqual({})
  })

  it('parses a wrapped full workspace payload', () => {
    const r = safeParseResponse(
      contextResponseSchema,
      wrap({
        primary_root: '/work',
        base_paths: ['/work', '/tmp/coder-cache'],
        platform: { os: 'linux', arch: 'aarch64' },
        git: {
          branch: 'feat/worktrees',
          status: [' M src/main.rs', '?? src/lib/'],
          status_truncated: false,
          recent_commits: ['936ba3be fix: surface stream-fatal errors'],
        },
        instruction_files: [
          { path: 'CLAUDE.md', content: '# Project notes\n', truncated: false },
        ],
      }),
    )
    expect(r?.primary_root).toBe('/work')
    expect(r?.git?.branch).toBe('feat/worktrees')
    expect(r?.git?.status).toHaveLength(2)
    expect(r?.instruction_files[0]?.path).toBe('CLAUDE.md')
  })

  it('tolerates an absent git block (not a repository)', () => {
    const r = safeParseResponse(contextResponseSchema, {
      primary_root: '/work',
      base_paths: ['/work'],
      platform: { os: 'darwin', arch: 'arm64' },
      instruction_files: [],
    })
    expect(r?.git).toBeUndefined()
  })
})

describe('worktree schemas', () => {
  it('parses add: name in, path + branch out', () => {
    const req = safeParseRequest(worktreeAddRequestSchema, {
      name: 'fix-timeouts',
    })
    expect(req?.name).toBe('fix-timeouts')
    const r = safeParseResponse(
      worktreeAddResponseSchema,
      wrap({
        path: '/work/.worktrees/fix-timeouts',
        branch: 'worktree-fix-timeouts',
      }),
    )
    expect(r?.path).toBe('/work/.worktrees/fix-timeouts')
    expect(r?.branch).toBe('worktree-fix-timeouts')
  })

  it('rejects an add request without a name', () => {
    expect(safeParseRequest(worktreeAddRequestSchema, {})).toBeNull()
  })

  it('parses remove: kept-dirty refusal with the branch intact', () => {
    const req = safeParseRequest(worktreeRemoveRequestSchema, {
      name: 'fix-timeouts',
    })
    expect(req?.name).toBe('fix-timeouts')
    const r = safeParseResponse(worktreeRemoveResponseSchema, {
      removed: false,
      dirty: true,
      path: '/work/.worktrees/fix-timeouts',
      branch: 'worktree-fix-timeouts',
      branch_deleted: false,
    })
    expect(r?.removed).toBe(false)
    expect(r?.dirty).toBe(true)
    expect(r?.branch_deleted).toBe(false)
  })
})

describe('formatUpdateOp', () => {
  it('labels line ops with 1-based ranges', () => {
    expect(formatUpdateOp({ op: 'remove', from_line: 3, to_line: 9 })).toBe(
      'remove L3–9',
    )
  })

  it('shows regex flags and expect_matches on replace ops', () => {
    expect(
      formatUpdateOp({
        op: 'replace',
        pattern: 'foo',
        replacement: 'bar',
        ignore_case: true,
        dot_matches_newline: true,
        expect_matches: 1,
      }),
    ).toBe('replace /foo/is → bar (expect 1)')
  })

  it('surfaces expect_matches: 0 (assert absence)', () => {
    expect(
      formatUpdateOp({
        op: 'replace',
        pattern: 'legacy',
        replacement: '',
        expect_matches: 0,
      }),
    ).toBe("replace /legacy/ → '' (expect 0)")
  })
})

describe('truncateInline', () => {
  it('collapses whitespace and appends an ellipsis past the cap', () => {
    expect(truncateInline('a  b\n\tc')).toBe('a b c')
    const long = 'x'.repeat(60)
    expect(truncateInline(long)).toHaveLength(48)
    expect(truncateInline(long).endsWith('…')).toBe(true)
  })
})

describe('joinEntryPath', () => {
  it('joins basenames onto parent paths without doubling slashes', () => {
    expect(joinEntryPath('/work/src', 'main.rs')).toBe('/work/src/main.rs')
    expect(joinEntryPath('/', 'etc')).toBe('/etc')
    expect(joinEntryPath('', 'rel.txt')).toBe('rel.txt')
  })
})
