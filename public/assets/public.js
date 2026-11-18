// Booking presentation only: API instants and availability decisions stay unchanged.
function bookingText(lang, key, values = {}) {
  const strings = {
    home: ['jaack.me booking home', 'jaack.me, pagina di prenotazione'],
    chooseDate: ['Choose a date', 'Scegli una data'],
    previousMonth: ['Previous month', 'Mese precedente'], nextMonth: ['Next month', 'Mese successivo'],
    namePlaceholder: ['Your name', 'Il tuo nome'], emailPlaceholder: ['you@example.com', 'nome@example.com'],
    titlePlaceholder: ['What would you like to discuss?', 'Di cosa ti piacerebbe parlare?'],
    purpose: ['What is this for?', 'Di che si tratta?'],
    timezone: ['Times in your time zone: {zone}', 'Orari nel tuo fuso: {zone}'],
    timezoneLive: ['Times now shown in {zone}', 'Orari ora mostrati nel fuso {zone}'],
    next: ['Next', 'Avanti'], nextLabel: ['Next, continue with {weekday} at {time}', 'Avanti, continua con {weekday} alle {time}'],
    action: ['Book {weekday} at {time}', 'Prenota {weekday} alle {time}'],
    booking: ['Booking…', 'Sto prenotando…'],
    loadingTimes: ['Looking up times…', 'Sto cercando gli orari…'],
    emptyDay: ['No times free on this day: try the next one', 'Nessun orario libero in questo giorno: prova il prossimo'],
    loadingDates: ['Checking available dates…', 'Sto controllando le date…'],
    emptyMonth: ['No dates with available times this month.', 'Nessuna data con orari liberi in questo mese.'],
    partial: ['Some dates could not be checked. Try again later.', 'Non sono riuscito a controllare tutte le date. Riprova tra poco.'],
    unavailable: ['Availability unavailable', 'Disponibilità non disponibile'],
    expired: ['Availability needs refreshing. Select a date to check again.', 'La disponibilità va aggiornata. Scegli di nuovo una data.'],
    slotGone: ['That time just became unavailable. Choose another time.', "Quell'orario non è più libero. Scegline un altro."],
    dateGone: ['That date just became unavailable. Choose another date.', "Quella data non è più libera. Scegline un'altra."],
    refreshFailed: ['Could not refresh times yet. Booking will recheck your selection.', 'Non sono ancora riuscito ad aggiornare gli orari. Al momento della prenotazione ricontrollo la tua scelta.'],
    submitting: ['Rechecking availability and creating your event…', "Sto ricontrollando la disponibilità e creando l'evento…"],
    conflict: ['That time is no longer available. Go back and choose another.', "Quell'orario non è più disponibile. Torna indietro e scegline un altro."],
    noTimesAria: [', no available times', ', nessun orario libero'], suggestedAria: [', suggested date', ', data consigliata'],
    email: ['Check the email sent to {email}', "Controlla l'email a {email}"],
    demo: ['Demo mode', 'Modalità demo'], demoAvailability: ['Demo availability', 'Disponibilità demo'],
    tomorrow: ['tomorrow', 'domani'], yesterday: ['yesterday', 'ieri'],
  };
  return strings[key][lang === 'it' ? 1 : 0].replace(/\{(\w+)\}/g, (_, name) => values[name] ?? '');
}
function bookingTime(iso, viewerZone, hostZone, lang, duration = 30) {
  const locale = lang === 'it' ? 'it-IT' : 'en-US';
  const format = (zone, options) => new Intl.DateTimeFormat(locale, { timeZone: zone, ...options }).format(new Date(iso));
  const clock = (zone) => format(zone, lang === 'it' ? { hour: '2-digit', minute: '2-digit', hourCycle: 'h23' } : { hour: 'numeric', minute: '2-digit', hour12: true });
  const date = (zone, year = false) => format(zone, { weekday: 'long', month: 'long', day: 'numeric', ...(year ? { year: 'numeric' } : {}) });
  const weekday = (zone) => format(zone, { weekday: 'long' });
  const parts = (zone) => Object.fromEntries(new Intl.DateTimeFormat('en-CA', { timeZone: zone, year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', hourCycle: 'h23' }).formatToParts(new Date(iso)).map(p => [p.type, p.value]));
  const guestParts = parts(viewerZone), hostParts = hostZone ? parts(hostZone) : guestParts;
  const calendarDay = (p) => Date.UTC(Number(p.year), Number(p.month) - 1, Number(p.day));
  const dayDiff = (calendarDay(hostParts) - calendarDay(guestParts)) / 86_400_000;
  const different = !!hostZone && (dayDiff !== 0 || hostParts.hour !== guestParts.hour || hostParts.minute !== guestParts.minute);
  const zoneName = (zone) => zone.replaceAll('_', ' ');
  const place = hostZone === 'Europe/Rome' ? (lang === 'it' ? 'Roma' : 'Rome') : zoneName(hostZone || viewerZone);
  const guestPlace = viewerZone.split('/').at(-1).replaceAll('_', ' ');
  const hostPlace = place;
  const guest = clock(viewerZone), hostTime = different ? clock(hostZone) : '';
  const inPlace = lang === 'it' ? `a ${place}` : `in ${place}`;
  const relativeDay = dayDiff === 1 ? bookingText(lang, 'tomorrow') : dayDiff === -1 ? bookingText(lang, 'yesterday') : different ? weekday(hostZone) : '';
  const host = different ? `${hostTime}${dayDiff ? `, ${relativeDay}` : ''} ${inPlace}` : '';
  const summaryHost = different ? `${hostTime}${dayDiff ? `, ${date(hostZone)}` : ''} ${inPlace}` : '';
  let slotLabel = `${date(viewerZone)}, ${guest}`;
  if (different) {
    slotLabel += lang === 'it'
      ? ` ora di ${guestPlace}, ${hostTime}${dayDiff ? ` di ${weekday(hostZone)}` : ''} ora di ${hostPlace}`
      : ` ${guestPlace} time, ${hostTime}${dayDiff ? ` ${weekday(hostZone)}` : ''} ${hostPlace} time`;
  }
  const values = { weekday: weekday(viewerZone), time: guest };
  return { guest, host, slotLabel, summaryHost, summaryGuest: `${date(viewerZone, true)}, ${guest}`,
    summaryMeta: `${duration} min · Google Meet · ${zoneName(viewerZone)}`,
    actionLabel: bookingText(lang, 'action', values), nextLabel: bookingText(lang, 'nextLabel', values) };
}
function renderTimeSummary(document, element, value, email = '') {
  element.replaceChildren();
  for (const [className, text] of [['summary-guest', value.summaryGuest], ['summary-host', value.summaryHost], ['summary-meta', value.summaryMeta], ['summary-email', email]]) {
    if (!text) continue;
    const line = document.createElement(className === 'summary-guest' ? 'strong' : 'span');
    line.className = className; line.textContent = text; element.append(line);
  }
}

const $ = (id) => document.getElementById(id);
const language = document.documentElement?.lang === 'it' ? 'it' : 'en';
const locale = language === 'it' ? 'it-IT' : 'en-US';
const t = (key, values) => bookingText(language, key, values);
const selectedTime = (iso) => bookingTime(iso, state.viewerTimeZone, state.availability?.hostTimezone || state.hostTimezone, language, state.durationMinutes);
function updateTimezoneDescription() { $('timezone-description').textContent = t('timezone', { zone: zoneLabel(state.viewerTimeZone) }); }
function showLoadingTimes() { $('slots').innerHTML = `<p class="times-empty">${t('loadingTimes')}</p><div class="loading-times" aria-hidden="true"><i></i><i></i><i></i></div>`; }
for (const element of document.querySelectorAll('[data-booking-label]')) element.setAttribute('aria-label', t(element.dataset.bookingLabel));
for (const element of document.querySelectorAll('[data-booking-placeholder]')) element.setAttribute('placeholder', t(element.dataset.bookingPlaceholder));

function detectedTimeZone() {
  const zone = Intl.DateTimeFormat().resolvedOptions().timeZone;
  return zone || 'UTC';
}
function dateKeyInZone(value, timeZone) {
  const parts = Object.fromEntries(new Intl.DateTimeFormat('en-CA', { timeZone, year: 'numeric', month: '2-digit', day: '2-digit' }).formatToParts(new Date(value)).filter((part) => part.type !== 'literal').map((part) => [part.type, part.value]));
  return `${parts.year}-${parts.month}-${parts.day}`;
}
function parseDateKey(value) {
  const [year, month, day] = value.split('-').map(Number);
  return new Date(year, month - 1, day, 12);
}
function localDateKey(date) {
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, '0');
  const day = String(date.getDate()).padStart(2, '0');
  return `${year}-${month}-${day}`;
}
function startOfMonth(date) { return new Date(date.getFullYear(), date.getMonth(), 1, 12); }
function zoneLabel(zone) { return zone.replaceAll('_', ' '); }
function populateTimeZones(selected) {
  const fallback = ['UTC', 'Europe/Rome', 'Europe/London', 'America/New_York', 'America/Los_Angeles', 'Asia/Dubai', 'Asia/Singapore', 'Asia/Tokyo', 'Australia/Sydney'];
  const supported = typeof Intl.supportedValuesOf === 'function' ? Intl.supportedValuesOf('timeZone') : fallback;
  const zones = [...new Set(['UTC', selected, ...supported])].filter(Boolean).sort((a, b) => a.localeCompare(b));
  const select = $('viewer-timezone'); select.replaceChildren();
  for (const zone of zones) {
    const option = document.createElement('option'); option.value = zone; option.textContent = zoneLabel(zone); option.selected = zone === selected; select.append(option);
  }
}

const viewerTimeZone = detectedTimeZone();
const state = {
  durationMinutes: 30,
  hostTimezone: null,
  submitting: false,
  viewerTimeZone,
  visibleMonth: startOfMonth(parseDateKey(dateKeyInZone(Date.now(), viewerTimeZone))),
  selectedDate: null,
  choice: null,
  availability: null,
  availabilityRequest: 0,
  availableDates: null,
  recommendedDate: null,
  monthRequest: 0,
  bookingAttempt: null,
  availabilityCache: new Map(),
};

function dayLabel(value) { return new Intl.DateTimeFormat(locale, { weekday: 'long', month: 'long', day: 'numeric', timeZone: 'UTC' }).format(new Date(`${value}T12:00:00Z`)); }

async function request(path, options) {
  const response = await fetch(path, { ...options, headers: { ...(options?.headers || {}), 'X-Jev-Booking': '1' } });
  const data = await response.json();
  if (!response.ok) throw Object.assign(new Error(data.error || 'Request failed'), {invalidatePreview: Boolean(data.invalidatePreview)});
  return data;
}

function setStatus(element, text = '', error = false) {
  element.textContent = text;
  element.className = `inline-status${text ? '' : ' hidden'}${error ? ' error' : ''}`;
}

function renderCalendar() {
  const month = state.visibleMonth;
  $('month-label').textContent = new Intl.DateTimeFormat(locale, { month: 'long', year: 'numeric' }).format(month);
  const grid = $('calendar-grid'); grid.replaceChildren();
  const mondayOffset = (month.getDay() + 6) % 7;
  const lastDay = new Date(month.getFullYear(), month.getMonth() + 1, 0).getDate();
  const today = parseDateKey(dateKeyInZone(Date.now(), state.viewerTimeZone));
  const maxDate = new Date(today); maxDate.setDate(maxDate.getDate() + 90);
  grid.setAttribute('aria-busy', String(state.availableDates === null));

  for (let i = 0; i < mondayOffset; i += 1) grid.append(document.createElement('span'));
  for (let day = 1; day <= lastDay; day += 1) {
    const date = new Date(month.getFullYear(), month.getMonth(), day, 12);
    const key = localDateKey(date);
    const button = document.createElement('button');
    button.type = 'button'; button.textContent = String(day); button.dataset.date = key;
    const inRange = date >= today && date <= maxDate;
    const unavailable = inRange && state.availableDates !== null && !state.availableDates.has(key);
    button.setAttribute('aria-label', `${dayLabel(key)}${unavailable ? t('noTimesAria') : key === state.recommendedDate ? t('suggestedAria') : ''}`);
    const disabled = !inRange || !state.availableDates?.has(key);
    button.disabled = disabled;
    if (key === localDateKey(today)) button.classList.add('today');
    if (!disabled && key === state.recommendedDate) button.classList.add('recommended');
    if (key === state.selectedDate) { button.classList.add('selected'); button.setAttribute('aria-current', 'date'); }
    if (!disabled) button.addEventListener('click', () => { showCalendar(); selectDate(key); });
    grid.append(button);
  }
  $('previous-month').disabled = month <= startOfMonth(today);
  $('next-month').disabled = new Date(month.getFullYear(), month.getMonth() + 1, 1, 12) > maxDate;
}

const AVAILABILITY_CACHE_MS = 60_000;
const AVAILABILITY_MAX_AGE_MS = 120_000;
function cacheAvailability(key, value, startedAt) {
  const sourceAge = Math.max(0, Number(value.snapshotAgeMs) || 0);
  const maxAge = Math.min(AVAILABILITY_MAX_AGE_MS, Number(value.previewMaxAgeMs) || AVAILABILITY_CACHE_MS);
  const entry = {value, expiresAt: startedAt + Math.max(0, AVAILABILITY_CACHE_MS - sourceAge), validUntil: startedAt + Math.max(0, maxAge - sourceAge)};
  state.availabilityCache.set(key, entry);
  return entry;
}
function clearAvailabilityTimers() {
  clearTimeout(state.availabilityExpiry);
  clearTimeout(state.availabilityRecheck);
}
function expireAvailability(key, entry, requestId) {
  clearAvailabilityTimers();
  state.availabilityRecheck = setTimeout(() => {
    if (document.visibilityState !== 'visible' || state.availabilityCache.get(key) !== entry || requestId !== state.availabilityRequest) return;
    selectDate(state.selectedDate, true);
  }, Math.max(5_000, Math.min(60_000, entry.validUntil - performance.now() - 5_000)));
  state.availabilityExpiry = setTimeout(() => {
    if (state.availabilityCache.get(key) !== entry || requestId !== state.availabilityRequest) return;
    state.availabilityCache.delete(key);
    state.availability = null;
    state.choice = null;
    $('slots').replaceChildren();
    showCalendar();
    setStatus($('availability-status'), t('expired'));
    $('retry-dates').classList.remove('hidden');
  }, Math.max(0, entry.validUntil - performance.now()));
}
function availabilityKey(date) {
  return JSON.stringify([date, state.durationMinutes, state.viewerTimeZone]);
}

async function loadMonth(preferredDate = null) {
  const requestId = ++state.monthRequest;
  const startedAt = performance.now();
  clearAvailabilityTimers();
  state.availabilityCache.clear();
  ++state.availabilityRequest;
  state.availableDates = null; state.recommendedDate = null; state.selectedDate = null; state.choice = null; state.availability = null;
  renderCalendar();
  $('retry-dates').classList.add('hidden');
  setStatus($('availability-status'), t('loadingDates'));
  $('times-date').textContent = t('loadingDates');
  showLoadingTimes();
  try {
    const month = localDateKey(state.visibleMonth).slice(0, 7);
    const result = await request('/api/available-dates', {
      method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ month, durationMinutes: state.durationMinutes, viewerTimeZone: state.viewerTimeZone }),
    });
    if (requestId !== state.monthRequest) return;
    state.availableDates = new Set(result.dates);
    for (const date of result.dates) {
      const availability = result.availabilityByDate?.[date] || (result.firstDateAvailability?.date === date ? result.firstDateAvailability : null);
      if (availability) cacheAvailability(availabilityKey(date), availability, startedAt);
    }
    state.recommendedDate = result.recommendedDate && state.availableDates.has(result.recommendedDate) ? result.recommendedDate : null;
    renderCalendar();
    if (!result.dates.length) {
      setStatus($('availability-status'), result.partial ? t('partial') : t('emptyMonth'), Boolean(result.partial));
      $('retry-dates').classList.remove('hidden');
      $('times-date').textContent = t('emptyMonth');
      $('slots').replaceChildren();
      return;
    }
    setStatus($('availability-status'));
    const firstDate = preferredDate && state.availableDates.has(preferredDate) ? preferredDate : result.dates[0];
    await selectDate(firstDate);
    if (result.partial && requestId === state.monthRequest) setStatus($('availability-status'), t('partial'));
  } catch (error) {
    if (requestId !== state.monthRequest) return;
    setStatus($('availability-status'), error instanceof TypeError ? t('unavailable') : `${t('unavailable')}: ${error.message}`, true);
    $('retry-dates').classList.remove('hidden');
    $('times-date').textContent = t('unavailable');
    $('slots').replaceChildren();
  }
}

async function selectDate(date, preserveChoice = false) {
  if (!state.availableDates?.has(date)) return;
  const requestId = ++state.availabilityRequest;
  const startedAt = performance.now();
  const key = availabilityKey(date);
  const cached = state.availabilityCache.get(key);
  const recent = cached?.validUntil > startedAt;
  clearAvailabilityTimers();
  setStatus($('availability-status'));
  state.selectedDate = date; if (!preserveChoice) state.choice = null; state.availability = recent ? cached.value : null; renderCalendar();
  $('times-date').textContent = dayLabel(date);
  if (recent) {
    $('viewer-timezone').value = cached.value.viewerTimeZone;
    state.hostTimezone = cached.value.hostTimezone || state.hostTimezone;
    renderSlots();
    expireAvailability(key, cached, requestId);
    if (cached.expiresAt > startedAt && !preserveChoice) return;
  } else {
    showLoadingTimes();
  }
  try {
    const availability = await request('/api/availability', {
      method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ date, durationMinutes: state.durationMinutes, viewerTimeZone: state.viewerTimeZone }),
    });
    if (requestId !== state.availabilityRequest) return;
    const entry = cacheAvailability(key, availability, startedAt);
    if (entry.validUntil <= performance.now()) throw new Error(t('expired'));
    const choice = state.choice;
    if (choice && !availability.candidates.some(slot => slot.start === choice.start && slot.end === choice.end)) {
      state.choice = null;
      showCalendar();
      setStatus($('availability-status'), t('slotGone'));
    }
    const candidatesChanged = JSON.stringify(state.availability?.candidates) !== JSON.stringify(availability.candidates) || state.availability?.hostTimezone !== availability.hostTimezone;
    state.availability = availability;
    state.viewerTimeZone = availability.viewerTimeZone;
    $('viewer-timezone').value = state.viewerTimeZone;
    state.hostTimezone = availability.hostTimezone || state.hostTimezone;
    updateTimezoneDescription();
    if (!availability.candidates.length) {
      state.availableDates.delete(date);
      if (state.recommendedDate === date) state.recommendedDate = null;
      state.selectedDate = null;
      renderCalendar();
      setStatus($('availability-status'), t('dateGone'));
      if (!state.availableDates.size) $('retry-dates').classList.remove('hidden');
    }
    // Preserve selections and keyboard focus when the background result is unchanged.
    if (candidatesChanged) renderSlots();
    expireAvailability(key, entry, requestId);
  } catch (error) {
    if (requestId !== state.availabilityRequest) return;
    if (error.invalidatePreview) {
      clearAvailabilityTimers();
      state.availabilityCache.clear();
      state.choice = null;
      showCalendar();
    }
    if (!error.invalidatePreview && recent && cached.validUntil > performance.now()) {
      setStatus($('availability-status'), t('refreshFailed'));
      return;
    }
    state.availabilityCache.delete(key);
    state.availability = null; state.choice = null;
    $('slots').replaceChildren();
    setStatus($('availability-status'), error instanceof TypeError ? t('unavailable') : error.message, true);
    $('retry-dates').classList.remove('hidden');
  }
}

function renderSlots() {
  const { candidates } = state.availability;
  const list = $('slots'); list.replaceChildren();
  if (!candidates.length) {
    const empty = document.createElement('p'); empty.className = 'times-empty'; empty.textContent = t('emptyDay'); list.append(empty); return;
  }
  for (const slot of candidates) {
    const row = document.createElement('div'); row.className = 'time-choice';
    const value = selectedTime(slot.start);
    const select = document.createElement('button'); select.type = 'button'; select.className = 'time-button'; select.setAttribute('aria-label', value.slotLabel);
    for (const [className, text] of [['time-guest', value.guest], ['time-host', value.host]]) {
      if (!text) continue;
      const line = document.createElement('span'); line.className = className; line.textContent = text; line.setAttribute('aria-hidden', 'true'); select.append(line);
    }
    select.setAttribute('aria-pressed', String(state.choice?.start === slot.start && state.choice?.end === slot.end));
    const next = document.createElement('button'); next.type = 'button'; next.className = 'next-button'; next.textContent = t('next'); next.setAttribute('aria-label', value.nextLabel); next.setAttribute('aria-hidden', 'true'); next.tabIndex = -1; next.disabled = true;
    select.addEventListener('click', () => activateTime(row, slot));
    next.addEventListener('click', () => showDetails(slot));
    if (state.choice?.start === slot.start && state.choice?.end === slot.end) {
      row.classList.add('selected'); next.disabled = false; next.tabIndex = 0; next.setAttribute('aria-hidden', 'false');
    }
    row.append(select, next); list.append(row);
  }
  $('demo-note').textContent = state.availability.demo ? t('demoAvailability') : '';
}

function activateTime(row, slot) {
  state.choice = slot;
  document.querySelectorAll('.time-choice').forEach((item) => {
    item.classList.remove('selected');
    item.querySelector('.time-button').setAttribute('aria-pressed', 'false');
    const next = item.querySelector('.next-button'); next.disabled = true; next.tabIndex = -1; next.setAttribute('aria-hidden', 'true');
  });
  row.classList.add('selected');
  row.querySelector('.time-button').setAttribute('aria-pressed', 'true');
  const next = row.querySelector('.next-button'); next.disabled = false; next.tabIndex = 0; next.setAttribute('aria-hidden', 'false'); next.focus();
}

function renderBookingTypes(types) {
  if (!Array.isArray(types) || !types.length) return;
  const group = document.createElement('fieldset'); group.className = 'booking-types';
  const legend = document.createElement('legend'); legend.textContent = t('purpose'); group.append(legend);
  for (const type of types) {
    const label = document.createElement('label');
    const radio = document.createElement('input');
    radio.type = 'radio'; radio.name = 'bookingType'; radio.value = type.id; radio.required = true;
    radio.checked = types.length === 1;
    const text = document.createElement('span'); text.textContent = type.label;
    label.append(radio, text); group.append(label);
  }
  $('booking-type-options').append(group);
}

function showDetails(slot) {
  clearAvailabilityTimers();
  ++state.availabilityRequest;
  state.choice = slot;
  $('times-step').classList.add('hidden');
  $('details-step').classList.remove('hidden');
  renderTimeSummary(document, $('selected-summary'), selectedTime(slot.start));
  $('booking-form').querySelector('button[type="submit"]').textContent = selectedTime(slot.start).actionLabel;
  $('guest-name').focus();
}

function showCalendar() {
  $('details-step').classList.add('hidden');
  $('times-step').classList.remove('hidden');
  setStatus($('booking-status'));
}

document.querySelectorAll('.duration-option').forEach((button) => button.addEventListener('click', () => {
  showCalendar();
  const preferredDate = state.selectedDate;
  state.durationMinutes = Number(button.dataset.duration);
  document.querySelectorAll('.duration-option').forEach((item) => {
    const selected = item === button; item.classList.toggle('selected', selected); item.setAttribute('aria-checked', String(selected));
  });
  $('duration-label').textContent = `${state.durationMinutes} min`;
  loadMonth(preferredDate);
}));

$('previous-month').addEventListener('click', () => { showCalendar(); state.visibleMonth = new Date(state.visibleMonth.getFullYear(), state.visibleMonth.getMonth() - 1, 1, 12); loadMonth(); });
$('next-month').addEventListener('click', () => { showCalendar(); state.visibleMonth = new Date(state.visibleMonth.getFullYear(), state.visibleMonth.getMonth() + 1, 1, 12); loadMonth(); });
$('retry-dates').addEventListener('click', () => loadMonth(state.selectedDate));
$('back-to-calendar').addEventListener('click', () => { showCalendar(); selectDate(state.selectedDate); });
document.addEventListener('visibilitychange', () => {
  if (document.visibilityState === 'visible' && !$('date-step').classList.contains('hidden') && !$('times-step').classList.contains('hidden')) selectDate(state.selectedDate, true);
});
$('viewer-timezone').addEventListener('change', () => {
  const preferredDate = state.selectedDate;
  state.viewerTimeZone = $('viewer-timezone').value;
  updateTimezoneDescription();
  $('timezone-live').textContent = t('timezoneLive', { zone: zoneLabel(state.viewerTimeZone) });
  state.visibleMonth = startOfMonth(state.selectedDate ? parseDateKey(state.selectedDate) : parseDateKey(dateKeyInZone(Date.now(), state.viewerTimeZone)));
  loadMonth(preferredDate);
});

$('booking-form').addEventListener('submit', async (event) => {
  event.preventDefault();
  if (state.submitting) return;
  state.submitting = true;
  $('date-step').inert = true;
  const button = event.submitter || $('booking-form').querySelector('button[type="submit"]'); button.disabled = true; button.textContent = t('booking');
  setStatus($('booking-status'), t('submitting'));
  try {
    const bookingInput = {
      ...state.choice,
      date: state.selectedDate,
      viewerTimeZone: state.viewerTimeZone,
      durationMinutes: state.durationMinutes,
      guestName: $('guest-name').value,
      guestEmail: $('guest-email').value,
      meetingTitle: $('meeting-title').value,
    };
    const selectedType = $('booking-form').querySelector('input[name="bookingType"]:checked');
    if (selectedType) bookingInput.bookingType = selectedType.value;
    const signature = JSON.stringify(bookingInput);
    if (state.bookingAttempt?.signature !== signature) state.bookingAttempt = { signature, key: crypto.randomUUID() };
    const result = await request('/api/book', {
      method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ ...bookingInput, idempotencyKey: state.bookingAttempt.key }),
    });
    clearAvailabilityTimers();
    $('date-step').classList.add('hidden'); $('details-step').classList.add('hidden'); $('success-step').classList.remove('hidden');
    $('confirmed-title').textContent = result.booking.meetingTitle || '';
    renderTimeSummary(document, $('success-summary'), selectedTime(state.choice.start), t('email', { email: bookingInput.guestEmail }));
    $('success-step').focus();
    if (selectedType) {
      const label = result.booking.bookingTypeLabel || selectedType.closest('label').textContent;
      const type = document.createElement('span'); type.textContent = label;
      $('success-summary').append(type);
    }
    if (result.booking.managementUrl) { $('manage-booking').href = result.booking.managementUrl; $('manage-booking').classList.remove('hidden'); }
    if (result.booking.meetLink) { $('meet-link').href = result.booking.meetLink; $('meet-link').classList.remove('hidden'); }
  } catch (error) {
    const conflict = error.message.includes('busy') || error.message.includes('required_buffer');
    setStatus($('booking-status'), conflict ? t('conflict') : error instanceof TypeError ? t('unavailable') : error.message, true);
  } finally {
    state.submitting = false;
    $('date-step').inert = false;
    button.disabled = false; button.textContent = selectedTime(state.choice.start).actionLabel;
  }
});

populateTimeZones(state.viewerTimeZone);
updateTimezoneDescription();
fetch('/api/config').then((response) => response.json()).then((config) => {
  renderBookingTypes(config.bookingTypes);
  if (config.demo) $('demo-note').textContent = t('demo');
  if (config.hostTimezone && !state.availability?.hostTimezone) {
    state.hostTimezone = config.hostTimezone;
    if (state.availability) renderSlots();
  }
}).catch(() => {});
loadMonth();
