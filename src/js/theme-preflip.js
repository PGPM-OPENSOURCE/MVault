/* js/theme-preflip.js: switch to the saved theme's stylesheet before the page paints (classic
 * script, loaded right after the theme <link>s). Replaces per-page inline scripts.
 * Page scripts apply themes through window.MemoroTheme so every page agrees on the default. */
(function () {
  'use strict';
  const DEFAULT = 'pgpm';

  // The saved theme when this page has a stylesheet for it, else the default
  function saved() {
    let theme = null;
    try { theme = sessionStorage.getItem('theme') ?? localStorage.getItem('theme'); } catch (_) {}
    return theme && document.getElementById(`theme-${theme}`) ? theme : DEFAULT;
  }

  // Enables theme-<name> and disables the other theme stylesheets; returns the theme now in effect
  function flip(theme) {
    if (!document.getElementById(`theme-${theme}`)) theme = DEFAULT;
    document.querySelectorAll('link[id^="theme-"]').forEach((link) => { link.disabled = link.id !== `theme-${theme}`; });
    return theme;
  }

  window.MemoroTheme = { DEFAULT, saved, flip };
  flip(saved());
})();
