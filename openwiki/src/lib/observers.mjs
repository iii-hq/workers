// The functions openwiki observes on other workers: the http worker's routes,
// the cron worker's ticks (one observer per wiki with an auto-refresh cadence)
// and the harness turn events. configuration::on-updated is observed in
// configuration.mjs.

// One http::on-request observer per route: its args are the method and the
// path, and the http worker calls the target with each matching request.
export const ROUTES = [
  { function_id: 'openwiki::http::ui', method: 'GET', path: '/openwiki' },
  { function_id: 'openwiki::http::ui', method: 'GET', path: '/openwiki/' },
  { function_id: 'openwiki::http::wikis-list', method: 'GET', path: '/openwiki/api/wikis' },
  { function_id: 'openwiki::http::models', method: 'GET', path: '/openwiki/api/models' },
  { function_id: 'openwiki::http::wikis-create', method: 'POST', path: '/openwiki/api/wikis' },
  { function_id: 'openwiki::http::wiki-get', method: 'GET', path: '/openwiki/api/wikis/:id' },
  { function_id: 'openwiki::http::wiki-delete', method: 'DELETE', path: '/openwiki/api/wikis/:id' },
  { function_id: 'openwiki::http::wiki-status', method: 'GET', path: '/openwiki/api/wikis/:id/status' },
  { function_id: 'openwiki::http::pages-list', method: 'GET', path: '/openwiki/api/wikis/:id/pages' },
  { function_id: 'openwiki::http::page-get', method: 'GET', path: '/openwiki/api/wikis/:id/pages/:slug' },
  { function_id: 'openwiki::http::search', method: 'GET', path: '/openwiki/api/wikis/:id/search' },
  { function_id: 'openwiki::http::refresh', method: 'POST', path: '/openwiki/api/wikis/:id/refresh' },
  { function_id: 'openwiki::http::schedule-set', method: 'PUT', path: '/openwiki/api/wikis/:id/schedule' },
  { function_id: 'openwiki::http::events', method: 'GET', path: '/openwiki/api/wikis/:id/events' },
  { function_id: 'openwiki::http::ask', method: 'POST', path: '/openwiki/api/wikis/:id/ask' },
  { function_id: 'openwiki::http::diagram', method: 'GET', path: '/openwiki/api/wikis/:id/diagram' },
];

export function observeRoutes(worker, routes = ROUTES) {
  return routes.map(({ function_id, method, path }) => {
    const handle = worker.observe({ function_id: 'http::on-request', args: { method, path } }, function_id);
    handle.onStateChange((state, reason) => {
      if (state === 'error') console.warn('[openwiki] route rejected', method, path, reason?.code, reason?.message);
    });
    return handle;
  });
}

// Each wiki carries its own auto-refresh cadence, set from the UI, never
// hardcoded. A preset maps to a cron expression and the interval after which a
// wiki is due; a raw 5-6 field cron is accepted for power users, and its due
// window is the tick cadence itself (refresh whenever it fires).
export const REFRESH_INTERVALS = {
  '3h': { cron: '0 0 */3 * * *', ms: 3 * 3600e3 },
  '6h': { cron: '0 0 */6 * * *', ms: 6 * 3600e3 },
  '12h': { cron: '0 0 */12 * * *', ms: 12 * 3600e3 },
  daily: { cron: '0 0 3 * * *', ms: 24 * 3600e3 },
  weekly: { cron: '0 0 3 * * 1', ms: 7 * 24 * 3600e3 },
};
export const SCHEDULE_PRESETS = ['off', ...Object.keys(REFRESH_INTERVALS)];
export const isRawCron = (s) => /^(\S+\s+){4,5}\S+$/.test(String(s || ''));

// Per-wiki cron observers: cron::on-tick with the wiki's expression, target
// openwiki::cron::refresh-due, metadata { wiki_id }. apply() observes (or
// replaces) one wiki's schedule; 'off' only disposes it. A new schedule is
// observed before the old one is disposed, so the cron worker keeps an
// expression's job running when both observers share it. The cron worker
// rejects an invalid expression with INVALID_CRON_EXPRESSION, logged from the
// handle. Observers live with this worker's connection, so the stored
// schedules are applied again on boot.
export function createWikiSchedules(worker) {
  const handles = new Map(); // wikiId -> observer handle

  function apply(wikiId, schedule) {
    const prev = handles.get(wikiId);
    handles.delete(wikiId);
    if (schedule && schedule !== 'off') {
      const expression = REFRESH_INTERVALS[schedule] ? REFRESH_INTERVALS[schedule].cron : schedule;
      const handle = worker.observe(
        { function_id: 'cron::on-tick', args: { expression } },
        'openwiki::cron::refresh-due',
        { metadata: { wiki_id: wikiId } },
      );
      handle.onStateChange((state, reason) => {
        if (state === 'error') console.warn('[openwiki] failed to schedule', wikiId, reason?.code, reason?.message);
      });
      handles.set(wikiId, handle);
    }
    prev?.dispose().catch(() => {
      /* already gone */
    });
  }

  return { apply, handles };
}

// Real-time page collection: one broad observer (no filters) of each harness
// turn event routes every turn to the active generation (turnbus); unowned
// roots are a cheap map miss. Both deliver the event itself
// (deliver: 'output'). They stay pending while the harness is absent;
// generation then uses its synchronous fallback.
export function observeTurnEvents(worker) {
  return [
    worker.observe({ function_id: 'harness::on-turn-completed', deliver: 'output' }, 'openwiki::on-turn-completed'),
    worker.observe({ function_id: 'harness::on-turn-started', deliver: 'output' }, 'openwiki::on-turn-started'),
  ];
}
