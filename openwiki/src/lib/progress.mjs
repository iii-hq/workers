// In-process progress bus. Generation runs in this worker; the SSE handler
// (openwiki::http::events) subscribes per wiki id and pushes frames to the
// browser over the HTTP response channel. Single-process worker; if openwiki
// ever runs multiple instances, swap this for stream::send + a stream trigger.
import { EventEmitter } from 'node:events';

const bus = new EventEmitter();
bus.setMaxListeners(0);

export function pushProgress(wikiId, evt) {
  bus.emit(wikiId, evt);
}

export function onProgress(wikiId, cb) {
  bus.on(wikiId, cb);
  return () => bus.off(wikiId, cb);
}
