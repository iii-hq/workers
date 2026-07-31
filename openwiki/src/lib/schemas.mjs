// Typed wire schemas for every openwiki function.
//
// SOP docs/sops/new-worker.md §5: every registered function must publish a
// typed request AND response schema — never the permissive AnyValue schema an
// untyped handler emits. The publish pipeline runs
// collect_worker_interface.py --assert-typed-schemas, so an untyped handler
// fails the release. These constants are the single source of truth; index.mjs
// attaches them at registration.

const STRING = { type: 'string' };
const BOOL = { type: 'boolean' };
const NUM = { type: 'number' };
const INT = { type: 'integer' };

// A wiki's stored metadata record.
export const WIKI_META = {
  type: 'object',
  additionalProperties: false,
  required: ['id', 'repo_url', 'repo_name', 'page_count', 'category_count', 'generating'],
  properties: {
    id: STRING,
    repo_url: STRING,
    repo_name: STRING,
    ref: STRING,
    commit: STRING,
    created_at: STRING,
    updated_at: STRING,
    page_count: INT,
    category_count: INT,
    categories: {
      type: 'array',
      items: {
        type: 'object',
        additionalProperties: false,
        required: ['id', 'title'],
        properties: { id: STRING, title: STRING, description: STRING },
      },
    },
    summary: STRING,
    model: STRING,
    generating: BOOL,
    content_hash: STRING,
    navigation: { type: 'array', items: { type: 'object', additionalProperties: true } },
    refresh_schedule: STRING,
    last_refresh_at: STRING,
    steer: { type: 'object', additionalProperties: true },
  },
};

// A source citation pinned to a line range at a known commit.
export const CITATION = {
  type: 'object',
  additionalProperties: false,
  required: ['path'],
  properties: {
    path: STRING,
    start_line: INT,
    end_line: INT,
    note: STRING,
    url: STRING, // host deep-link at the pinned commit
  },
};

// A page's frontmatter/metadata (no markdown body).
export const PAGE_META = {
  type: 'object',
  additionalProperties: false,
  required: ['slug', 'title', 'category'],
  properties: {
    slug: STRING,
    title: STRING,
    category: STRING,
    source_paths: { type: 'array', items: STRING },
    citations: { type: 'array', items: CITATION },
    last_updated: STRING,
    confidence: { type: 'string', enum: ['low', 'medium', 'high'] },
    status: { type: 'string', enum: ['current', 'needs-review', 'stale'] },
    generator: { type: 'string', enum: ['harness', 'router', 'heuristic'] },
    quality_issues: INT,
  },
};

export const STATUS = {
  type: 'object',
  additionalProperties: true,
  required: ['phase', 'progress'],
  properties: {
    phase: {
      type: 'string',
      enum: ['queued', 'cloning', 'inventorying', 'planning', 'generating', 'linting', 'ready', 'error', 'unknown'],
    },
    progress: NUM,
    message: STRING,
    error: STRING,
    updated_at: STRING,
  },
};

const ID_REQ = {
  type: 'object',
  additionalProperties: false,
  required: ['id'],
  properties: { id: STRING },
};

// ---------- generate ----------
export const GENERATE_REQ = {
  type: 'object',
  additionalProperties: false,
  required: ['repo_url'],
  properties: {
    repo_url: { ...STRING, description: 'HTTPS git URL of a public repository.' },
    ref: { ...STRING, description: 'Optional branch/tag/commit to check out (default: default branch).' },
    model: { ...STRING, description: 'Optional LLM model id, routed via llm-router.' },
    steer: {
      type: 'object',
      description: 'Optional per-repo steering (repo_notes, explicit pages, caps). Mirrors openwiki.json.',
      additionalProperties: true,
    },
  },
};
export const GENERATE_RES = {
  type: 'object',
  additionalProperties: false,
  required: ['wiki_id', 'status'],
  properties: { wiki_id: STRING, status: STRING },
};

// ---------- refresh ----------
export const REFRESH_RES = {
  type: 'object',
  additionalProperties: false,
  required: ['wiki_id', 'refresh'],
  properties: {
    wiki_id: STRING,
    refresh: { type: 'string', enum: ['regenerating', 'up_to_date', 'in_progress', 'error'] },
    changed: {
      type: 'array',
      items: {
        type: 'object',
        additionalProperties: false,
        required: ['status', 'path'],
        properties: { status: STRING, path: STRING },
      },
    },
    pages_affected: { type: 'array', items: STRING },
  },
};

// ---------- status / wikis / wiki / pages / page / search ----------
export const STATUS_REQ = ID_REQ;
export const STATUS_RES = STATUS;

export const WIKIS_REQ = { type: 'object', additionalProperties: false, properties: {} };
export const WIKIS_RES = {
  type: 'object',
  additionalProperties: false,
  required: ['wikis'],
  properties: { wikis: { type: 'array', items: WIKI_META } },
};

export const MODELS_REQ = { type: 'object', additionalProperties: false, properties: {} };
export const MODELS_RES = {
  type: 'object',
  additionalProperties: false,
  required: ['models', 'default_model'],
  properties: {
    models: {
      type: 'array',
      items: {
        type: 'object',
        additionalProperties: false,
        required: ['id', 'provider'],
        properties: {
          id: { type: 'string' },
          provider: { type: 'string' },
          supports_structured_output: { type: 'boolean' },
        },
      },
    },
    default_model: { type: 'string' },
  },
};

export const WIKI_REQ = ID_REQ;
export const WIKI_RES = WIKI_META;

export const PAGES_REQ = ID_REQ;
export const PAGES_RES = {
  type: 'object',
  additionalProperties: false,
  required: ['pages'],
  properties: { pages: { type: 'array', items: PAGE_META } },
};

export const PAGE_REQ = {
  type: 'object',
  additionalProperties: false,
  required: ['id', 'slug'],
  properties: { id: STRING, slug: STRING },
};

// A page-writer sub-agent stores its finished page directly, so the parent never
// carries the markdown. Agent-exposed but guarded to a generating wiki.
export const WRITE_PAGE_REQ = {
  type: 'object',
  additionalProperties: false,
  required: ['id', 'slug', 'markdown'],
  properties: {
    id: STRING,
    slug: STRING,
    title: STRING,
    category: STRING,
    markdown: STRING,
    source_paths: { type: 'array', items: STRING },
    citations: {
      type: 'array',
      items: {
        type: 'object',
        additionalProperties: false,
        required: ['path'],
        properties: { path: STRING, start_line: INT, end_line: INT, note: STRING },
      },
    },
  },
};
export const WRITE_PAGE_RES = {
  type: 'object',
  additionalProperties: false,
  required: ['slug', 'ok'],
  properties: { slug: STRING, ok: { type: 'boolean' } },
};
export const SET_SCHEDULE_REQ = {
  type: 'object',
  additionalProperties: false,
  required: ['id', 'schedule'],
  properties: { id: STRING, schedule: STRING },
};
export const SET_SCHEDULE_RES = {
  type: 'object',
  additionalProperties: false,
  required: ['id', 'schedule', 'ok'],
  properties: { id: STRING, schedule: STRING, ok: { type: 'boolean' } },
};
export const PAGE_RES = {
  type: 'object',
  additionalProperties: false,
  required: ['slug', 'markdown'],
  properties: {
    slug: STRING,
    title: STRING,
    category: STRING,
    source_paths: { type: 'array', items: STRING },
    citations: { type: 'array', items: CITATION },
    last_updated: STRING,
    confidence: { type: 'string', enum: ['low', 'medium', 'high'] },
    status: { type: 'string', enum: ['current', 'needs-review', 'stale'] },
    generator: { type: 'string', enum: ['harness', 'router', 'heuristic'] },
    quality_issues: INT,
    markdown: STRING,
  },
};

export const SEARCH_REQ = {
  type: 'object',
  additionalProperties: false,
  required: ['id', 'q'],
  properties: { id: STRING, q: STRING },
};
export const SEARCH_RES = {
  type: 'object',
  additionalProperties: false,
  required: ['results'],
  properties: {
    results: {
      type: 'array',
      items: {
        type: 'object',
        additionalProperties: false,
        required: ['slug', 'score'],
        properties: {
          slug: STRING,
          title: STRING,
          category: STRING,
          source_paths: { type: 'array', items: STRING },
          score: NUM,
          snippet: STRING,
          matched: { type: 'array', items: STRING },
        },
      },
    },
  },
};

// ---------- ask ----------
export const ASK_REQ = {
  type: 'object',
  additionalProperties: false,
  required: ['id', 'q'],
  properties: {
    id: STRING,
    q: STRING,
    mode: { type: 'string', enum: ['fast', 'deep'], description: 'fast = router retrieval; deep = harness multi-hop.' },
    file_answer: { ...BOOL, description: 'File a good answer back into the wiki as a new page.' },
  },
};
export const ASK_RES = {
  type: 'object',
  additionalProperties: false,
  required: ['answer'],
  properties: {
    answer: STRING,
    citations: { type: 'array', items: CITATION },
    filed_slug: STRING,
  },
};

// ---------- lint ----------
export const LINT_REQ = ID_REQ;
export const LINT_RES = {
  type: 'object',
  additionalProperties: false,
  required: ['checked', 'issues'],
  properties: {
    checked: INT,
    issues: {
      type: 'array',
      items: {
        type: 'object',
        additionalProperties: false,
        required: ['slug', 'kind'],
        properties: {
          slug: STRING,
          kind: {
            type: 'string',
            enum: ['broken-citation', 'orphan', 'missing-xref', 'stale', 'thin'],
          },
          detail: STRING,
        },
      },
    },
  },
};

// ---------- diagram ----------
export const DIAGRAM_REQ = {
  type: 'object',
  additionalProperties: false,
  required: ['id'],
  properties: {
    id: STRING,
    kind: { type: 'string', enum: ['architecture', 'dataflow', 'deps'] },
  },
};
export const DIAGRAM_RES = {
  type: 'object',
  additionalProperties: false,
  required: ['mermaid'],
  properties: { mermaid: STRING, kind: STRING },
};

// ---------- export-agents-md ----------
export const EXPORT_AGENTS_REQ = {
  type: 'object',
  additionalProperties: false,
  required: ['id'],
  properties: {
    id: STRING,
    targets: { type: 'array', items: { type: 'string', enum: ['AGENTS.md', 'CLAUDE.md'] } },
    base_url: STRING,
  },
};
export const EXPORT_AGENTS_RES = {
  type: 'object',
  additionalProperties: false,
  required: ['content'],
  properties: { content: STRING, targets: { type: 'array', items: STRING } },
};

// ---------- harness page output contract ----------
// The JSON a harness turn returns for one page. The model supplies path + line
// range + note only; openwiki fills the host deep-link url itself, so the
// model-facing citation schema deliberately omits `url`.
const MODEL_CITATION = {
  type: 'object',
  additionalProperties: false,
  required: ['path'],
  properties: {
    path: STRING,
    start_line: INT,
    end_line: INT,
    note: STRING,
  },
};

export const PAGE_HARNESS_OUT = {
  type: 'object',
  additionalProperties: false,
  required: ['title', 'markdown'],
  properties: {
    title: STRING,
    markdown: STRING,
    citations: { type: 'array', items: MODEL_CITATION },
    links: { type: 'array', items: STRING },
    confidence: { type: 'string', enum: ['low', 'medium', 'high'] },
    status: { type: 'string', enum: ['current', 'needs-review'] },
  },
};

// ---------- harness plan output contract ----------
// The JSON a harness turn returns when planning a wiki after exploring the repo.
// navigation is a nested tree (folder = title + children, no slug; leaf = title
// + slug); pages is the flat list of leaves to generate.
const NAV_LEAF = {
  type: 'object',
  additionalProperties: false,
  required: ['title', 'slug'],
  properties: { title: STRING, slug: STRING },
};
const NAV_L2 = {
  type: 'object',
  additionalProperties: false,
  required: ['title'],
  properties: { title: STRING, slug: STRING, children: { type: 'array', items: NAV_LEAF } },
};
const NAV_L1 = {
  type: 'object',
  additionalProperties: false,
  required: ['title'],
  properties: { title: STRING, slug: STRING, children: { type: 'array', items: NAV_L2 } },
};

export const PLAN_HARNESS_OUT = {
  type: 'object',
  additionalProperties: false,
  required: ['summary', 'pages', 'navigation'],
  properties: {
    summary: STRING,
    pages: {
      type: 'array',
      items: {
        type: 'object',
        additionalProperties: false,
        required: ['slug', 'title'],
        properties: { slug: STRING, title: STRING, brief: STRING, source_paths: { type: 'array', items: STRING } },
      },
    },
    navigation: { type: 'array', items: NAV_L1 },
  },
};

// Nav tree node as stored on the wiki and read by the UI.
export const NAV_NODE = NAV_L1;

// ---------- MCP structure surface ----------
export const MCP_STRUCTURE_RES = {
  type: 'object',
  additionalProperties: false,
  required: ['pages'],
  properties: {
    repo: STRING,
    summary: STRING,
    categories: {
      type: 'array',
      items: {
        type: 'object',
        additionalProperties: false,
        required: ['id'],
        properties: { id: STRING, title: STRING, description: STRING },
      },
    },
    pages: {
      type: 'array',
      items: {
        type: 'object',
        additionalProperties: false,
        required: ['slug'],
        properties: { slug: STRING, title: STRING, category: STRING },
      },
    },
  },
};

// ---------- scoped source-read functions (the harness's exploration tools) ----------
export const SRC_READ_REQ = {
  type: 'object',
  additionalProperties: false,
  required: ['id', 'path'],
  properties: { id: STRING, path: STRING, from: INT, to: INT },
};
export const SRC_READ_RES = {
  type: 'object',
  additionalProperties: false,
  required: ['path', 'content'],
  properties: { path: STRING, content: STRING, from: INT, to: INT, total_lines: INT, truncated: BOOL },
};
export const SRC_LIST_REQ = {
  type: 'object',
  additionalProperties: false,
  required: ['id'],
  properties: { id: STRING, dir: STRING },
};
export const SRC_LIST_RES = {
  type: 'object',
  additionalProperties: false,
  required: ['files'],
  properties: {
    files: {
      type: 'array',
      items: {
        type: 'object',
        additionalProperties: false,
        required: ['path'],
        properties: { path: STRING, language: STRING, size: INT, priority: INT },
      },
    },
    truncated: BOOL,
  },
};
export const SRC_GREP_REQ = {
  type: 'object',
  additionalProperties: false,
  required: ['id', 'pattern'],
  properties: { id: STRING, pattern: STRING, max: INT },
};
export const SRC_GREP_RES = {
  type: 'object',
  additionalProperties: false,
  required: ['matches'],
  properties: {
    matches: {
      type: 'array',
      items: {
        type: 'object',
        additionalProperties: false,
        required: ['path', 'line', 'text'],
        properties: { path: STRING, line: INT, text: STRING },
      },
    },
    truncated: BOOL,
  },
};
