// fluxo — progressive enhancement layer.
// Every page is complete, usable HTML without this file; this adds layout, interactivity and polish.
(function () {
  'use strict';
  var $ = function (s, r) { return (r || document).querySelector(s); };
  var $$ = function (s, r) { return Array.prototype.slice.call((r || document).querySelectorAll(s)); };

  /* ---------- Theme (saved to the account) ---------- */
  function getTheme() { return document.documentElement.getAttribute('data-theme') || 'system'; }
  function applyTheme(t) {
    var d = document.documentElement;
    if (t === 'system') d.removeAttribute('data-theme'); else d.setAttribute('data-theme', t);
    $$('input[name="theme"]').forEach(function (r) { r.checked = r.value === t; });
    $$('[data-theme-toggle]').forEach(function (b) { b.setAttribute('title', 'Theme: ' + t); });
  }
  function saveTheme(t) {
    var meta = $('meta[name="csrf"]');
    if (!meta || !window.fetch) return;
    fetch('/settings/theme', {
      method: 'POST',
      credentials: 'same-origin',
      headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
      body: 'csrf=' + encodeURIComponent(meta.content) + '&theme=' + encodeURIComponent(t)
    }).catch(function () {});
  }
  $$('[data-theme-toggle]').forEach(function (b) {
    b.hidden = false;
    b.addEventListener('click', function () {
      var order = ['system', 'light', 'dark'];
      var t = order[(order.indexOf(getTheme()) + 1) % 3];
      applyTheme(t); saveTheme(t);
    });
  });
  $$('input[name="theme"]').forEach(function (r) {
    r.addEventListener('change', function () { applyTheme(r.value); saveTheme(r.value); });
  });
  applyTheme(getTheme());

  /* ---------- Copy buttons: data-copy="#id" copies that field's text ---------- */
  $$('[data-copy]').forEach(function (b) {
    b.addEventListener('click', function () {
      var el = $(b.getAttribute('data-copy'));
      if (!el) return;
      var text = el.value != null ? el.value : el.textContent;
      var done = function () { b.textContent = 'Copied'; };
      if (navigator.clipboard) navigator.clipboard.writeText(text).then(done, function () { if (el.select) el.select(); });
      else if (el.select) el.select();
    });
  });

  /* ---------- Menus (<details>): close on outside click / Escape ---------- */
  document.addEventListener('click', function (e) {
    $$('details.menu[open]').forEach(function (m) { if (!m.contains(e.target)) m.open = false; });
  });
  document.addEventListener('keydown', function (e) {
    if (e.key === 'Escape') $$('details.menu[open]').forEach(function (m) { m.open = false; });
  });

  /* ---------- Goal path graph ----------
     The goal sits at the top (row 0). Each step sits one row below the lowest step it feeds,
     so reading down answers "what does this need?" and work rises toward the goal.
     Markup: <li data-id data-deps="ids it needs" data-path="1" if on the most likely critical path>.
     The goal's own <li> lists the final steps as its deps. */
  $$('.graph').forEach(function (graph) {
    var list = $('.path-list', graph);
    var lis = $$(':scope > li[data-id]', list);
    var byId = {};
    lis.forEach(function (li) {
      byId[li.dataset.id] = { li: li, id: li.dataset.id, deps: (li.dataset.deps || '').split(/\s+/).filter(Boolean), path: li.dataset.path === '1' };
    });
    var nodes = lis.map(function (li) { return byId[li.dataset.id]; });
    nodes.forEach(function (n) { n.kids = nodes.filter(function (m) { return m.deps.indexOf(n.id) !== -1; }); });
    // row = longest chain of "feeds" links up to the goal
    function rowOf(n, seen) {
      if (n.row != null) return n.row;
      seen = seen || {}; if (seen[n.id]) return 0; seen[n.id] = 1;
      var r = 0;
      n.kids.forEach(function (k) { r = Math.max(r, rowOf(k, seen) + 1); });
      return (n.row = r);
    }
    nodes.forEach(function (n) { rowOf(n); });
    var nRows = Math.max.apply(null, nodes.map(function (n) { return n.row; })) + 1;

    var svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
    svg.setAttribute('class', 'edges');
    svg.setAttribute('aria-hidden', 'true');
    graph.insertBefore(svg, list);
    graph.classList.add('enhanced');

    function layout() {
      var W = graph.clientWidth, gapX = 16, gapY = 46, DW = 10;
      var rows = []; for (var i = 0; i < nRows; i++) rows.push([]);
      nodes.forEach(function (n) { rows[n.row].push(n); n.ups = []; });
      // Every edge joins an upper item (the dependent, smaller row) to a lower one (its prerequisite).
      // Edges spanning several rows get a waypoint per row crossed, so they run through gaps.
      var chains = [];
      nodes.forEach(function (n) {
        n.deps.forEach(function (d) {
          var p = byId[d]; if (!p) return;
          var chain = [n];
          for (var r = n.row + 1; r < p.row; r++) {
            var dm = { dummy: true, row: r, ups: [chain[chain.length - 1]] };
            rows[r].push(dm); chain.push(dm);
          }
          p.ups.push(chain[chain.length - 1]);
          chain.push(p);
          chains.push({ upper: n, lower: p, pts: chain });
        });
      });
      var nodeW = 240;
      rows.forEach(function (r) {
        var real = r.filter(function (x) { return !x.dummy; }).length, dum = r.length - real;
        if (real) nodeW = Math.min(nodeW, Math.floor((W - dum * (DW + gapX) - (real - 1) * gapX) / real));
      });
      var wOf = function (x) { return x.dummy ? DW : (x.li.dataset.wide ? Math.min(W, Math.round(nodeW * 1.5)) : nodeW); };
      var cx = function (x) { return x.x + wOf(x) / 2; };
      function place(row) {
        var total = row.reduce(function (a, x) { return a + wOf(x); }, 0) + (row.length - 1) * gapX;
        var x = (W - total) / 2;
        row.forEach(function (it) { it.x = x; x += wOf(it) + gapX; });
      }
      place(rows[0]);
      rows.forEach(function (row, ri) {
        if (ri === 0) return;
        row.forEach(function (it, i) {
          var ps = (it.ups || []).filter(function (u) { return u.x != null; }).map(cx);
          it.bary = ps.length ? ps.reduce(function (a, b) { return a + b; }, 0) / ps.length : W + i;
        });
        row.sort(function (a, b) { return a.bary - b.bary; });
        place(row);
      });
      var y = 0;
      rows.forEach(function (row) {
        var h = 0;
        row.forEach(function (it) {
          if (it.dummy) return;
          it.li.style.left = it.x + 'px'; it.li.style.top = y + 'px'; it.li.style.width = wOf(it) + 'px';
          h = Math.max(h, it.li.offsetHeight);
        });
        row.forEach(function (it) { it.y = y; it.h = it.dummy ? h : it.li.offsetHeight; });
        y += h + gapY;
      });
      var H = y - gapY;
      graph.style.height = H + 'px';
      svg.setAttribute('viewBox', '0 0 ' + W + ' ' + H);
      svg.setAttribute('width', W); svg.setAttribute('height', H);
      svg.innerHTML = '';
      chains.forEach(function (c) {
        var d = '', pts = c.pts;
        for (var i = 0; i < pts.length - 1; i++) {
          var a = pts[i], b = pts[i + 1];
          var x1 = cx(a), y1 = a.y + a.h, x2 = cx(b), y2 = b.y, my = (y1 + y2) / 2;
          d += (i ? ' L' : 'M') + x1 + ',' + y1 + ' C' + x1 + ',' + my + ' ' + x2 + ',' + my + ' ' + x2 + ',' + y2;
          if (b.dummy) d += ' L' + x2 + ',' + (b.y + b.h);
        }
        var crit = c.upper.path && c.lower.path;
        var path = document.createElementNS('http://www.w3.org/2000/svg', 'path');
        path.setAttribute('d', d);
        if (crit) path.setAttribute('class', 'crit');
        path.dataset.from = c.lower.id; path.dataset.to = c.upper.id;
        svg.appendChild(path);
        // small upward arrowhead where the edge enters the step it feeds
        var x0 = cx(c.upper), y0 = c.upper.y + c.upper.h;
        var tip = document.createElementNS('http://www.w3.org/2000/svg', 'path');
        tip.setAttribute('d', 'M' + (x0 - 4) + ',' + (y0 + 7) + ' L' + x0 + ',' + (y0 + 2) + ' L' + (x0 + 4) + ',' + (y0 + 7));
        tip.setAttribute('class', 'tip' + (crit ? ' crit' : ''));
        tip.dataset.from = c.lower.id; tip.dataset.to = c.upper.id;
        svg.appendChild(tip);
      });
    }
    layout();
    var rt; window.addEventListener('resize', function () { clearTimeout(rt); rt = setTimeout(layout, 80); });

    // Hover/focus a step → highlight what it needs (below) and what it leads to (above)
    function walk(n, dir, acc) {
      acc[n.id] = 1;
      (dir === 'down' ? n.deps.map(function (d) { return byId[d]; }) : n.kids).forEach(function (m) { if (m && !acc[m.id]) walk(m, dir, acc); });
      return acc;
    }
    nodes.forEach(function (n) {
      var on = function () {
        var set = walk(n, 'down', {}); walk(n, 'up', set);
        graph.classList.add('focusing');
        nodes.forEach(function (m) { var el = $('.node', m.li); if (el) el.classList.toggle('hl', !!set[m.id]); });
        $$('[data-from]', svg).forEach(function (p) { p.classList.toggle('dim', !(set[p.dataset.from] && set[p.dataset.to])); });
      };
      var off = function () {
        graph.classList.remove('focusing');
        $$('[data-from]', svg).forEach(function (p) { p.classList.remove('dim'); });
      };
      n.li.addEventListener('mouseenter', on); n.li.addEventListener('mouseleave', off);
      n.li.addEventListener('focusin', on); n.li.addEventListener('focusout', off);
    });
  });

  /* ---------- Risks: live likelihood slider ---------- */
  $$('.risk').forEach(function (r) {
    var range = $('input[type=range]', r), out = $('.lk', r);
    if (!range) return;
    var update = function () {
      out.textContent = range.value + '%';
      r.classList.toggle('retired', range.value === '0');
    };
    range.addEventListener('input', update); update();
  });

  /* ---------- OTP: replace single field with 6 boxes ---------- */
  $$('[data-otp]').forEach(function (single) {
    var wrap = document.createElement('div'); wrap.className = 'otp';
    var boxes = [];
    for (var i = 0; i < 6; i++) {
      var b = document.createElement('input');
      b.inputMode = 'numeric'; b.maxLength = 1; b.autocomplete = i === 0 ? 'one-time-code' : 'off';
      b.setAttribute('aria-label', 'Digit ' + (i + 1));
      boxes.push(b); wrap.appendChild(b);
    }
    var sync = function () { single.value = boxes.map(function (b) { return b.value; }).join(''); };
    boxes.forEach(function (b, i) {
      b.addEventListener('input', function () {
        b.value = b.value.replace(/\D/g, '').slice(-1);
        if (b.value && boxes[i + 1]) boxes[i + 1].focus();
        sync();
        if (single.value.length === 6 && single.form && single.form.dataset.autosubmit != null) single.form.requestSubmit();
      });
      b.addEventListener('keydown', function (e) {
        if (e.key === 'Backspace' && !b.value && boxes[i - 1]) boxes[i - 1].focus();
      });
      b.addEventListener('paste', function (e) {
        var t = (e.clipboardData.getData('text') || '').replace(/\D/g, '').slice(0, 6);
        if (!t) return; e.preventDefault();
        t.split('').forEach(function (c, j) { boxes[j].value = c; });
        boxes[Math.min(t.length, 5)].focus(); sync();
      });
    });
    single.type = 'hidden';
    single.parentNode.insertBefore(wrap, single);
  });

  /* ---------- Password strength ---------- */
  $$('[data-strength]').forEach(function (input) {
    var bar = $(input.getAttribute('data-strength'));
    if (!bar) return;
    bar.parentElement.hidden = false;
    input.addEventListener('input', function () {
      var v = input.value, s = 0;
      if (v.length >= 12) s++; if (v.length >= 16) s++;
      if (/[a-z]/.test(v) && /[A-Z]/.test(v)) s++; if (/\d/.test(v)) s++; if (/[^\w]/.test(v)) s++;
      var pct = Math.min(100, s * 20);
      bar.style.width = pct + '%';
      bar.style.background = pct < 50 ? 'var(--red)' : pct < 80 ? 'var(--amber)' : 'var(--green)';
    });
  });

  /* ---------- Forms: save on change, kind-specific fields, confirmations ---------- */
  $$('form[data-autosave]').forEach(function (f) {
    f.addEventListener('change', function () { f.requestSubmit(); });
  });
  var kindRadios = $$('[data-kind-toggle]');
  function showKind() {
    var picked = kindRadios.filter(function (r) { return r.checked; })[0];
    if (!picked) return;
    $$('.kind-fields').forEach(function (fs) { fs.hidden = fs.getAttribute('data-kind') !== picked.value; });
  }
  kindRadios.forEach(function (r) { r.addEventListener('change', showKind); });
  showKind();
  $$('form[data-confirm]').forEach(function (f) {
    f.addEventListener('submit', function (e) {
      if (!window.confirm(f.getAttribute('data-confirm'))) e.preventDefault();
    });
  });

  /* ---------- Keyboard: g f → Focus, g g → Goals, g s → Settings ---------- */
  var g = false;
  document.addEventListener('keydown', function (e) {
    if (/INPUT|TEXTAREA|SELECT/.test(e.target.tagName) || e.target.isContentEditable || e.metaKey || e.ctrlKey || e.altKey) return;
    if (!$('.topbar')) return; // signed-in pages only
    if (g && e.key === 'g') { g = false; location.href = '/goals'; return; }
    if (e.key === 'g') { g = true; setTimeout(function () { g = false; }, 800); return; }
    if (g && e.key === 'f') location.href = '/';
    if (g && e.key === 's') location.href = '/settings';
    g = false;
  });
})();
