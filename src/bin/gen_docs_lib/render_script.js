(function(){
  const q = document.getElementById('q');
  const onlyUsed = document.getElementById('only-used');
  const dark = document.getElementById('dark');

  function filter(){
    const needle = (q.value || '').toLowerCase();
    document.querySelectorAll('[data-search]').forEach(el=>{
      const hay = el.dataset.search.toLowerCase();
      el.classList.toggle('hidden', needle && !hay.includes(needle));
    });
  }
  function applyOnlyUsed(){
    document.querySelectorAll('.api-method').forEach(el=>{
      const used = el.dataset.used === '1';
      el.classList.toggle('unused', !used);
      if (onlyUsed.checked){ el.classList.toggle('hidden', !used); }
    });
  }
  function applyDark(){ document.body.classList.toggle('dark', dark.checked); }

  if (q) q.addEventListener('input', filter);
  if (onlyUsed) onlyUsed.addEventListener('change', applyOnlyUsed);
  if (dark) dark.addEventListener('change', applyDark);
  applyOnlyUsed();
})();
