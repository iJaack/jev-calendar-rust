import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import { testDocument } from './booking-types-dom.mjs';
const html = readFileSync(new URL('../public/index.html', import.meta.url), 'utf8');
const source = readFileSync(new URL('../public/assets/public.js', import.meta.url), 'utf8');
function helpers() {
  const context = vm.createContext({ Intl, Date });
  vm.runInContext(source.slice(0, source.indexOf('const $ =')), context);
  return context;
}
const instant = '2026-11-24T14:00:00Z';
test('language follows only the first browser preference, with capture overrides', () => {
  const head = html.match(/<script id="booking-language">([\s\S]*?)<\/script>/)?.[1];
  assert.ok(head, 'synchronous language script exists');
  const document = { documentElement: {}, title: '' };
  const ctx = vm.createContext({ document, URLSearchParams, location: { search: '' } });
  vm.runInContext(head, ctx);
  for (const languages of [['it'], ['it-IT'], ['it-CH'], ['it-SM'], ['IT-it']]) assert.equal(ctx.detectBookingLanguage({ languages }), 'it');
  for (const languages of [['en-US', 'it'], ['de', 'it-IT'], ['en-IT'], ['en'], []]) assert.equal(ctx.detectBookingLanguage({ languages }), 'en');
  assert.equal(ctx.detectBookingLanguage(), 'en');
  assert.equal(ctx.detectBookingLanguage({ language: 'it-IT' }), 'it');
  assert.equal(ctx.detectBookingLanguage({ languages: [], language: 'it-IT' }), 'it');
  assert.equal(ctx.detectBookingLanguage({ languages: ['en'] }, '?lang=it'), 'it');
  assert.equal(ctx.detectBookingLanguage({ languages: ['it'] }, '?lang=en'), 'en');
  assert.equal(ctx.detectBookingLanguage({ languages: ['it'] }, '?lang=fr'), 'it');
  assert.equal(document.documentElement.lang, 'en');
  assert.equal(document.title, 'Book a meeting with Jaack');
});
for (const lang of ['it', 'en']) {
  test(`${lang}: clock and calendar-day comparisons, host wording and accessible names`, () => {
    const { bookingTime, bookingText } = helpers();
    const b = (iso, zone, host = 'Europe/Rome') => bookingTime(iso, zone, host, lang, 45);
    for (const zone of ['Europe/Rome', 'Europe/Paris']) {
      const value = b(instant, zone);
      assert.equal(value.host, ''); assert.equal(value.summaryHost, '');
      assert.equal(value.slotLabel, lang === 'it' ? 'martedì 24 novembre, 15:00' : 'Tuesday, November 24, 3:00 PM');
    }
    const ny = b(instant, 'America/New_York');
    assert.equal(ny.guest, lang === 'it' ? '09:00' : '9:00 AM');
    assert.equal(ny.host, lang === 'it' ? '15:00 a Roma' : '3:00 PM in Rome');
    assert.equal(ny.slotLabel, lang === 'it' ? 'martedì 24 novembre, 09:00 ora di New York, 15:00 ora di Roma' : 'Tuesday, November 24, 9:00 AM New York time, 3:00 PM Rome time');
    assert.equal(ny.nextLabel, lang === 'it' ? 'Avanti, continua con martedì alle 09:00' : 'Next, continue with Tuesday at 9:00 AM');
    assert.equal(ny.actionLabel, lang === 'it' ? 'Prenota martedì alle 09:00' : 'Book Tuesday at 9:00 AM');
    assert.equal(ny.summaryGuest, lang === 'it' ? 'martedì 24 novembre 2026, 09:00' : 'Tuesday, November 24, 2026, 9:00 AM');
    assert.equal(ny.summaryMeta, '45 min · Google Meet · America/New York');
    assert.equal(bookingText(lang, 'booking'), lang === 'it' ? 'Sto prenotando…' : 'Booking…');
    const la = b('2026-11-24T05:00:00Z', 'America/Los_Angeles');
    assert.equal(la.host, lang === 'it' ? '06:00, domani a Roma' : '6:00 AM, tomorrow in Rome');
    assert.equal(la.summaryHost, lang === 'it' ? '06:00, martedì 24 novembre a Roma' : '6:00 AM, Tuesday, November 24 in Rome');
    assert.equal(la.slotLabel, lang === 'it' ? 'lunedì 23 novembre, 21:00 ora di Los Angeles, 06:00 di martedì ora di Roma' : 'Monday, November 23, 9:00 PM Los Angeles time, 6:00 AM Tuesday Rome time');
    const asia = b('2026-11-23T23:00:00Z', 'Asia/Tokyo', 'America/New_York');
    assert.equal(asia.host, lang === 'it' ? '18:00, ieri a America/New York' : '6:00 PM, yesterday in America/New York');
    const romeYesterday = b('2026-11-23T22:00:00Z', 'Asia/Tokyo');
    assert.equal(romeYesterday.host, lang === 'it' ? '23:00, ieri a Roma' : '11:00 PM, yesterday in Rome');
    // Extreme offsets can cross two calendar dates; use a weekday rather than tomorrow.
    const far = b('2026-11-24T11:00:00Z', 'Etc/GMT+12', 'Pacific/Kiritimati');
    assert.match(far.host, lang === 'it' ? /mercoledì a Pacific\/Kiritimati/ : /Wednesday in Pacific\/Kiritimati/);
    // Comparison happens at the instant, including mismatched daylight-saving transitions.
    assert.notEqual(b('2026-03-20T14:00:00Z', 'Europe/London').host, '');
  });
}
test('host header and time-zone control structure', () => {
  const document = testDocument(html);
  assert.doesNotMatch(document.querySelector('.book-host').textContent, /Europe\/Rome|host-timezone/);
  const panel = document.querySelector('.times-panel');
  assert.ok(panel.querySelector('#viewer-timezone'));
  const step = panel.querySelector('#times-step');
  assert.ok(step.children.indexOf(step.querySelector('.timezone-row')) < step.children.indexOf(step.querySelector('#times-date')));
  assert.equal(document.querySelector('label[for="viewer-timezone"]').textContent.includes('Change'), true);
  assert.equal(document.getElementById('timezone-live').getAttribute('aria-live'), 'polite');
});
test('summaries share complete safe text, including the invitation destination', () => {
  const { bookingTime, renderTimeSummary } = helpers();
  for (const lang of ['it', 'en']) for (const zone of ['America/New_York', 'Europe/Rome', 'Europe/Paris', 'America/Los_Angeles']) {
    const document = testDocument('<div id="summary"></div><div id="success"></div>');
    const value = bookingTime('2026-11-24T05:00:00Z', zone, 'Europe/Rome', lang, 30);
    const summary = document.getElementById('summary'), success = document.getElementById('success');
    renderTimeSummary(document, summary, value);
    renderTimeSummary(document, success, value, lang === 'it' ? "Controlla l'email a nome@example.com" : 'Check the email sent to name@example.com');
    for (const el of [summary, success]) {
      assert.equal(el.querySelector('.summary-guest').textContent, value.summaryGuest);
      assert.equal(el.querySelector('.summary-host')?.textContent || '', value.summaryHost);
      assert.equal(el.querySelector('.summary-meta').textContent, value.summaryMeta);
    }
    assert.ok(success.textContent.includes('nome@example.com') || success.textContent.includes('name@example.com'));
  }
});

async function bookingFixture(lang = 'en', zone = 'America/New_York', options = {}) {
  const document = testDocument(html);
  document.documentElement = document.querySelector('html');
  document.documentElement.lang = lang;
  const calls = [];
  const NativeDate = Date;
  class FixtureDate extends NativeDate {
    constructor(...args) { super(...(args.length ? args : ['2026-11-18T12:00:00Z'])); }
    static now() { return NativeDate.parse('2026-11-18T12:00:00Z'); }
  }
  const fixtureIntl = { ...Intl, DateTimeFormat: function(locale, config) {
    return config ? new Intl.DateTimeFormat(locale, config) : { resolvedOptions: () => ({ timeZone: zone }) };
  } };
  const ctx = vm.createContext({ document, Intl: fixtureIntl, Date: FixtureDate, URLSearchParams, performance: { now: () => 1000 },
    setTimeout: () => 1, clearTimeout() {}, crypto: { randomUUID: () => 'synthetic-booking-key' },
    fetch: async (path, config = {}) => {
      const input = config.body ? JSON.parse(config.body) : null; calls.push({ path, input });
      if (path === '/api/book' && options.book) return options.book(input);
      let value;
      const availability = { date: '2026-11-24', viewerTimeZone: input?.viewerTimeZone || zone, hostTimezone: 'Europe/Rome', candidates: options.emptyDay ? [] : [{ start: instant, end: '2026-11-24T14:30:00Z' }] };
      if (path === '/api/config') value = { hostTimezone: 'Europe/Rome' };
      if (path === '/api/available-dates') value = { dates: options.emptyMonth ? [] : ['2026-11-24'], firstDateAvailability: availability };
      if (path === '/api/availability') value = availability;
      if (path === '/api/book') value = { booking: { meetingTitle: 'Synthetic meeting' } };
      return { ok: true, json: async () => value };
    },
  });
  vm.runInContext(source, ctx);
  const settle = () => new Promise(resolve => setImmediate(resolve));
  await settle();
  return { document, calls, ctx, settle, async event(id, name, event = {}) {
    for (const fn of document.getElementById(id).listeners[name] || []) await fn({ preventDefault() {}, ...event });
    await settle();
  }, details() { vm.runInContext('showDetails(state.availability.candidates[0])', ctx); } };
}
for (const lang of ['it', 'en']) {
  test(`${lang}: shipped flow translates, changes zones, restores named action after failure and prevents duplicate submit`, async () => {
    let finish;
    const f = await bookingFixture(lang, 'Europe/Rome', { book: () => new Promise(resolve => { finish = resolve; }) });
    const get = id => f.document.getElementById(id);
    assert.equal(f.document.querySelectorAll('.time-host').length, 0);
    get('viewer-timezone').value = 'America/New_York'; await f.event('viewer-timezone', 'change');
    assert.equal(f.document.querySelector('.time-host').textContent, lang === 'it' ? '15:00 a Roma' : '3:00 PM in Rome');
    assert.equal(get('timezone-live').textContent, lang === 'it' ? 'Orari ora mostrati nel fuso America/New York' : 'Times now shown in America/New York');
    assert.equal(get('times-date').textContent, lang === 'it' ? 'martedì 24 novembre' : 'Tuesday, November 24');
    assert.equal(get('month-label').textContent, lang === 'it' ? 'novembre 2026' : 'November 2026');
    f.details();
    assert.equal(get('date-step').classList.contains('hidden'), false, 'desktop calendar stays mounted');
    assert.equal(get('times-step').classList.contains('hidden'), true);
    assert.equal(get('guest-name').getAttribute('placeholder'), lang === 'it' ? 'Il tuo nome' : 'Your name');
    get('guest-email').value = 'nome@example.com';
    const button = get('booking-form').querySelector('button[type="submit"]');
    const label = button.textContent;
    const pending = f.event('booking-form', 'submit', { submitter: button });
    assert.equal(button.disabled, true);
    assert.equal(button.textContent, lang === 'it' ? 'Sto prenotando…' : 'Booking…');
    await f.event('booking-form', 'submit', { submitter: button });
    assert.equal(f.calls.filter(c => c.path === '/api/book').length, 1);
    finish({ ok: false, json: async () => ({ error: 'busy' }) }); await pending;
    assert.equal(button.disabled, false); assert.equal(button.textContent, label);
    assert.equal(get('date-step').inert, false);
    assert.equal(get('booking-status').textContent, lang === 'it' ? "Quell'orario non è più disponibile. Torna indietro e scegline un altro." : 'That time is no longer available. Go back and choose another.');
    const success = f.event('booking-form', 'submit', { submitter: button });
    finish({ ok: true, json: async () => ({ booking: { meetingTitle: '<Synthetic>' } }) }); await success;
    assert.equal(get('success-step').classList.contains('hidden'), false);
    assert.ok(get('success-summary').textContent.includes('nome@example.com'));
    assert.ok(get('success-summary').textContent.includes('30 min · Google Meet · America/New York'));
    assert.equal(get('confirmed-title').textContent, '<Synthetic>');
    assert.equal(f.calls.filter(c => c.path === '/api/book')[0].input.idempotencyKey, f.calls.filter(c => c.path === '/api/book')[1].input.idempotencyKey);
  });
  test(`${lang}: empty month and empty day have distinct readable states`, async () => {
    const emptyMonth = await bookingFixture(lang, 'Europe/Rome', { emptyMonth: true });
    assert.equal(emptyMonth.document.getElementById('times-date').textContent, lang === 'it' ? 'Nessuna data con orari liberi in questo mese.' : 'No dates with available times this month.');
    const emptyDay = await bookingFixture(lang, 'Europe/Rome', { emptyDay: true });
    assert.equal(emptyDay.document.querySelector('.times-empty').textContent, lang === 'it' ? 'Nessun orario libero in questo giorno: prova il prossimo' : 'No times free on this day: try the next one');
  });
}
