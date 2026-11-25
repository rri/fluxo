// fluxo service worker: keeps pages readable offline (G20). Served from /sw.js; the server fills
// in BUILD, which changes whenever this file or a static file does.
//
// - Static files are kept per build and served from the device.
// - Focus, Goals, goal pages and task pages are fetched from the network first and a copy is kept;
//   offline, the copy is shown, marked with when it was saved. No other page is ever kept
//   (Settings, sign-in, invitations, exports, codes).
// - Pages belong to whoever is signed in: signing in or out, or being sent to sign-in, deletes
//   every kept page.
'use strict';

const BUILD = '__BUILD__';
const STATIC = 'fluxo-static-' + BUILD;
const PAGES = 'fluxo-pages';
const ASSETS = [
  '/offline',
  '/static/styles.css',
  '/static/app.js',
  '/static/theme.js',
  '/static/favicon.svg',
  '/static/manifest.webmanifest',
  '/static/icon-180.png',
];
const KEEP_MS = 30 * 24 * 3600 * 1000; // pages opened in the last 30 days stay
const REFRESH_MS = 30 * 60 * 1000; // re-save the person's own pages at most every 30 minutes
const MAX_PAGES = 400;
const NETWORK_WAIT_MS = 6000; // then show the saved copy rather than keep waiting
const SAVED = 'x-fluxo-saved'; // when a kept copy was saved (ms since 1970)

// Focus, Goals, a goal, a task. Nothing else is kept.
const KEEPABLE = /^\/(?:goals(?:\/\d+)?|tasks\/\d+)?$/;
// Posting to these changes who is signed in.
const SIGN_IN_OR_OUT = /^\/(?:login(?:\/2fa|\/recovery)?|logout|signup|setup|join\/[^/]+|locked|reset-password|settings\/delete)$/;

let lastRefresh = 0;

self.addEventListener('install', (e) => {
  e.waitUntil(caches.open(STATIC).then((c) => c.addAll(ASSETS)).then(() => self.skipWaiting()));
});

self.addEventListener('activate', (e) => {
  e.waitUntil(
    caches
      .keys()
      .then((keys) => Promise.all(keys.filter((k) => k.startsWith('fluxo-static-') && k !== STATIC).map((k) => caches.delete(k))))
      .then(() => self.clients.claim()),
  );
});

self.addEventListener('message', (e) => {
  const type = e.data && e.data.type;
  if (type === 'refresh') e.waitUntil(refresh());
  if (type === 'signed-out') e.waitUntil(forgetPages()); // the sign-in page says so
});

self.addEventListener('fetch', (e) => {
  const req = e.request;
  const url = new URL(req.url);
  if (url.origin !== self.location.origin) return;
  if (req.method === 'POST' && SIGN_IN_OR_OUT.test(url.pathname)) {
    e.respondWith(forgetPages().then(() => fetch(req)));
    return;
  }
  if (req.method !== 'GET') return;
  if (url.pathname.startsWith('/static/')) {
    e.respondWith(caches.match(req, { cacheName: STATIC }).then((hit) => hit || fetch(req)));
    return;
  }
  if (req.mode !== 'navigate') return;
  e.respondWith(KEEPABLE.test(url.pathname) ? keptPage(e, req) : livePage(req));
});

/** A page that's never kept: the network, or the offline page when there's none. */
async function livePage(req) {
  try {
    return await fetch(req);
  } catch (_) {
    return offlinePage();
  }
}

/** A page that's kept: the network first, saving a copy; the saved copy when that fails or stalls. */
async function keptPage(e, req) {
  const network = fetch(req).then(async (res) => {
    await remember(req.url, res.clone());
    return res;
  });
  e.waitUntil(network.catch(() => {}));
  let timer;
  const stalled = new Promise((resolve) => { timer = setTimeout(resolve, NETWORK_WAIT_MS); });
  try {
    const res = await Promise.race([network, stalled]);
    if (res) return res;
  } catch (_) {
    // offline: fall through to the saved copy
  } finally {
    clearTimeout(timer);
  }
  const saved = await savedCopy(req.url);
  if (saved) return saved;
  try {
    return await network; // nothing saved: keep waiting for the network after all
  } catch (_) {
    return offlinePage();
  }
}

/**
 * Keep a copy of a page response. A page that's gone or no longer readable (404, 403) loses its
 * copy; being sent to sign-in loses every copy. Other redirects (a goal address for a task) and
 * page navigations' opaque redirects keep nothing.
 */
async function remember(url, res) {
  if (res.type === 'opaqueredirect') return;
  const path = new URL(res.url || url).pathname;
  if (res.redirected && !KEEPABLE.test(path)) {
    if (path === '/login') await forgetPages();
    return;
  }
  if (res.status === 404 || res.status === 403) {
    await (await caches.open(PAGES)).delete(url);
    return;
  }
  const type = res.headers.get('content-type') || '';
  if (res.status !== 200 || !type.startsWith('text/html')) return;
  const headers = new Headers(res.headers);
  headers.set(SAVED, String(Date.now()));
  const copy = new Response(await res.blob(), { status: 200, statusText: res.statusText, headers });
  const cache = await caches.open(PAGES);
  await cache.put(url, copy);
}

/** The saved copy of a page, marked so the page knows it's a copy and from when. */
async function savedCopy(url) {
  const cache = await caches.open(PAGES);
  const hit = await cache.match(url);
  if (!hit) return null;
  const saved = hit.headers.get(SAVED) || '0';
  const html = (await hit.text()).replace('<head>', '<head><meta name="fluxo-saved" content="' + Number(saved) + '">');
  const headers = new Headers(hit.headers);
  headers.delete(SAVED);
  return new Response(html, { status: 200, headers });
}

async function offlinePage() {
  return (await caches.match('/offline', { cacheName: STATIC })) || Response.error();
}

function forgetPages() {
  lastRefresh = 0;
  return caches.delete(PAGES);
}

/**
 * Save fresh copies of the person's own pages (the server lists them), then drop copies that are
 * neither on that list nor opened in the last 30 days.
 */
async function refresh() {
  if (Date.now() - lastRefresh < REFRESH_MS) return;
  lastRefresh = Date.now();
  let list;
  try {
    const res = await fetch('/offline/pages', { credentials: 'same-origin' });
    if (res.redirected && new URL(res.url).pathname === '/login') return forgetPages();
    if (!res.ok || !(res.headers.get('content-type') || '').startsWith('application/json')) return;
    list = (await res.json()).pages.slice(0, MAX_PAGES);
  } catch (_) {
    lastRefresh = 0; // try again on the next page
    return;
  }
  const wanted = new Set(list.map((p) => new URL(p, self.location.origin).href));
  const queue = Array.from(wanted);
  const worker = async () => {
    for (let url = queue.shift(); url; url = queue.shift()) {
      try {
        await remember(url, await fetch(url, { credentials: 'same-origin' }));
      } catch (_) {
        return; // offline again: stop quietly
      }
    }
  };
  await Promise.all([worker(), worker()]);
  const cache = await caches.open(PAGES);
  const old = Date.now() - KEEP_MS;
  const kept = [];
  for (const req of await cache.keys()) {
    const res = await cache.match(req);
    const saved = Number((res && res.headers.get(SAVED)) || 0);
    if (!wanted.has(req.url) && saved < old) await cache.delete(req);
    else kept.push({ req, saved, wanted: wanted.has(req.url) });
  }
  // Over the limit: the person's own pages first, then the most recently saved.
  kept.sort((a, b) => (b.wanted - a.wanted) || (b.saved - a.saved));
  for (const k of kept.slice(MAX_PAGES)) await cache.delete(k.req);
}
