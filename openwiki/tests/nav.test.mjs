import { test } from 'node:test';
import assert from 'node:assert/strict';
import { normalizePlan, navSlugs, slugToSection, navFromCategories } from '../src/lib/nav.mjs';

test('normalizePlan (harness shape) keeps nav and derives page sections', () => {
  const planned = {
    summary: 's',
    pages: [
      { slug: 'overview', title: 'Overview' },
      { slug: 'api', title: 'API' },
    ],
    navigation: [
      { title: 'Start Here', children: [{ title: 'Overview', slug: 'overview' }] },
      { title: 'Reference', children: [{ title: 'API', slug: 'api' }] },
    ],
  };
  const n = normalizePlan(planned);
  assert.equal(n.outline.length, 2);
  assert.equal(n.outline.find((o) => o.slug === 'api').category, 'Reference');
  assert.equal(n.navigation.length, 2);
});

test('normalizePlan (fallback shape) builds nav from categories', () => {
  const planned = {
    summary: 's',
    categories: [{ id: 'c1', title: 'Cat1' }],
    outline: [{ slug: 'a', title: 'A', category: 'c1' }],
  };
  const n = normalizePlan(planned);
  assert.equal(n.navigation[0].title, 'Cat1');
  assert.equal(n.navigation[0].children[0].slug, 'a');
  assert.equal(n.outline[0].category, 'Cat1');
});

test('navSlugs flattens nested leaves', () => {
  const nav = [
    {
      title: 'F',
      children: [
        { title: 'G', children: [{ title: 'L', slug: 'l' }] },
        { title: 'M', slug: 'm' },
      ],
    },
  ];
  assert.deepEqual(navSlugs(nav).sort(), ['l', 'm']);
});

test('slugToSection maps deep leaves to their top folder', () => {
  const nav = [{ title: 'Top', children: [{ title: 'Sub', children: [{ title: 'Deep', slug: 'd' }] }] }];
  assert.equal(slugToSection(nav).d, 'Top');
});

test('navFromCategories puts uncategorized pages under Other', () => {
  const nav = navFromCategories(
    [{ id: 'c', title: 'C' }],
    [
      { slug: 'a', title: 'A', category: 'c' },
      { slug: 'b', title: 'B' },
    ],
  );
  assert.equal(nav.find((f) => f.title === 'C').children[0].slug, 'a');
  assert.equal(nav.find((f) => f.title === 'Other').children[0].slug, 'b');
});
