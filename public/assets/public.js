const $ = (id) => document.getElementById(id);

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

function dayLabel(value) { return new Intl.DateTimeFormat('en-US', { weekday: 'long', month: 'long', day: 'numeric', timeZone: 'UTC' }).format(new Date(`${value}T12:00:00Z`)); }
function time(iso) { return new Intl.DateTimeFormat('en-US', { hour: 'numeric', minute: '2-digit', timeZoneName: 'short', timeZone: state.viewerTimeZone }).format(new Date(iso)); }
function fullDateTime(iso) { return new Intl.DateTimeFormat('en-US', { weekday: 'long', month: 'long', day: 'numeric', hour: 'numeric', minute: '2-digit', timeZoneName: 'short', timeZone: state.viewerTimeZone }).format(new Date(iso)); }

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
  $('month-label').textContent = new Intl.DateTimeFormat('en-US', { month: 'long', year: 'numeric' }).format(month);
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
    button.setAttribute('aria-label', `${dayLabel(key)}${unavailable ? ', no available times' : key === state.recommendedDate ? ', suggested date' : ''}`);
    const disabled = !inRange || !state.availableDates?.has(key);
    button.disabled = disabled;
    if (key === localDateKey(today)) button.classList.add('today');
    if (!disabled && key === state.recommendedDate) button.classList.add('recommended');
    if (key === state.selectedDate) { button.classList.add('selected'); button.setAttribute('aria-current', 'date'); }
    if (!disabled) button.addEventListener('click', () => selectDate(key));
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
    setStatus($('availability-status'), 'Availability needs refreshing. Select a date to check again.');
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
  setStatus($('availability-status'), 'Checking available dates…');
  $('times-date').textContent = 'Checking available dates';
  $('slots').innerHTML = '<div class="loading-times"><i></i><i></i><i></i></div>';
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
      setStatus($('availability-status'), result.partial ? 'Some dates could not be checked. Try again later.' : 'No dates with available times this month.', Boolean(result.partial));
      $('retry-dates').classList.remove('hidden');
      $('times-date').textContent = 'No available dates';
      $('slots').replaceChildren();
      return;
    }
    setStatus($('availability-status'));
    const firstDate = preferredDate && state.availableDates.has(preferredDate) ? preferredDate : result.dates[0];
    await selectDate(firstDate);
    if (result.partial && requestId === state.monthRequest) setStatus($('availability-status'), 'Some dates could not be checked. Try again later.');
  } catch (error) {
    if (requestId !== state.monthRequest) return;
    setStatus($('availability-status'), `Could not check available dates: ${error.message}`, true);
    $('retry-dates').classList.remove('hidden');
    $('times-date').textContent = 'Availability unavailable';
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
    $('host-timezone').textContent = zoneLabel(cached.value.hostTimezone);
    renderSlots();
    expireAvailability(key, cached, requestId);
    if (cached.expiresAt > startedAt && !preserveChoice) return;
  } else {
    $('slots').innerHTML = '<div class="loading-times"><i></i><i></i><i></i></div>';
  }
  try {
    const availability = await request('/api/availability', {
      method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ date, durationMinutes: state.durationMinutes, viewerTimeZone: state.viewerTimeZone }),
    });
    if (requestId !== state.availabilityRequest) return;
    const entry = cacheAvailability(key, availability, startedAt);
    if (entry.validUntil <= performance.now()) throw new Error('Calendar is refreshing. Please check again shortly.');
    const choice = state.choice;
    if (choice && !availability.candidates.some(slot => slot.start === choice.start && slot.end === choice.end)) {
      state.choice = null;
      showCalendar();
      setStatus($('availability-status'), 'That time just became unavailable. Choose another time.');
    }
    const candidatesChanged = JSON.stringify(state.availability?.candidates) !== JSON.stringify(availability.candidates);
    state.availability = availability;
    state.viewerTimeZone = availability.viewerTimeZone;
    $('viewer-timezone').value = state.viewerTimeZone;
    $('host-timezone').textContent = zoneLabel(availability.hostTimezone);
    if (!availability.candidates.length) {
      state.availableDates.delete(date);
      if (state.recommendedDate === date) state.recommendedDate = null;
      state.selectedDate = null;
      renderCalendar();
      setStatus($('availability-status'), 'That date just became unavailable. Choose another date.');
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
      setStatus($('availability-status'), 'Could not refresh times yet. Booking will recheck your selection.');
      return;
    }
    state.availabilityCache.delete(key);
    state.availability = null; state.choice = null;
    $('slots').replaceChildren();
    setStatus($('availability-status'), error.message, true);
    $('retry-dates').classList.remove('hidden');
  }
}

function renderSlots() {
  const { candidates } = state.availability;
  const list = $('slots'); list.replaceChildren();
  if (!candidates.length) {
    const empty = document.createElement('p'); empty.className = 'times-empty'; empty.textContent = 'No times fit Jaack’s calendar and meeting spacing on this date. Try another date.'; list.append(empty); return;
  }
  for (const slot of candidates) {
    const row = document.createElement('div'); row.className = 'time-choice';
    const select = document.createElement('button'); select.type = 'button'; select.className = 'time-button'; select.textContent = time(slot.start);
    select.setAttribute('aria-pressed', String(state.choice?.start === slot.start && state.choice?.end === slot.end));
    const next = document.createElement('button'); next.type = 'button'; next.className = 'next-button'; next.textContent = 'Next'; next.setAttribute('aria-label', `Next, continue with ${time(slot.start)}`); next.setAttribute('aria-hidden', 'true'); next.tabIndex = -1; next.disabled = true;
    select.addEventListener('click', () => activateTime(row, slot));
    next.addEventListener('click', () => showDetails(slot));
    if (state.choice?.start === slot.start && state.choice?.end === slot.end) {
      row.classList.add('selected'); next.disabled = false; next.tabIndex = 0; next.setAttribute('aria-hidden', 'false');
    }
    row.append(select, next); list.append(row);
  }
  $('demo-note').textContent = state.availability.demo ? 'Demo availability' : '';
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
  const legend = document.createElement('legend'); legend.textContent = 'What is this for?'; group.append(legend);
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
  $('date-step').classList.add('hidden');
  $('details-step').classList.remove('hidden');
  $('selected-summary').innerHTML = `<strong>${fullDateTime(slot.start)}</strong><span>${state.durationMinutes} min · Google Meet · ${zoneLabel(state.viewerTimeZone)}</span>`;
  $('guest-name').focus();
}

function showCalendar() {
  $('details-step').classList.add('hidden');
  $('date-step').classList.remove('hidden');
  setStatus($('booking-status'));
}

document.querySelectorAll('.duration-option').forEach((button) => button.addEventListener('click', () => {
  const preferredDate = state.selectedDate;
  state.durationMinutes = Number(button.dataset.duration);
  document.querySelectorAll('.duration-option').forEach((item) => {
    const selected = item === button; item.classList.toggle('selected', selected); item.setAttribute('aria-checked', String(selected));
  });
  $('duration-label').textContent = `${state.durationMinutes} min`;
  loadMonth(preferredDate);
}));

$('previous-month').addEventListener('click', () => { state.visibleMonth = new Date(state.visibleMonth.getFullYear(), state.visibleMonth.getMonth() - 1, 1, 12); loadMonth(); });
$('next-month').addEventListener('click', () => { state.visibleMonth = new Date(state.visibleMonth.getFullYear(), state.visibleMonth.getMonth() + 1, 1, 12); loadMonth(); });
$('retry-dates').addEventListener('click', () => loadMonth(state.selectedDate));
$('back-to-calendar').addEventListener('click', () => { showCalendar(); selectDate(state.selectedDate); });
document.addEventListener('visibilitychange', () => {
  if (document.visibilityState === 'visible' && !$('date-step').classList.contains('hidden')) selectDate(state.selectedDate, true);
});
$('viewer-timezone').addEventListener('change', () => {
  const preferredDate = state.selectedDate;
  state.viewerTimeZone = $('viewer-timezone').value;
  state.visibleMonth = startOfMonth(state.selectedDate ? parseDateKey(state.selectedDate) : parseDateKey(dateKeyInZone(Date.now(), state.viewerTimeZone)));
  loadMonth(preferredDate);
});

$('booking-form').addEventListener('submit', async (event) => {
  event.preventDefault();
  const button = event.submitter; button.disabled = true; button.textContent = 'Scheduling…';
  setStatus($('booking-status'), 'Rechecking availability and creating your event…');
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
    $('details-step').classList.add('hidden'); $('success-step').classList.remove('hidden');
    $('confirmed-title').textContent = result.booking.meetingTitle || '';
    $('success-summary').innerHTML = `<strong>${fullDateTime(state.choice.start)}</strong><span>${state.durationMinutes} min · Google Meet · ${zoneLabel(state.viewerTimeZone)}</span>`;
    if (selectedType) {
      const label = result.booking.bookingTypeLabel || selectedType.closest('label').textContent;
      const type = document.createElement('span'); type.textContent = label;
      $('success-summary').append(type);
    }
    if (result.booking.managementUrl) { $('manage-booking').href = result.booking.managementUrl; $('manage-booking').classList.remove('hidden'); }
    if (result.booking.meetLink) { $('meet-link').href = result.booking.meetLink; $('meet-link').classList.remove('hidden'); }
  } catch (error) {
    const conflict = error.message.includes('busy') || error.message.includes('required_buffer');
    setStatus($('booking-status'), conflict ? 'That time is no longer available. Go back and choose another.' : error.message, true);
  } finally {
    button.disabled = false; button.textContent = 'Schedule event';
  }
});

populateTimeZones(state.viewerTimeZone);
fetch('/api/config').then((response) => response.json()).then((config) => {
  renderBookingTypes(config.bookingTypes);
  if (config.demo) $('demo-note').textContent = 'Demo mode';
  if (config.hostTimezone) $('host-timezone').textContent = zoneLabel(config.hostTimezone);
}).catch(() => {});
loadMonth();
