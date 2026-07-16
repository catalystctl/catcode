(function(){
  var maxEl = 1000;
  var elements = [];
  var i = 0;
  function vis(el){
    var st = window.getComputedStyle(el);
    if (st.display === 'none' || st.visibility === 'hidden' || st.opacity === '0') return false;
    var r = el.getBoundingClientRect();
    return r.width > 0 && r.height > 0;
  }
  function walk(el){
    if (!el || elements.length >= maxEl) return;
    var tag = (el.tagName || '').toLowerCase();
    if (!tag || tag === 'script' || tag === 'style' || tag === 'noscript') {
      for (var c = 0; c < (el.children || []).length; c++) walk(el.children[c]);
      return;
    }
    var role = el.getAttribute('role');
    var interactive = ['a','button','input','textarea','select','option'].indexOf(tag) >= 0
      || role === 'button' || role === 'link' || role === 'textbox' || role === 'checkbox'
      || el.tabIndex >= 0;
    var text = (el.innerText || '').trim();
    if (interactive || (!el.children.length && text)) {
      var ref = 'e' + (++i);
      try { el.setAttribute('data-catalyst-ref', ref); } catch (e) {}
      var typ = el.type || null;
      var value = null;
      if ('value' in el) value = (typ === 'password') ? '[REDACTED]' : String(el.value || '');
      var r = el.getBoundingClientRect();
      elements.push({
        ref: ref, tag: tag, role: role,
        name: el.getAttribute('aria-label') || el.getAttribute('name') || text.slice(0, 80) || null,
        type: typ, value: value, placeholder: el.placeholder || null,
        required: !!el.required, disabled: !!el.disabled, visible: vis(el),
        focused: document.activeElement === el, href: el.href || null,
        bounds: { x: r.x, y: r.y, width: r.width, height: r.height }
      });
    }
    for (var j = 0; j < (el.children || []).length; j++) walk(el.children[j]);
  }
  if (document.body) walk(document.body);
  var snap = {
    url: location.href,
    title: document.title,
    text: (document.body && document.body.innerText || '').slice(0, 50000),
    elements: elements,
    document_state: {
      ready_state: document.readyState,
      scroll_x: window.scrollX,
      scroll_y: window.scrollY,
      viewport_width: window.innerWidth,
      viewport_height: window.innerHeight
    }
  };
  window.__cc_snap = snap;
  return snap;
})()
