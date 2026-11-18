function detectBookingLanguage(browser, search = '') {
  const override = new URLSearchParams(search).get('lang');
  if (override === 'it' || override === 'en') return override;
  const first = browser?.languages?.[0] || browser?.language || '';
  return first.toLowerCase().startsWith('it') ? 'it' : 'en';
}
document.documentElement.lang = detectBookingLanguage(typeof navigator === 'undefined' ? undefined : navigator, location.search);
document.title = document.documentElement.lang === 'it' ? 'Prenota un incontro con Jaack' : 'Book a meeting with Jaack';
