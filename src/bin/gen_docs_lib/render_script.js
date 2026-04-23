(function(){
  const q = document.getElementById('q');
  const onlyUsed = document.getElementById('only-used');
  const dark = document.getElementById('dark');

  function applyFilters(){
    const needle = (q.value || '').toLowerCase();
    const onlyUsedActive = onlyUsed && onlyUsed.checked;
    document.querySelectorAll('[data-search]').forEach(el=>{
      const hay = el.dataset.search.toLowerCase();
      const needleMismatch = needle && !hay.includes(needle);
      let unusedFilter = false;
      if (onlyUsedActive && el.classList.contains('api-method')) {
        unusedFilter = el.dataset.used !== '1';
      }
      el.classList.toggle('hidden', needleMismatch || unusedFilter);
    });
  }
  function applyUnusedDim(){
    document.querySelectorAll('.api-method').forEach(el=>{
      el.classList.toggle('unused', el.dataset.used !== '1');
    });
  }
  function applyDark(){ document.body.classList.toggle('dark', dark.checked); }

  if (q) q.addEventListener('input', applyFilters);
  if (onlyUsed) onlyUsed.addEventListener('change', applyFilters);
  if (dark) dark.addEventListener('change', applyDark);
  applyUnusedDim();
  applyFilters();
})();
