// Loaded blocking in <head> so the saved theme applies before first paint (no flash).
(function () {
  var d = document.documentElement;
  d.classList.add('js');
  try {
    var t = localStorage.getItem('fluxo-theme');
    if (t === 'light' || t === 'dark') d.setAttribute('data-theme', t);
  } catch (e) { /* storage unavailable: follow OS */ }
})();
