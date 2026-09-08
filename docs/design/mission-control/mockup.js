// Theme toggle: press "d" or click the floating button. Mockup-only.
(function () {
  const root = document.documentElement;
  if (location.hash === '#dark') root.classList.add('dark');
  const btn = document.createElement('button');
  btn.className = 'btn theme-toggle';
  const label = () => (btn.textContent = root.classList.contains('dark') ? 'Paper (light)' : 'Graphite (dark)');
  btn.onclick = () => { root.classList.toggle('dark'); label(); };
  document.addEventListener('keydown', (e) => { if (e.key === 'd' && !e.metaKey) btn.click(); });
  window.addEventListener('DOMContentLoaded', () => { document.body.appendChild(btn); label(); });
})();
