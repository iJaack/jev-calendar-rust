const $ = (id) => document.getElementById(id);
const status = $('status');
const addressInput = $('base-location');
const addressResults = $('address-results');
const addressHelp = $('address-help');
const addressLanguage = (navigator.language || 'en').split('-')[0];
let selectedProviderId = '';
let selectedAddressToken = '';
let selectedAddress = '';
let addressTimer;
let addressRequest;
let activeAddressIndex = -1;

for (let hour = 0; hour <= 24; hour += 1) {
  const text = `${String(hour).padStart(2, '0')}:00`;
  for (const id of ['start-hour', 'end-hour']) {
    const option = document.createElement('option'); option.value = hour; option.textContent = text; $(id).append(option);
  }
}

function message(text, error = false) { status.textContent = text; status.className = `status${error ? ' error' : ''}`; }
async function request(path, options) {
  const response = await fetch(path, options); const json = await response.json();
  if (!response.ok) throw new Error(json.error || 'Request failed');
  return json;
}
function detectedTimeZone() { return Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC'; }
async function syncTimeZone() {
  if (document.visibilityState === 'hidden') return;
  const timezone = detectedTimeZone();
  try {
    const result = await request('/api/timezone', { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ timezone }) });
    $('current-timezone').textContent = `${result.timezone.replaceAll('_', ' ')} · synced now`;
  } catch {
    $('current-timezone').textContent = `${timezone.replaceAll('_', ' ')} · sync unavailable`;
  }
}

function hideAddressResults() {
  addressResults.replaceChildren();
  addressResults.classList.add('hidden');
  addressInput.setAttribute('aria-expanded', 'false');
  addressInput.removeAttribute('aria-activedescendant');
  activeAddressIndex = -1;
}

function selectAddress(suggestion) {
  selectedProviderId = '';
  selectedAddressToken = suggestion.selectionToken;
  selectedAddress = suggestion.text;
  addressInput.value = selectedAddress;
  addressInput.classList.remove('invalid');
  addressHelp.textContent = 'Complete address selected and ready to save.';
  addressHelp.className = 'verified';
  hideAddressResults();
}

function renderAddressResults(suggestions) {
  addressResults.replaceChildren();
  activeAddressIndex = -1;
  for (const [index, suggestion] of suggestions.entries()) {
    const option = document.createElement('button');
    option.type = 'button'; option.className = 'address-option'; option.id = `address-option-${index}`; option.setAttribute('role', 'option');
    const main = document.createElement('strong'); main.textContent = suggestion.mainText;
    const secondary = document.createElement('span'); secondary.textContent = suggestion.secondaryText;
    option.append(main, secondary);
    option.addEventListener('mousedown', (event) => event.preventDefault());
    option.addEventListener('click', () => selectAddress(suggestion));
    addressResults.append(option);
  }
  addressResults.classList.toggle('hidden', suggestions.length === 0);
  addressInput.setAttribute('aria-expanded', String(suggestions.length > 0));
}

async function searchAddresses() {
  const input = addressInput.value.trim();
  if (input.length < 3 || input === selectedAddress) return hideAddressResults();
  addressRequest?.abort(); addressRequest = new AbortController();
  addressHelp.textContent = 'Searching complete addresses…'; addressHelp.className = '';
  try {
    const result = await request('/api/address-search', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ input, languageCode: addressLanguage }), signal: addressRequest.signal });
    renderAddressResults(result.suggestions);
    addressHelp.textContent = result.suggestions.length ? 'Choose the correct complete address.' : 'No matching address found. Try adding the city or postal code.';
  } catch (error) {
    if (error.name === 'AbortError') return;
    hideAddressResults(); addressHelp.textContent = error.message; addressHelp.className = 'error';
  }
}

addressInput.addEventListener('input', () => {
  if (addressInput.value.trim() !== selectedAddress) { selectedProviderId = ''; selectedAddressToken = ''; addressInput.classList.remove('valid'); }
  addressInput.classList.remove('invalid');
  clearTimeout(addressTimer); addressTimer = setTimeout(searchAddresses, 250);
});
addressInput.addEventListener('keydown', (event) => {
  const options = [...addressResults.querySelectorAll('.address-option')];
  if (!options.length) return;
  if (event.key === 'Escape') return hideAddressResults();
  if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
    event.preventDefault(); activeAddressIndex = event.key === 'ArrowDown' ? (activeAddressIndex + 1) % options.length : (activeAddressIndex - 1 + options.length) % options.length;
    options.forEach((option, index) => option.classList.toggle('active', index === activeAddressIndex));
    addressInput.setAttribute('aria-activedescendant', options[activeAddressIndex].id);
  } else if (event.key === 'Enter' && activeAddressIndex >= 0) { event.preventDefault(); options[activeAddressIndex].click(); }
});
addressInput.addEventListener('blur', () => setTimeout(hideAddressResults, 120));

try {
  const settings = await request('/api/settings');
  addressInput.value = settings.baseLocation || '';
  selectedAddress = settings.baseLocationProviderId ? settings.baseLocation || '' : '';
  selectedProviderId = settings.baseLocationProviderId || '';
  if (selectedProviderId) { addressHelp.textContent = 'Verified complete address.'; addressHelp.className = 'verified'; }
  else if (settings.baseLocation) { addressHelp.textContent = 'This older value is unverified. Search and choose the complete address before saving.'; addressHelp.className = 'error'; }
  $('start-hour').value = settings.hours.start;
  $('end-hour').value = settings.hours.end;
  $('default-duration').value = settings.durationMinutes;
  $('min-buffer').value = settings.minimumTravelBufferMinutes;
  $('min-notice-hours').value = (settings.minimumBookingNoticeMinutes || 0) / 60;
  $('current-timezone').textContent = `${(settings.timezone || detectedTimeZone()).replaceAll('_', ' ')} · ${settings.timezoneSource || 'saved'}`;
  await syncTimeZone();
} catch (error) { message(error.message, true); }

$('settings-form').addEventListener('submit', async (event) => {
  event.preventDefault();
  const baseLocation = addressInput.value.trim();
  if (baseLocation && ((!selectedProviderId && !selectedAddressToken) || baseLocation !== selectedAddress)) {
    addressInput.classList.add('invalid'); addressHelp.textContent = 'Choose a complete address from the search results first.'; addressHelp.className = 'error'; addressInput.focus(); return;
  }
  try {
    const noticeHours = Number($('min-notice-hours').value);
    if (!Number.isFinite(noticeHours) || noticeHours < 0 || noticeHours > 168 || !Number.isInteger(noticeHours * 60)) throw new Error('Enter advance notice between 0 and 168 hours');
    const result = await request('/api/settings', { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ baseLocation, baseLocationProviderId: selectedProviderId, addressSelectionToken: selectedAddressToken, hours: { start: Number($('start-hour').value), end: Number($('end-hour').value) }, durationMinutes: Number($('default-duration').value), minimumTravelBufferMinutes: Number($('min-buffer').value), minimumBookingNoticeMinutes: noticeHours * 60 }) });
    addressInput.value = result.settings.baseLocation || ''; selectedAddress = addressInput.value; selectedProviderId = result.settings.baseLocationProviderId || ''; selectedAddressToken = '';
    addressHelp.textContent = selectedProviderId ? 'Verified complete address.' : 'Start typing, then choose a complete address.'; addressHelp.className = selectedProviderId ? 'verified' : '';
    message('Settings saved locally. Jev will refresh the advance notice in the background every six hours.');
  } catch (error) { message(error.message, true); }
});

document.addEventListener('visibilitychange', () => { if (document.visibilityState === 'visible') syncTimeZone(); });
window.addEventListener('focus', syncTimeZone);
setInterval(syncTimeZone, 60_000);

let workloadRules = [];
let workloadCalendars = [];
function workloadMessage(text, error = false) {
  const element = $('workload-status');
  element.textContent = text;
  element.className = `status${error ? ' error' : ''}`;
}
function projectInput(label, value, placeholder = '') {
  const wrapper = document.createElement('label');
  wrapper.textContent = label;
  const input = document.createElement('input');
  input.value = value; input.placeholder = placeholder;
  wrapper.append(input);
  return [wrapper, input];
}
function renderWorkloadRules() {
  const container = $('project-rules'); container.replaceChildren();
  for (const [index, rule] of workloadRules.entries()) {
    const row = document.createElement('details'); row.className = 'project-rule'; row.open = !rule.name;
    const heading = document.createElement('summary'); heading.textContent = rule.name || 'New project'; row.append(heading);
    const fields = document.createElement('div'); fields.className = 'project-fields';
    const [nameLabel, nameInput] = projectInput('Project', rule.name, 'Project name');
    nameInput.maxLength = 80; nameInput.addEventListener('input', () => { rule.name = nameInput.value; heading.textContent = rule.name || 'New project'; }); fields.append(nameLabel);
    const [keywordsLabel, keywordsInput] = projectInput('Title or calendar keywords', rule.keywords.join(', '), 'Separate keywords with commas');
    keywordsInput.addEventListener('input', () => { rule.keywords = keywordsInput.value.split(',').map(word => word.trim()).filter(Boolean); }); fields.append(keywordsLabel);
    const typeLabel = document.createElement('label'); typeLabel.textContent = 'Workload type';
    const typeSelect = document.createElement('select');
    for (const [value, label] of [['meetings', 'Meetings - cluster slots'], ['focus', 'Focus - protect middle hours'], ['deadline', 'Deadline - close heavy days']]) {
      const option = document.createElement('option'); option.value = value; option.textContent = label; typeSelect.append(option);
    }
    typeSelect.value = rule.type; typeSelect.addEventListener('change', () => { rule.type = typeSelect.value; }); typeLabel.append(typeSelect); fields.append(typeLabel);
    const calendarsLabel = document.createElement('label'); calendarsLabel.textContent = 'Matching calendars';
    const calendarsSelect = document.createElement('select'); calendarsSelect.multiple = true; calendarsSelect.size = Math.min(4, Math.max(2, workloadCalendars.length));
    for (const calendar of workloadCalendars) {
      const option = document.createElement('option'); option.value = calendar.id; option.textContent = calendar.name; option.selected = rule.calendarIds.includes(calendar.id); calendarsSelect.append(option);
    }
    calendarsSelect.addEventListener('change', () => { rule.calendarIds = [...calendarsSelect.selectedOptions].map(option => option.value); });
    calendarsLabel.append(calendarsSelect); fields.append(calendarsLabel);
    const remove = document.createElement('button'); remove.type = 'button'; remove.className = 'account-button danger'; remove.textContent = 'Remove project';
    remove.addEventListener('click', () => { workloadRules.splice(index, 1); renderWorkloadRules(); }); fields.append(remove);
    row.append(fields);
    container.append(row);
  }
}
async function loadWorkload() {
  try {
    const result = await request('/api/workload');
    workloadRules = result.rules; workloadCalendars = result.calendars;
    $('workload-learning').textContent = result.learning?.updatedAt
      ? `Last checked ${new Date(result.learning.updatedAt).toLocaleString()}. Typical busy-day threshold: ${result.learning.baselineBusyMinutes} minutes. Current advance notice: ${(result.learning.bookingNotice?.minutes ?? 0) / 60} hours (${result.learning.bookingNotice?.source === 'jev' ? 'Jev' : 'base setting'}).`
      : 'The first private workload check is pending. Availability still uses the current calendar.';
    const alert = result.learning?.weekAlert;
    const suggestions = { cluster_meetings: 'Group new calls onto existing meeting days.', protect_focus: 'Keep the quieter days open for focused work.', protect_deadline: 'Reduce new meetings before the deadline.' };
    $('weekly-alert').textContent = alert
      ? `Coming week: ${alert.heavyDays} unusually busy days and ${Math.round(alert.busyMinutes / 60)} hours committed. ${alert.source === 'jev' ? 'Jev suggests' : 'Suggested adjustment'}: ${suggestions[alert.suggestion] || 'Review your calendar.'} This is for your review; no calendar changes were made.`
      : result.learning?.updatedAt ? 'No unusual workload detected in the next five workdays.' : 'The coming-week check is pending.';
    renderWorkloadRules();
  } catch (error) { workloadMessage(error.message, true); }
}
$('add-project').addEventListener('click', () => { workloadRules.push({ name: '', keywords: [], calendarIds: [], type: 'meetings' }); renderWorkloadRules(); });
$('workload-form').addEventListener('submit', async (event) => {
  event.preventDefault();
  try {
    const result = await request('/api/workload', { method: 'PUT', headers: { 'Content-Type': 'application/json', 'X-Jev-Owner': '1' }, body: JSON.stringify({ rules: workloadRules }) });
    workloadRules = result.rules; renderWorkloadRules(); workloadMessage('Project rules saved. The workload baseline will refresh in the background.');
  } catch (error) { workloadMessage(error.message, true); }
});
void loadWorkload();


function accountMessage(text, error = false) {
  $('account-status').textContent = text;
  $('account-status').className = `status${error ? ' error' : ''}`;
}
async function accountRequest(path, method, body) {
  return request(path, { method, headers: { 'Content-Type': 'application/json', 'X-Jev-Owner': '1' }, body: JSON.stringify(body) });
}
function accountButton(label, action, className = 'account-button') {
  const button = document.createElement('button');
  button.type = 'button'; button.className = className; button.textContent = label;
  button.addEventListener('click', async () => {
    button.disabled = true;
    try { await action(); } catch (error) { accountMessage(error.message, true); } finally { button.disabled = false; }
  });
  return button;
}
async function connectAccount(accountId) {
  accountMessage('Opening Google…');
  const result = await accountRequest('/api/google/connect', 'POST', { accountId });
  window.location.assign(result.url);
}
function renderAccounts({ accounts, demo }) {
  const list = $('google-accounts'); list.replaceChildren(); list.setAttribute('aria-busy', 'false');
  $('google-connect').textContent = accounts.length ? 'Connect another account' : 'Connect Google account';
  $('google-connect').disabled = demo;
  if (demo) $('connection-copy').textContent = 'Demo preview. Google authorization is available in the live dashboard.';
  if (!accounts.length) {
    const empty = document.createElement('p'); empty.className = 'note';
    empty.textContent = demo ? 'No real Google accounts are connected in this demo.' : 'No accounts connected. Availability is paused until you connect Google.';
    list.append(empty);
  }
  for (const account of accounts) {
    const row = document.createElement('article'); row.className = 'google-account'; row.setAttribute('aria-label', account.email);
    const details = document.createElement('div'); details.className = 'account-details';
    const email = document.createElement('strong'); email.textContent = account.email;
    const state = document.createElement('span'); state.className = 'account-state';
    state.textContent = account.needsReconnect ? 'Connection needs attention · reconnect this account' : account.allCalendarsAuthorized ? account.receivesBookings ? 'New bookings · primary calendar' : 'Availability checks' : 'Reconnect to grant all calendar permissions';
    if (account.needsReconnect || !account.allCalendarsAuthorized) state.classList.add('error');
    details.append(email, state);
    const actions = document.createElement('div'); actions.className = 'account-actions';
    if (!account.receivesBookings) actions.append(accountButton('Use for new bookings', async () => {
      renderAccounts(await accountRequest('/api/google/accounts', 'PUT', { accountId: account.id }));
      accountMessage(`New bookings will go to ${account.email}.`);
    }));
    if (!demo) actions.append(accountButton('Reconnect', () => connectAccount(account.id)));
    const confirmation = document.createElement('div'); confirmation.className = 'account-confirm hidden';
    const explanation = document.createElement('p');
    explanation.textContent = accounts.length === 1 ? 'Disconnect your last account? New bookings will pause until you connect an account. Existing meetings stay unchanged.' : `Stop checking calendars for ${account.email}? Existing meetings stay unchanged.`;
    const confirmButton = accountButton('Confirm disconnect', async () => {
      renderAccounts(await accountRequest('/api/google/accounts', 'DELETE', { accountId: account.id }));
      accountMessage(`${account.email} disconnected. Existing meetings were kept.`);
      $('google-connect').focus();
    }, 'account-button danger');
    let disconnect;
    const cancel = accountButton('Cancel', () => { confirmation.classList.add('hidden'); disconnect.setAttribute('aria-expanded', 'false'); disconnect.focus(); });
    confirmation.append(explanation, confirmButton, cancel);
    disconnect = accountButton('Disconnect', () => {
      confirmation.classList.remove('hidden'); disconnect.setAttribute('aria-expanded', 'true'); cancel.focus();
    }, 'account-button danger');
    disconnect.setAttribute('aria-expanded', 'false');
    if (account.receivesBookings && accounts.length > 1) {
      disconnect.disabled = true;
      const hint = document.createElement('small'); hint.className = 'account-disconnect-hint'; hint.textContent = 'Choose another account for new bookings before disconnecting this one.'; details.append(hint);
    }
    actions.append(disconnect); row.append(details, actions, confirmation); list.append(row);
  }
}
$('google-connect').addEventListener('click', async () => {
  $('google-connect').disabled = true;
  try { await connectAccount(); } catch (error) { accountMessage(error.message, true); $('google-connect').disabled = false; }
});
try {
  renderAccounts(await request('/api/google/accounts'));
  const error = new URLSearchParams(window.location.search).get('connectionError');
  if (error) { accountMessage(error, true); window.history.replaceState(null, '', '/owner'); }
} catch (error) {
  $('google-accounts').setAttribute('aria-busy', 'false');
  $('google-accounts').textContent = 'Could not load Google accounts. Reload this page to try again.';
  accountMessage(error.message, true);
}
