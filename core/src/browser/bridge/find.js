(function(){
  var strategy = window.__cc_find_strategy || 'text';
  var value = window.__cc_find_value || '';
  var out = [];
  var all = document.querySelectorAll('[data-catalyst-ref]');
  for (var i = 0; i < all.length && out.length < 20; i++) {
    var el = all[i];
    var ref = el.getAttribute('data-catalyst-ref');
    var ok = false;
    if (strategy === 'css') { try { ok = el.matches(value); } catch (e) {} }
    else if (strategy === 'role') {
      ok = (el.getAttribute('role') || '') === value || el.tagName.toLowerCase() === value;
    } else if (strategy === 'placeholder') {
      ok = String(el.placeholder || '').toLowerCase().indexOf(String(value).toLowerCase()) >= 0;
    } else {
      var t = (el.getAttribute('aria-label') || el.innerText || '').toLowerCase();
      ok = t.indexOf(String(value).toLowerCase()) >= 0;
    }
    if (ok) out.push({
      ref: ref,
      role: el.getAttribute('role'),
      name: (el.getAttribute('aria-label') || el.innerText || '').trim().slice(0, 80),
      visible: true
    });
  }
  window.__cc_find = out;
  return out;
})()
