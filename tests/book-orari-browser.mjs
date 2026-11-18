// Run this callback with playwright-cli run-code (strip `export default`).
// Requires .local/preview.py: production CSP and reference/public asset fallback.
// All API responses, names, addresses and calendar instants below are synthetic.
export default async (page) => {
  const origin = 'http://127.0.0.1:18766';
  const output = '.local/review/book-orari';
  const productionCsp = "default-src 'self'; base-uri 'none'; connect-src 'self'; form-action 'self'; frame-ancestors 'none'; img-src 'self' data:; object-src 'none'; script-src 'self'; style-src 'self'";
  const report = { captures: [], checks: [], errors: [], cspViolations: [], csp: productionCsp, documents: 0 };
  const check = (condition, message) => { if (!condition) throw new Error(message); report.checks.push(message); };
  await page.exposeFunction('recordBookingCspViolation', violation => report.cspViolations.push(violation));
  await page.addInitScript(() => {
    document.addEventListener('securitypolicyviolation', event => {
      window.recordBookingCspViolation({ directive: event.effectiveDirective, blockedURI: event.blockedURI, disposition: event.disposition });
    });
  });
  page.on('response', response => {
    if (response.request().isNavigationRequest() && response.url().startsWith(origin + '/')) {
      report.documents++;
      check(response.headers()['content-security-policy'] === productionCsp, 'document enforces the unchanged production CSP');
    }
  });
  page.on('pageerror', error => report.errors.push(error.message));
  page.on('console', message => { if (message.type() === 'error') report.errors.push(message.text()); });
  let scenario = 'different', bookMode = 'success', bookRequests = 0;
  let releaseBooking;
  await page.clock.setFixedTime(new Date('2026-11-18T12:00:00Z'));
  await page.context().unroute('**/*');
  await page.context().route('**/*', async route => {
    const request = route.request(), url = new URL(request.url());
    if (url.origin !== origin) return route.abort();
    if (!url.pathname.startsWith('/api/')) return route.continue();
    const body = request.postDataJSON() || {};
    if (scenario === 'network' && url.pathname === '/api/available-dates') return route.abort('failed');
    const day = scenario === 'rollover' ? '2026-11-23' : '2026-11-24';
    const starts = scenario === 'rollover' ? ['2026-11-24T05:00:00Z', '2026-11-24T05:30:00Z', '2026-11-24T06:00:00Z'] : ['2026-11-24T14:00:00Z', '2026-11-24T14:30:00Z', '2026-11-24T15:00:00Z', '2026-11-24T15:30:00Z'];
    const candidates = scenario === 'emptyDay' ? [] : starts.map(start => ({ start, end: new Date(Date.parse(start) + (body.durationMinutes || 30) * 60000).toISOString() }));
    const availability = { date: day, viewerTimeZone: body.viewerTimeZone || 'America/New_York', hostTimezone: 'Europe/Rome', candidates };
    let data;
    if (url.pathname === '/api/config') data = { hostTimezone: 'Europe/Rome' };
    else if (url.pathname === '/api/available-dates') data = { dates: scenario === 'emptyMonth' ? [] : [day], firstDateAvailability: availability, recommendedDate: day };
    else if (url.pathname === '/api/availability') data = availability;
    else if (url.pathname === '/api/book') {
      bookRequests++;
      if (bookMode === 'pending') await new Promise(resolve => { releaseBooking = resolve; });
      if (bookMode === 'conflict') return route.fulfill({ status: 409, json: { error: 'busy' } });
      data = { booking: { meetingTitle: body.meetingTitle || 'Synthetic meeting', meetLink: 'https://meet.example.com/synthetic', managementUrl: '/manage#synthetic' } };
    } else throw new Error('Unexpected synthetic API request');
    await route.fulfill({ json: data });
  });
  async function prepare(lang, width, theme, kind = 'different') {
    scenario = kind;
    await page.setViewportSize({ width, height: width === 466 ? 678 : width === 669 ? 951 : 900 });
    await page.goto(`${origin}/?lang=${lang}`);
    await page.locator('#viewer-timezone').selectOption(kind === 'same' ? 'Europe/Rome' : kind === 'rollover' ? 'America/Los_Angeles' : 'America/New_York');
    await page.locator('.time-button').first().waitFor();
    await page.locator(`[data-m="${theme === 'ombra' ? 'dark' : 'light'}"]`).click();
    await page.waitForFunction(theme => document.documentElement.dataset.theme === theme, theme === 'ombra' ? 'dark' : 'light');
    check(await page.locator('html').getAttribute('lang') === lang, `${lang}/${width}/${theme}/${kind}: page language`);
    check(await page.locator('.time-host').count() === (kind === 'same' ? 0 : kind === 'rollover' ? 3 : 4), `${lang}/${width}/${theme}/${kind}: host lines`);
  }
  async function capture(lang, kind, step, width, theme) {
    await page.evaluate(() => { document.activeElement?.blur(); scrollTo(0, 0); });
    const path = `${output}/${lang}-${kind}-${step}-${width}-${theme}.png`;
    await page.screenshot({ path, fullPage: true }); report.captures.push(path);
    const fit = await page.evaluate(() => {
      const visible = el => !!el.getClientRects().length;
      const buttons = [...document.querySelectorAll('.time-button,.next-button,.confirm-button,.duration-option,.calendar-grid button,#viewer-timezone,.back-button')].filter(el => visible(el) && !el.disabled);
      return { overflow: document.documentElement.scrollWidth > innerWidth,
        smallTargets: buttons.filter(el => { const box = el.getBoundingClientRect(); return box.height < 44 || box.width < 44; }).map(el => el.className),
        columns: getComputedStyle(document.getElementById('slots')).gridTemplateColumns.split(' ').length };
    });
    check(!fit.overflow, `${lang}/${width}/${theme}/${step}: no horizontal overflow`);
    check(!fit.smallTargets.length, `${lang}/${width}/${theme}/${step}: targets at least 44px (${fit.smallTargets.join(',')})`);
    if (step === 'slot') check(fit.columns <= (width === 669 ? 2 : 1), `${lang}/${width}/${theme}: slot columns`);
    if (width === 375 || width === 466) {
      if (step === 'slot') await page.locator('#times-panel').evaluate(el => scrollTo(0, el.getBoundingClientRect().top + scrollY - 90));
      const viewport = path.replace('.png', '-viewport.png');
      await page.screenshot({ path: viewport }); report.captures.push(viewport);
    }
  }
  async function details(width) {
    await page.locator('.time-button').first().click();
    const selectedFit = await page.locator('.time-choice.selected').evaluate(el => {
      const host = el.querySelector('.time-host');
      return !host || host.getBoundingClientRect().height / parseFloat(getComputedStyle(host).lineHeight) < 1.05;
    });
    check(selectedFit, `selected host line stays on one line at ${width}`);
    await page.locator('.time-choice.selected .next-button').click();
    check(await page.locator('#details-step').isVisible(), `form visible at ${width}`);
    check(await page.locator('.calendar').isVisible() === (width === 1280), `calendar visibility in form at ${width}`);
    if (width === 1280) {
      const calendar = await page.locator('.calendar').boundingBox(), form = await page.locator('#details-step').boundingBox();
      check(form.x > calendar.x + calendar.width, 'desktop form stays in the times column');
    }
  }
  async function success(lang) {
    const summary = await page.locator('#selected-summary').innerText();
    await page.locator('#guest-name').fill('Alex Rivera');
    await page.locator('#guest-email').fill('nome@example.com');
    await page.locator('#meeting-title').fill('Synthetic meeting');
    await page.locator('.confirm-button').click();
    await page.locator('#success-step').waitFor();
    check((await page.locator('#success-summary').innerText()).startsWith(summary), `${lang}: confirmation repeats summary`);
    check((await page.locator('#success-summary').innerText()).includes('nome@example.com'), `${lang}: confirmation email`);
  }
  for (const lang of ['en', 'it']) {
    for (const width of [375, 466, 1280]) {
      await prepare(lang, width, 'chiaro');
      await capture(lang, 'diversi', 'slot', width, 'chiaro');
      if (width === 375) continue;
      await details(width); await capture(lang, 'diversi', 'form', width, 'chiaro');
      await success(lang); await capture(lang, 'diversi', 'conferma', width, 'chiaro');
    }
    await prepare(lang, 466, 'ombra');
    await capture(lang, 'diversi', 'slot', 466, 'ombra');
    await details(466); await capture(lang, 'diversi', 'form', 466, 'ombra');
    await success(lang); await capture(lang, 'diversi', 'conferma', 466, 'ombra');
    await prepare(lang, 466, 'chiaro', 'same'); await capture(lang, 'uguali', 'slot', 466, 'chiaro');
    await details(466); check(await page.locator('.summary-host').count() === 0, `${lang}: no host in equal-zone summary`);
    await success(lang); check(await page.locator('.summary-host').count() === 0, `${lang}: no host in equal-zone success`);
    await prepare(lang, 466, 'chiaro', 'rollover'); await capture(lang, 'cambio-giorno', 'slot', 466, 'chiaro');
    check((await page.locator('.time-host').first().innerText()).includes(lang === 'it' ? 'domani' : 'tomorrow'), `${lang}: rollover in slots`);
    await details(466); await capture(lang, 'cambio-giorno', 'form', 466, 'chiaro');
    await success(lang); await capture(lang, 'cambio-giorno', 'conferma', 466, 'chiaro');
  }
  await prepare('en', 1280, 'ombra'); await capture('en', 'diversi', 'slot', 1280, 'ombra');
  await details(1280); await capture('en', 'diversi', 'form', 1280, 'ombra');
  await page.locator('#back-to-calendar').click(); check(await page.locator('#times-step').isVisible(), 'Back restores list');
  await page.locator('#viewer-timezone').selectOption('Europe/Rome'); await page.waitForFunction(() => !document.querySelector('.time-host'));
  check((await page.locator('#timezone-live').innerText()).includes('Europe/Rome'), 'reverse zone change announced');
  await page.locator('#viewer-timezone').selectOption('Europe/Paris'); await page.waitForFunction(() => document.querySelector('#timezone-description').textContent.includes('Europe/Paris'));
  check(await page.locator('.time-host').count() === 0, 'Paris uses the same clock as Rome');
  await prepare('it', 669, 'chiaro'); await capture('it', 'duo-aperto', 'slot', 669, 'chiaro');
  const calendar = await page.locator('.calendar').boundingBox(), times = await page.locator('#times-panel').boundingBox();
  check(times.y >= calendar.y + calendar.height, 'Duo open has calendar above times');
  await page.setViewportSize({width:466,height:678});
  check(await page.locator('#slots').evaluate(el => getComputedStyle(el).gridTemplateColumns.split(' ').length) === 1, 'folding Duo returns to one slot column');
  await page.setViewportSize({width:1280,height:900});
  const unfolded = await page.locator('#times-panel').boundingBox();
  check(unfolded.x > (await page.locator('.calendar').boundingBox()).x, 'unfold to desktop reflows without reload');
  await prepare('en',1280,'chiaro','rollover');
  await page.locator('.time-button').first().click();
  const selectedHostLines = await page.locator('.time-choice.selected .time-host').evaluate(el => el.getBoundingClientRect().height / parseFloat(getComputedStyle(el).lineHeight));
  check(selectedHostLines < 1.05, 'selected desktop rollover keeps the host on one line');
  await capture('en','cambio-giorno','selected',1280,'chiaro');
  // Native validation, pending request and conflict recovery through real input.
  await prepare('en',466,'chiaro'); await details(466);
  const beforeInvalid = bookRequests;
  await page.locator('.confirm-button').click(); check(bookRequests === beforeInvalid, 'native form validation prevents incomplete booking');
  await page.locator('#guest-name').fill('Alex Rivera'); await page.locator('#guest-email').fill('nome@example.com');
  const namedAction = await page.locator('.confirm-button').innerText();
  bookMode = 'pending'; await page.locator('.confirm-button').click();
  await page.waitForFunction(() => document.querySelector('.confirm-button').disabled);
  check(await page.locator('.confirm-button').innerText() === 'Booking…', 'pending booking label and disabled state');
  bookMode = 'conflict'; releaseBooking();
  await page.waitForFunction(() => !document.querySelector('.confirm-button').disabled);
  check(await page.locator('.confirm-button').innerText() === namedAction, 'conflict restores named action');
  check((await page.locator('#booking-status').innerText()).includes('no longer available'), 'conflict message');
  bookMode = 'success'; await page.locator('.confirm-button').click(); await page.locator('#success-step').waitFor();
  // Distinct empty/error states and retry, also without live backend data.
  for (const kind of ['emptyMonth', 'emptyDay']) {
    scenario = kind; await page.goto(`${origin}/?lang=it`);
    await page.waitForFunction(() => !document.querySelector('#times-date').textContent.includes('controllando'));
    check((await page.locator(kind === 'emptyMonth' ? '#times-date' : '.times-empty').innerText()).includes(kind === 'emptyMonth' ? 'Nessuna data' : 'Nessun orario'), `${kind}: translated empty state`);
  }
  scenario = 'network'; await page.goto(`${origin}/?lang=it`); await page.locator('#retry-dates').waitFor();
  check((await page.locator('#availability-status').innerText()).includes('Disponibilità non disponibile'), 'network failure translated');
  scenario = 'different'; await page.locator('#retry-dates').click(); await page.locator('.time-button').first().waitFor();
  check(await page.locator('.time-button').count() > 0, 'retry recovers from network failure');
  // Let queued securitypolicyviolation events reach the Node-side binding.
  await page.evaluate(() => new Promise(resolve => setTimeout(resolve, 0)));
  check(report.documents > 0, 'CSP header was verified on real document responses');
  check(report.cspViolations.length === 0, 'zero securitypolicyviolation events throughout the booking flow');
  // Resource errors are expected only for the injected 409 and network abort.
  check(report.errors.every(error => error.includes('ERR_FAILED') || error.includes('409 (Conflict)')), 'no unexpected page or console errors');
  return report;
}
