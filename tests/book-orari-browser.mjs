// Run this callback with playwright-cli run-code (strip `export default`).
// Requires .local/preview.py: production CSP and reference/public asset fallback.
// All API responses, names, addresses and calendar instants below are synthetic.
// Pass { captureScreenshots: false } to validate without replacing the review pack.
// Pass { round5Only: true } for desktop line-fit, focus and r2 capture regressions.
export default async (page, { captureScreenshots = true, round5Only = false } = {}) => {
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
    const day = scenario === 'wednesday' ? '2026-11-25' : scenario === 'rollover' ? '2026-11-23' : '2026-11-24';
    const starts = scenario === 'wednesday' ? ['2026-11-25T15:30:00Z'] : scenario === 'rollover' ? ['2026-11-24T05:00:00Z', '2026-11-24T05:30:00Z', '2026-11-24T06:00:00Z'] : ['2026-11-24T14:00:00Z', '2026-11-24T14:30:00Z', '2026-11-24T15:00:00Z', '2026-11-24T15:30:00Z'];
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
    await page.waitForFunction(() => !document.documentElement.classList.contains('jk-th-fade'));
    check(await page.locator('html').getAttribute('lang') === lang, `${lang}/${width}/${theme}/${kind}: page language`);
    check(await page.locator('.time-host').count() === (kind === 'same' ? 0 : kind === 'wednesday' ? 1 : kind === 'rollover' ? 3 : 4), `${lang}/${width}/${theme}/${kind}: host lines`);
  }
  async function frameSlots() {
    await page.locator('#times-panel').evaluate(el => {
      const headerBottom = document.querySelector('.jk-top').getBoundingClientRect().bottom;
      scrollTo(0, el.getBoundingClientRect().top + scrollY - headerBottom - 8);
    });
  }
  async function capture(lang, kind, step, width, theme) {
    await page.evaluate(() => document.fonts.ready);
    const clocks = await page.evaluate(() => {
      const results = [];
      for (const element of document.querySelectorAll('.time-guest,.time-host,.summary-guest,.summary-host,.confirm-button')) {
        if (!element.getClientRects().length) continue;
        const walker = document.createTreeWalker(element, NodeFilter.SHOW_TEXT);
        while (walker.nextNode()) {
          const node = walker.currentNode;
          for (const match of node.textContent.matchAll(/\d{1,2}:\d{2}(?:\s+[AP]M)?/g)) {
            const range = document.createRange();
            range.setStart(node, match.index); range.setEnd(node, match.index + match[0].length);
            const rects = [...range.getClientRects()].filter(rect => rect.width && rect.height);
            results.push({ element: element.className, text: match[0], tops: [...new Set(rects.map(rect => rect.top))] });
          }
        }
      }
      return results;
    });
    const wrapped = clocks.filter(clock => clock.tops.length !== 1);
    check(clocks.length > 0 && !wrapped.length, `${lang}/${width}/${theme}/${step}: every guest/host clock stays on one line${wrapped.length ? ` (${JSON.stringify(wrapped)})` : ''}`);
    if (lang === 'en' && width === 1280 && kind === 'diversi' && step === 'form') {
      check(clocks.some(clock => clock.element.includes('confirm-button') && clock.text === '9:00\u00a0AM' && clock.tops.length === 1), `1280/${theme}: confirm button keeps 9:00 AM on one line`);
    }
    await page.evaluate(() => { document.activeElement?.blur(); scrollTo(0, 0); });
    const path = `${output}/${lang}-${kind}-${step}-${width}-${theme}.png`;
    if (captureScreenshots) {
      await page.screenshot({ path, fullPage: true }); report.captures.push(path);
    }
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
      if (step === 'slot') await frameSlots();
      const viewport = path.replace('.png', '-viewport.png');
      if (captureScreenshots) {
        await page.screenshot({ path: viewport }); report.captures.push(viewport);
      }
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
  if (round5Only) {
    const failures = [];
    report.measurements = { shots: [], pills: [], longestWeekday: [], slotFrames: [] };
    const verify = (condition, message) => {
      report.checks.push(`${condition ? 'PASS' : 'FAIL'}: ${message}`);
      if (!condition) failures.push(message);
    };
    async function textGeometry(selector) {
      return page.locator(selector).evaluate(element => {
        const range = document.createRange();
        range.selectNodeContents(element);
        const rects = [...range.getClientRects()].filter(rect => rect.width && rect.height);
        return { text: element.textContent, lineCount: new Set(rects.map(rect => rect.top)).size,
          width: element.getBoundingClientRect().width, fontSize: getComputedStyle(element).fontSize };
      });
    }
    async function layoutGeometry() {
      return page.evaluate(() => {
        const box = selector => document.querySelector(selector).getBoundingClientRect();
        const grid = box('.date-time-grid'), panel = box('#times-panel');
        return { bookingContentWidth: box('.jk-cb').width,
          gridWidth: grid.width, calendarColumnWidth: box('.calendar').width,
          timesFormColumnWidth: panel.width, detailsPanelWidth: box('#details-step').width,
          formPanelWidth: box('#details-step .book-form').width,
          leftoverGap: document.querySelector('.jk-cb').getBoundingClientRect().right - panel.right };
      });
    }
    for (const width of [375, 466, 1280]) for (const theme of ['chiaro', 'ombra']) {
      await prepare('it', width, theme);
      // Persist the real switch choice, then inspect a fresh load before any input.
      await page.reload();
      await page.locator('.time-button').first().waitFor();
      await page.evaluate(() => document.fonts.ready);
      const pill = await page.locator('a.skip-link').evaluate(element => {
        const rect = element.getBoundingClientRect(), css = getComputedStyle(element);
        const intersection = Math.max(0, Math.min(innerWidth, rect.right) - Math.max(0, rect.left)) *
          Math.max(0, Math.min(innerHeight, rect.bottom) - Math.max(0, rect.top));
        const clipped = css.clip === 'rect(0px, 0px, 0px, 0px)' || css.clipPath === 'inset(50%)';
        const selected = document.querySelector('.calendar-grid button.selected');
        const calendarRect = selected.getBoundingClientRect();
        return { selector: 'body.booking-page > a.skip-link', boundingBox: rect.toJSON(),
          visibleArea: clipped ? 0 : intersection, unclippedIntersectionArea: intersection,
          position: css.position, transform: css.transform, clip: css.clip, clipPath: css.clipPath,
          activeElement: document.activeElement.tagName.toLowerCase(), scrollY,
          theme: document.documentElement.dataset.theme,
          calendarSelectionOnLoad: { selector: '.calendar-grid button.recommended.selected',
            text: selected.textContent, boundingBox: calendarRect.toJSON(),
            position: getComputedStyle(selected).position, transform: getComputedStyle(selected).transform,
            visibleArea: Math.max(0, Math.min(innerWidth, calendarRect.right) - Math.max(0, calendarRect.left)) *
              Math.max(0, Math.min(innerHeight, calendarRect.bottom) - Math.max(0, calendarRect.top)) } };
      });
      report.measurements.pills.push({ width, themeName: theme, ...pill });
      verify(pill.activeElement === 'body' && pill.scrollY === 0, `${width}/${theme}: no autofocus or scroll on load`);
      verify(pill.visibleArea === 0, `${width}/${theme}: skip link has zero visible area on load (${pill.visibleArea})`);
      verify(pill.boundingBox.width === 1 && pill.boundingBox.height === 1 && pill.clipPath === 'inset(50%)',
        `${width}/${theme}: unfocused skip link uses the clipped 1px pattern`);
      verify(pill.theme === (theme === 'ombra' ? 'dark' : 'light'), `${width}/${theme}: real theme persists on reload`);
      if (captureScreenshots && width === 466) {
        const path = `${output}/r2/top-466-${theme}.png`;
        await page.screenshot({ path }); report.captures.push(path);
      }
      await page.keyboard.press('Tab');
      const focused = await page.locator('a.skip-link').evaluate(element => {
        const rect = element.getBoundingClientRect(), css = getComputedStyle(element);
        return { active: document.activeElement === element, x: rect.x, y: rect.y,
          width: rect.width, height: rect.height, right: rect.right, bottom: rect.bottom,
          clip: css.clip, clipPath: css.clipPath, viewportWidth: innerWidth, viewportHeight: innerHeight };
      });
      report.measurements.pills.at(-1).afterTab = focused;
      verify(focused.active && focused.width > 44 && focused.height >= 44 && focused.x >= 0 && focused.y >= 0 &&
        focused.right <= width && focused.bottom <= focused.viewportHeight && focused.clip === 'auto' && focused.clipPath === 'none',
        `${width}/${theme}: first Tab reveals the whole skip link`);
      await page.keyboard.press('Enter');
      verify(await page.evaluate(() => location.hash === '#scheduler'), `${width}/${theme}: skip link reaches booking`);
    }
    for (const theme of ['chiaro', 'ombra']) {
      await prepare('en', 466, theme);
      await page.evaluate(() => document.activeElement.blur());
      await frameSlots();
      const frame = await page.locator('.calendar-grid button.selected').evaluate(element => ({
        selector: '.calendar-grid button.recommended.selected', text: element.textContent,
        boundingBox: element.getBoundingClientRect().toJSON(), position: getComputedStyle(element).position,
        transform: getComputedStyle(element).transform, scrollY,
      }));
      report.measurements.slotFrames.push({ theme, ...frame });
      verify(frame.boundingBox.bottom <= 0, `466/${theme}: scrolled slot capture has no selected-calendar pill at the top (${frame.boundingBox.bottom})`);
    }
    for (const kind of ['different', 'wednesday']) for (const lang of ['it', 'en']) for (const theme of ['chiaro', 'ombra']) {
      await prepare(lang, 1280, theme, kind);
      await details(1280);
      await page.evaluate(() => document.fonts.ready);
      const button = await textGeometry('.confirm-button');
      const formDate = await textGeometry('#selected-summary .summary-guest');
      const layout = await layoutGeometry();
      const key = `${lang}/1280/${theme}/${kind}`;
      verify(button.lineCount === 1, `${key}: whole confirm action occupies one line (${button.lineCount})`);
      verify(formDate.lineCount === 1, `${key}: selected date occupies one line (${formDate.lineCount})`);
      verify(Math.abs(layout.leftoverGap) < 1, `${key}: times/form uses the booking content width (${layout.leftoverGap}px leftover)`);
      const expectedAction = lang === 'it' ? (kind === 'wednesday' ? 'Prenota mercoledì alle 10:30' : 'Prenota martedì alle 09:00') :
        (kind === 'wednesday' ? 'Book Wednesday at 10:30 AM' : 'Book Tuesday at 9:00 AM');
      verify(button.text.replaceAll('\u00a0', ' ') === expectedAction, `${key}: exact fixture action`);
      const base = { lang, theme, buttonLineCount: button.lineCount, buttonWidth: button.width,
        buttonText: button.text, selectedDateLineCount: formDate.lineCount, selectedDateText: formDate.text, ...layout };
      const formPath = `${output}/r2/${lang}-diversi-form-1280-${theme}.png`;
      if (captureScreenshots && kind === 'different') {
        await page.evaluate(() => { document.activeElement.blur(); scrollTo(0, 0); });
        await page.screenshot({ path: formPath, fullPage: true }); report.captures.push(formPath);
      }
      await success(lang);
      const date = await textGeometry('#success-summary .summary-guest');
      verify(date.lineCount === 1, `${key}: confirmation date occupies one line (${date.lineCount})`);
      verify(date.text === formDate.text, `${key}: confirmation preserves the full date`);
      const successPath = `${output}/r2/${lang}-diversi-conferma-1280-${theme}.png`;
      if (kind === 'different') {
        report.measurements.shots.push({ path: formPath, step: 'form', ...base, confirmationDateLineCount: date.lineCount });
        report.measurements.shots.push({ path: successPath, step: 'conferma', ...base,
          confirmationDateLineCount: date.lineCount, confirmationDateText: date.text,
          layoutMeasurementState: 'preceding form; calendar, times panel and button are hidden on confirmation' });
        if (captureScreenshots) {
          await page.evaluate(() => { document.activeElement.blur(); scrollTo(0, 0); });
          await page.screenshot({ path: successPath, fullPage: true }); report.captures.push(successPath);
        }
      } else report.measurements.longestWeekday.push({ ...base, confirmationDateLineCount: date.lineCount, confirmationDateText: date.text });
    }
    await page.evaluate(() => new Promise(resolve => setTimeout(resolve, 0)));
    check(report.cspViolations.length === 0 && report.errors.length === 0, 'Round 5: zero CSP violations and page/console errors');
    check(!failures.length, `Round 5 regressions: ${JSON.stringify({ failures, measurements: report.measurements })}`);
    return report;
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
  await success('en'); await capture('en', 'diversi', 'conferma', 1280, 'ombra');
  await prepare('en', 1280, 'ombra'); await details(1280);
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
  const beforePending = bookRequests;
  bookMode = 'pending'; await page.locator('.confirm-button').click();
  await page.waitForFunction(() => document.querySelector('.confirm-button').disabled);
  check(await page.locator('.confirm-button').innerText() === 'Booking…', 'pending booking label and disabled state');
  check(await page.locator('#booking-status').evaluate(el => !el.closest('[inert]')), 'pending status has no inert ancestor');
  check((await page.locator('#booking-status').ariaSnapshot()).includes('Rechecking availability'), 'pending status is exposed to assistive technology');
  await page.locator('#booking-form').evaluate(form => form.dispatchEvent(new SubmitEvent('submit', { bubbles: true, cancelable: true })));
  check(bookRequests === beforePending + 1, 'second submit during pending does not send another booking request');
  bookMode = 'conflict'; releaseBooking();
  await page.waitForFunction(() => !document.querySelector('.confirm-button').disabled);
  check(await page.locator('.confirm-button').innerText() === namedAction, 'conflict restores named action');
  check((await page.locator('#booking-status').innerText()).includes('no longer available'), 'conflict message');
  check(await page.locator('#booking-status').evaluate(el => !el.closest('[inert]')), 'conflict status has no inert ancestor');
  check((await page.locator('#booking-status').ariaSnapshot()).includes('no longer available'), 'conflict status is exposed to assistive technology');
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
