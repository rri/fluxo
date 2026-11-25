// Draw the home-screen icons (G19) from the flame mark: a white flame on indigo, as PNGs, because
// iOS and Android don't take SVG icons. Run after changing the mark:
//
//   (cd tests/browser && npm ci) && node scripts/app-icons.mjs
//
// Uses the browser tests' Playwright, so it draws exactly what a browser would.

import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const { chromium } = createRequire(join(root, 'tests/browser/package.json'))('playwright');
const out = join(root, 'crates/fluxo-web/static');

const TILE = '#3434b8';
const SIDES =
  'M9.9 21.5C5.6 20.6 3 17.4 3.4 13.6C3.7 11 5.4 9.6 5.3 6.8C7.6 8 8.6 9.6 8.4 11.4C6.4 14.6 6.8 18.4 9.9 21.5Z' +
  'M14.3 21.5C18.6 20.6 21 17.6 20.7 14.2C20.5 12.4 19.4 11.2 19.6 9.2C17.6 10 16.9 11.4 17.2 13C17.4 16 16.4 19 14.3 21.5Z';
const CORE = 'M12 21.75C8.8 19.4 7.9 15.6 9.6 12.2C11 9.4 13.2 7 12.6 2.25C16 4.8 17.2 9 15.9 12.6C15 15.4 14.6 18.6 12 21.75Z';

// `scale`: the flame's height as a share of the icon. Maskable icons keep it inside the central
// circle that Android may crop to; iOS rounds the corners itself, so no icon is rounded here.
const icons = [
  { file: 'icon-180.png', size: 180, scale: 0.6 },
  { file: 'icon-192.png', size: 192, scale: 0.6 },
  { file: 'icon-512.png', size: 512, scale: 0.6 },
  { file: 'icon-maskable-512.png', size: 512, scale: 0.48 },
];

const svg = (scale) => {
  const k = (24 * scale) / 19.5; // the flame is 19.5 units tall, centred on (12.05, 12)
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><rect width="24" height="24" fill="${TILE}"/>` +
    `<g fill="#fff" transform="translate(12 12) scale(${k}) translate(-12.05 -12)">` +
    `<path opacity=".6" d="${SIDES}"/><path d="${CORE}"/></g></svg>`;
};

const browser = await chromium.launch();
for (const { file, size, scale } of icons) {
  const page = await browser.newPage({ viewport: { width: size, height: size }, deviceScaleFactor: 1 });
  await page.setContent(`<style>html,body{margin:0}svg{display:block;width:${size}px;height:${size}px}</style>${svg(scale)}`);
  await page.screenshot({ path: join(out, file), omitBackground: false });
  await page.close();
  console.log('wrote', file);
}
await browser.close();
