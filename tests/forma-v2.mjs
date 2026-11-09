import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync, mkdirSync, mkdtempSync, cpSync, rmSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import vm from 'node:vm';
import { testDocument } from './booking-types-dom.mjs';

const root = fileURLToPath(new URL('..', import.meta.url));
const read = (path) => readFileSync(root + path, 'utf8');
const hash = (bytes) => createHash('sha256').update(bytes).digest('hex');
function files(dir) {
  return readdirSync(root + dir, { withFileTypes: true }).flatMap((entry) =>
    entry.isDirectory() ? files(`${dir}/${entry.name}`) : [`${dir}/${entry.name}`]);
}
const vendor = 'vendor/jaack-design-system';
const publicFiles = files('public');
const shippedCSS = [
  ...publicFiles.filter((path) => path.endsWith('.css')),
  `${vendor}/tokens.css`,
  ...files(`${vendor}/components`).filter((path) => path.endsWith('.css')),
];
const palette = JSON.parse(read('tests/forma-v2/palettes.json')).book;
const css = read('public/assets/app.css');
function declarations(block) {
  return Object.fromEntries([...block.matchAll(/(--[\w-]+)\s*:\s*([^;}]+)/g)]
    .map(([, name, value]) => [name, value.trim()]));
}
function themeValues(theme) {
  const block = css.match(new RegExp(`html\\[data-theme="${theme}"\\]:has\\(body\\.booking-page\\)\\s*\\{([^}]+)\\}`));
  assert.ok(block, `${theme} booking palette exists`);
  return declarations(block[1]);
}

test('no pure black in shipped CSS or vendored token/component CSS', () => {
  const forbidden = /#000\b|#000000|(?<![-\w])black\b|rgb\(\s*0\s*,\s*0\s*,\s*0\s*\)|rgba\(\s*0\s*,\s*0\s*,\s*0\s*,/i;
  for (const path of shippedCSS) assert.doesNotMatch(read(path), forbidden, path);
});

test('site assets contain no old gold, near-black, or veil markup/tokens', () => {
  for (const path of publicFiles.filter((path) => /\.(css|js|html)$/.test(path))) {
    assert.doesNotMatch(read(path), /0c0c0c/i, path);
    if (path === 'public/assets/jaack/jaack.css') continue;
    assert.doesNotMatch(read(path), /e8c547/i, path);
    assert.doesNotMatch(read(path), /--jk-bg-[\w-]+|class="jk-bg"/, path);
  }
});

// The authoritative canonical CSS still ships this default. Keep the literal
// acceptance criterion visible instead of silently weakening it or patching vendor.
test('no legacy gold anywhere in public', {
  todo: 'Canonical jaack.css contains #e8c547; removing it conflicts with byte-identical vendoring.',
}, () => {
  for (const path of publicFiles.filter((path) => /\.(css|js|html)$/.test(path))) {
    assert.ok(!/e8c547/i.test(read(path)), `${path} contains the canonical default gold`);
  }
});

test('booking palettes equal the book fixture, with twelve-percent accent washes', () => {
  const names = { bg: 'bg', surface: 'surface', ink: 'ink', ink2: 'ink-2', t2: 't2', t3: 't3', accent: 'accent', accentText: 'accent-text', onAccent: 'on-accent' };
  for (const theme of ['light', 'dark']) {
    const values = themeValues(theme);
    for (const [key, suffix] of Object.entries(names)) assert.equal(values[`--jk-${suffix}`], palette[theme][key]);
    const channels = palette[theme].accent.slice(1).match(/../g).map((hex) => parseInt(hex, 16));
    assert.equal(values['--jk-accent-wash'], `rgba(${channels.join(',')},.12)`);
  }
  const owner = declarations(read('public/assets/owner-app.css').match(/:root\s*\{([^}]+)\}/)[1]);
  for (const [key, token] of Object.entries({ bg: 'bg', surface: 'paper', ink: 'ink', ink2: 'muted', accent: 'accent', accentText: 'accent-text', onAccent: 'on-accent' })) {
    assert.equal(owner[`--${token}`], palette.dark[key]);
  }
});

test('fixture contrast checker passes and booking status colours meet AA on the background', () => {
  const result = spawnSync('python3', ['tests/forma-v2/contrast.py'], { cwd: root, encoding: 'utf8' });
  assert.equal(result.status, 0, result.stderr || result.error?.message);
  assert.match(result.stderr, /FAIL: 0/);
  const luminance = (hex) => hex.slice(1).match(/../g).map((channel) => parseInt(channel, 16) / 255)
    .map((channel) => channel <= .04045 ? channel / 12.92 : ((channel + .055) / 1.055) ** 2.4)
    .reduce((sum, channel, i) => sum + channel * [.2126, .7152, .0722][i], 0);
  for (const theme of ['light', 'dark']) {
    const values = themeValues(theme);
    for (const key of ['ok', 'fail']) {
      const a = luminance(values[`--jk-${key}`]), b = luminance(values['--jk-bg']);
      assert.ok((Math.max(a, b) + .05) / (Math.min(a, b) + .05) >= 4.5, `${theme} ${key}`);
    }
  }
});

test('loading is static, no infinite animations ship, and reduced motion remains enabled', () => {
  // Stronger than excluding reduced-motion rules: no infinite declarations anywhere.
  for (const path of shippedCSS) assert.doesNotMatch(read(path), /\b(?:animation|animation-iteration-count)\s*:[^;}]*\binfinite\b/i, path);
  assert.doesNotMatch(css, /@keyframes|book-pulse/);
  for (const path of ['public/assets/app.css', 'public/assets/owner-app.css', `${vendor}/tokens.css`]) {
    assert.match(read(path), /prefers-reduced-motion:\s*reduce/);
  }
  assert.match(read(`${vendor}/tokens.css`), /--jk-d-3:\s*280ms/);
  assert.match(read(`${vendor}/tokens.css`), /animation-duration:0ms!important/);
  assert.match(read(`${vendor}/tokens.css`), /--jk-bg:\s*#14110e\b/);
});

test('vendored manifest and provenance digest match the complete source tree', () => {
  const manifest = read(`${vendor}/SHA256SUMS`);
  const entries = manifest.trimEnd().split('\n').map((line) => line.split('  '));
  const expected = files(vendor).filter((path) => !/\/(?:VENDOR.md|SHA256SUMS|SCREENSHOTS.sha256)$/.test(path))
    .map((path) => path.slice(vendor.length + 1)).sort();
  assert.deepEqual(entries.map(([, path]) => path), expected);
  for (const [digest, path] of entries) assert.equal(hash(readFileSync(`${root}${vendor}/${path}`)), digest, path);
  assert.ok(read(`${vendor}/VENDOR.md`).includes(hash(manifest)));
});

test('vendored bundles equal clean build output; served copies are byte-identical', () => {
  const tempRoot = `${root}.tmp-forma`;
  mkdirSync(tempRoot, { recursive: true });
  const dir = mkdtempSync(`${tempRoot}/vendor-build-`);
  try {
    cpSync(`${root}${vendor}`, dir, { recursive: true });
    const result = spawnSync('sh', ['build.sh'], { cwd: dir, encoding: 'utf8' });
    assert.equal(result.status, 0, result.stderr);
    for (const name of ['jaack.css', 'jaack.js']) {
      assert.deepEqual(readFileSync(`${dir}/${name}`), readFileSync(`${root}${vendor}/${name}`));
    }
    for (const name of ['theme.js', 'jaack.css', 'jaack.js']) {
      assert.deepEqual(readFileSync(`${root}public/assets/jaack/${name}`), readFileSync(`${root}${vendor}/${name}`));
    }
  } finally { rmSync(dir, { recursive: true, force: true }); }
});

test('every design-system page loads local overrides next, and local CSS/JS URLs have content digests', () => {
  for (const path of publicFiles.filter((path) => path.endsWith('.html'))) {
    const html = read(path);
    if (html.includes('/assets/jaack/jaack.css')) {
      assert.match(html, /href="\/assets\/jaack\/jaack.css\?v=[a-f0-9]{16}">\s*<link rel="stylesheet" href="\/assets\/jaack-local.css\?v=[a-f0-9]{16}">/);
      assert.match(html, /<script src="\/assets\/jaack-local.js\?v=[a-f0-9]{16}" defer><\/script>/);
      assert.ok(html.indexOf('/assets/jaack/theme.js') < html.indexOf('/assets/jaack-local.js'));
    }
    for (const [, asset, digest] of html.matchAll(/(?:src|href)="(\/assets\/[^"?]+\.(?:css|js))\?v=([^"&]+)"/g)) {
      // manage.js is an unchanged reference fallback, absent from this override tree.
      if (!publicFiles.includes(`public${asset}`)) continue;
      assert.equal(digest, hash(readFileSync(`${root}public${asset}`)).slice(0, 16), `${path}: ${asset}`);
    }
  }
});

test('selected slots have a 2px border and a check with an empty speech alternative', () => {
  assert.match(css, /\.time-choice\.selected\s*\{[^}]*border:\s*2px solid/);
  assert.match(css, /\.manage-slots button\.selected\s*\{[^}]*border:\s*2px solid/);
  assert.match(css, /\.time-choice\.selected \.time-button::after,\s*\.manage-slots button\.selected::after\s*\{[^}]*content:\s*"✓"\s*\/\s*""/);
});

test('theme switch accessible names start with their canonical visible label in English and Italian', () => {
  for (const lang of ['en', 'it-IT']) {
    const document = testDocument(`<html lang="${lang}"><body><span data-jk-theme-switch></span></body></html>`);
    document.documentElement = document.querySelector('html');
    // The existing DOM fixture exposes attributes directly, rather than HTML lang reflection.
    document.documentElement.lang = lang;
    const context = vm.createContext({ document, Date, URLSearchParams, location: { search: '' },
      matchMedia: () => ({ matches: false }), localStorage: { getItem: () => null },
      setInterval() {}, addEventListener() {}, dispatchEvent() {}, CustomEvent: class {} });
    vm.runInContext(read('public/assets/jaack/theme.js'), context);
    vm.runInContext(read('public/assets/jaack-local.js'), context);
    for (const fn of document.listeners.DOMContentLoaded) fn();
    const buttons = document.querySelectorAll('.jk-thsw button');
    assert.equal(buttons.length, 3);
    for (const button of buttons) {
      assert.equal(button.getAttribute('aria-label'), button.querySelector('.l').textContent + (lang.startsWith('it') ? ', tema' : ', theme'));
      assert.match(button.getAttribute('aria-pressed'), /^(true|false)$/);
    }
  }
});

test('slot selection announces its pressed state through selection changes and rerenders', () => {
  const document = testDocument('<div id="slots"></div><p id="demo-note"></p>');
  const slots = [{ start: '2030-01-01T12:00:00Z', end: '2030-01-01T12:30:00Z' }, { start: '2030-01-01T13:00:00Z', end: '2030-01-01T13:30:00Z' }];
  const state = { availability: { candidates: slots }, choice: null };
  const context = vm.createContext({ document, state, $: (id) => document.getElementById(id), time: (value) => value, showDetails() {} });
  const source = read('public/assets/public.js');
  vm.runInContext(source.slice(source.indexOf('function renderSlots()'), source.indexOf('function renderBookingTypes(')), context);
  context.renderSlots();
  let rows = document.querySelectorAll('.time-choice');
  assert.ok(rows.every((row) => row.querySelector('.time-button').getAttribute('aria-pressed') === 'false'));
  context.activateTime(rows[0], slots[0]);
  context.activateTime(rows[1], slots[1]);
  assert.equal(rows[0].querySelector('.time-button').getAttribute('aria-pressed'), 'false');
  assert.equal(rows[1].querySelector('.time-button').getAttribute('aria-pressed'), 'true');
  assert.equal(rows[1].querySelector('.next-button').getAttribute('aria-hidden'), 'false');
  assert.ok(rows[1].querySelector('.next-button').getAttribute('aria-label').startsWith('Next'));
  context.renderSlots();
  rows = document.querySelectorAll('.time-choice');
  assert.equal(rows[1].querySelector('.time-button').getAttribute('aria-pressed'), 'true');
});
