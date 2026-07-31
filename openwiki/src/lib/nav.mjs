// Navigation-tree helpers. The planner returns a nested nav tree (folders +
// leaf pages) plus a flat page list; these normalize the two planner shapes
// (harness: {pages, navigation}; fallback: {categories, outline}) into a single
// { summary, outline, navigation } the generator and UI both use.

// Flatten every leaf slug in a nav tree.
export function navSlugs(navigation) {
  const out = [];
  const walk = (nodes) => {
    for (const n of nodes || []) {
      if (n.slug) out.push(n.slug);
      if (n.children) walk(n.children);
    }
  };
  walk(navigation);
  return out;
}

// Map each leaf slug to its top-level folder title (used as page.category).
export function slugToSection(navigation) {
  const map = {};
  for (const l1 of navigation || []) {
    const section = l1.title;
    const mark = (nodes) => {
      for (const n of nodes || []) {
        if (n.slug) map[n.slug] = section;
        if (n.children) mark(n.children);
      }
    };
    if (l1.slug) map[l1.slug] = section;
    mark(l1.children);
  }
  return map;
}

// Build a flat nav tree from {categories, outline} (fallback / heuristic plan).
export function navFromCategories(categories, outline) {
  const cats = categories?.length ? categories : [];
  const nav = [];
  const covered = new Set();
  for (const c of cats) {
    const leaves = (outline || [])
      .filter((o) => (o.category || '') === c.id)
      .map((o) => {
        covered.add(o.slug);
        return { title: o.title, slug: o.slug };
      });
    if (leaves.length) nav.push({ title: c.title || c.id, children: leaves });
  }
  const rest = (outline || []).filter((o) => !covered.has(o.slug)).map((o) => ({ title: o.title, slug: o.slug }));
  if (rest.length) nav.push({ title: cats.length ? 'Other' : 'Pages', children: rest });
  return nav;
}

// Normalize any planner output into { summary, outline, navigation }.
export function normalizePlan(planned) {
  if (Array.isArray(planned.pages)) {
    const navigation = planned.navigation?.length
      ? planned.navigation
      : [{ title: 'Pages', children: planned.pages.map((p) => ({ title: p.title, slug: p.slug })) }];
    const sect = slugToSection(navigation);
    const outline = planned.pages.map((p) => ({
      slug: p.slug,
      title: p.title,
      brief: p.brief,
      source_paths: p.source_paths || [],
      category: sect[p.slug] || 'Pages',
    }));
    return { summary: planned.summary || '', outline, navigation };
  }
  const navigation = navFromCategories(planned.categories || [], planned.outline || []);
  const sect = slugToSection(navigation);
  const outline = (planned.outline || []).map((o) => ({ ...o, category: sect[o.slug] || o.category || 'Pages' }));
  return { summary: planned.summary || '', outline, navigation };
}
