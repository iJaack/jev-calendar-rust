// theme.js mounts first, in its earlier DOMContentLoaded listener. Preserve the
// visible canonical label at the start of each accessible name (WCAG 2.5.3).
document.addEventListener('DOMContentLoaded', () => {
  const lang = document.documentElement.lang.toLowerCase().startsWith('it') ? 'it' : 'en';
  document.querySelectorAll('.jk-thsw button').forEach((button) => {
    const label = button.querySelector('.l');
    const suffix = button.querySelector(`.lang-${lang}`);
    if (label && suffix) button.setAttribute('aria-label', label.textContent + suffix.textContent);
  });
});
