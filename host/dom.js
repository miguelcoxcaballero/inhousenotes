// DOM op applier (ARCHITECTURE §3.3 family 1): node handles, listeners and event encoding. A dumb executor of
// rust/crates/ihn-app/src/dom.rs; ops on a missing node are ignored, like 5.18.0's `element?.…`.

const DOM = 1 << 8;
const KIND_DOM = 1, KIND_POINTER = 2;
const POINTER_KIND = {
  pointerdown: 1, pointermove: 2, pointerup: 3, pointercancel: 4, pointerleave: 5, pointerenter: 6, pointerover: 7,
  pointerout: 8, pointerrawupdate: 9, lostpointercapture: 10, gotpointercapture: 11
};
const POINTER_TYPE = { mouse: 1, pen: 2, touch: 3 };

export default function install(host) {
  // Handles are chosen by Rust: 1..4 fixed, 16.. the ids of index.html, 1024.. dynamic.
  const nodes = [null, document, window, document.documentElement, document.body];
  const listeners = new Map();
  // node → its handle, or an array when Rust holds several handles to one node (a `Ui` id that is also queried).
  const handles = new WeakMap();
  const forget = h => {
    const node = nodes[h], known = node && handles.get(node);
    if (known === h) handles.delete(node);
    else if (Array.isArray(known)) handles.set(node, known.filter(other => other !== h));
    nodes[h] = null;
  };
  const set = (h, node) => {
    if (!h) return;
    forget(h);
    if (!node) return;
    nodes[h] = node;
    const known = handles.get(node);
    handles.set(node, known === undefined ? h : [].concat(known, h));
  };
  host.node = h => nodes[h] || null;
  for (let h = 1; h < nodes.length; h += 1) handles.set(nodes[h], h);

  // Known handles from the event target upwards, nearest first.
  const path = target => {
    const found = [];
    for (let node = target; node; node = node.parentNode || node.host) {
      const known = handles.get(node);
      if (typeof known === 'number') found.push(known);
      else if (known) found.push(...known);
    }
    return found;
  };
  const flags = (event, found) => (event.ctrlKey ? 1 : 0) | (event.shiftKey ? 2 : 0) | (event.altKey ? 4 : 0)
    | (event.metaKey ? 8 : 0) | (event.isComposing ? 16 : 0) | (event.repeat ? 32 : 0)
    | (event.target?.checked === true ? 64 : 0) | (event.isTrusted ? 128 : 0) | (event.isPrimary ? 256 : 0)
    | (found.length && nodes[found[0]] === event.target ? 512 : 0);
  const sample = (w, e) => w.f64(e.clientX).f64(e.clientY).f64(e.timeStamp).f32(e.pressure).f32(e.tiltX).f32(e.tiltY).u32(e.buttons);
  const touches = (w, list) => {
    w.u32(list ? list.length : 0);
    for (let i = 0; list && i < list.length; i += 1) w.i32(list[i].identifier).f64(list[i].clientX).f64(list[i].clientY);
  };

  function encodePointer(id, h, kind, event) {
    let coalesced = null;
    if (kind === 2) { try { coalesced = event.getCoalescedEvents?.(); } catch (_) { /* v5: synthetic input */ } }
    const count = coalesced ? coalesced.length : 0;
    const found = kind === 2 || kind === 9 ? [] : path(event.target);
    const w = host.writer.begin(72 + count * 40 + found.length * 4);
    w.u32(id).u32(h).u8(kind).u8(POINTER_TYPE[event.pointerType] || 0).i32(event.pointerId).i32(event.button).u32(flags(event, found));
    sample(w, event);
    w.u32(count);
    for (let i = 0; i < count; i += 1) sample(w, coalesced[i]);
    w.u32(found.length);
    for (const handle of found) w.u32(handle);
    return w;
  }

  function encodeDom(id, h, event) {
    const found = path(event.target);
    const target = event.target;
    const key = event.key || '', code = event.code || '';
    const raw = target && typeof target.value === 'string' ? target.value : event.animationName || event.propertyName || '';
    const value = typeof raw === 'string' ? raw : '';
    const data = event.dataTransfer?.files?.length ? event.dataTransfer.files
      : event.clipboardData || (target?.files?.length ? target.files : null);
    const count = (event.touches?.length || 0) + (event.changedTouches?.length || 0);
    const w = host.writer.begin(96 + (key.length + code.length + value.length) * 3 + found.length * 4 + count * 20);
    w.u32(id).u32(h).u32(flags(event, found)).i32(event.button ?? 0).u32(event.buttons || 0)
      .f64(event.clientX || 0).f64(event.clientY || 0).f64(event.deltaX || 0).f64(event.deltaY || 0)
      .u32(event.deltaMode || 0).i32(event.detail | 0).u8(POINTER_TYPE[event.pointerType] || 0).u32(host.hold(data))
      .str(key).str(code).str(value).u32(found.length);
    for (const handle of found) w.u32(handle);
    touches(w, event.touches);
    touches(w, event.changedTouches);
    return w;
  }

  function listen(h, type, id, bits) {
    const node = nodes[h];
    if (!node) return;
    const kind = POINTER_KIND[type];
    const options = { capture: !!(bits & 1), once: !!(bits & 4) };
    if (bits & 2) options.passive = true;
    else if (bits & 8) options.passive = false;
    const fn = event => {
      if (options.once) listeners.delete(id);
      const result = host.enter(() => {
        const w = kind ? encodePointer(id, h, kind, event) : encodeDom(id, h, event);
        return host.ex.ihn_event(kind ? KIND_POINTER : KIND_DOM, w.base, w.length);
      });
      if (result & 1) event.preventDefault();
      if (result & 2) event.stopPropagation();
      if (result & 4) event.stopImmediatePropagation();
    };
    unlisten(id);
    listeners.set(id, { node, type, fn, capture: options.capture });
    node.addEventListener(type, fn, options);
  }
  function unlisten(id) {
    const entry = listeners.get(id);
    if (!entry) return;
    entry.node.removeEventListener(entry.type, entry.fn, { capture: entry.capture });
    listeners.delete(id);
  }
  function insertHtml(h, parent, before, html) {
    if (!parent) return set(h, null);
    const template = document.createElement('template');
    template.innerHTML = html;
    const first = template.content.firstElementChild;
    parent.insertBefore(template.content, before);
    set(h, first);
  }
  function style(node, name, value) {
    if (!node) return;
    if (name.includes('-')) {
      if (value === null) node.style.removeProperty(name);
      else node.style.setProperty(name, value);
    } else node.style[name] = value === null ? '' : value;
  }
  function scrollTo(node, x, y, behavior) {
    const options = {};
    if (!Number.isNaN(x)) options.left = x;
    if (!Number.isNaN(y)) options.top = y;
    if (behavior) options.behavior = behavior === 1 ? 'smooth' : 'instant';
    node?.scrollTo?.(options);
  }
  const quiet = fn => { try { fn(); } catch (_) { /* v5 swallows these too */ } };
  const prop = r => {
    const tag = r.u8();
    return tag === 0 ? r.str() : tag === 1 ? r.f64() : r.u8() !== 0;
  };

  return {
    [DOM | 1]: r => set(r.u32(), document.getElementById(r.str())),
    [DOM | 2]: r => { const h = r.u32(), root = nodes[r.u32()]; set(h, root?.querySelector?.(r.str())); },
    [DOM | 3]: r => set(r.u32(), document.createElement(r.str())),
    [DOM | 4]: r => insertHtml(r.u32(), nodes[r.u32()], nodes[r.u32()] || null, r.str()),
    [DOM | 5]: r => {
      const h = r.u32();
      let node = nodes[r.u32()];
      for (const index of r.bytes()) node = node?.children[index];
      set(h, node);
    },
    [DOM | 6]: r => { const parent = nodes[r.u32()], before = nodes[r.u32()] || null, node = nodes[r.u32()]; if (parent && node) parent.insertBefore(node, before); },
    [DOM | 7]: r => nodes[r.u32()]?.remove?.(),
    [DOM | 8]: r => forget(r.u32()),
    [DOM | 9]: r => {
      const node = nodes[r.u32()], name = r.str(), value = r.ostr();
      if (value === null) node?.removeAttribute(name);
      else node?.setAttribute(name, value);
    },
    [DOM | 10]: r => { const node = nodes[r.u32()], name = r.str(), value = prop(r); if (node) node[name] = value; },
    [DOM | 11]: r => { const node = nodes[r.u32()], text = r.str(); if (node) node.textContent = text; },
    [DOM | 12]: r => { const node = nodes[r.u32()], name = r.str(); node?.classList.toggle(name, r.u8() !== 0); },
    [DOM | 13]: r => { const node = nodes[r.u32()], value = r.str(); if (node) node.className = value; },
    [DOM | 14]: r => style(nodes[r.u32()], r.str(), r.ostr()),
    [DOM | 15]: r => listen(r.u32(), r.str(), r.u32(), r.u8()),
    [DOM | 16]: r => unlisten(r.u32()),
    [DOM | 17]: r => { const node = nodes[r.u32()], preventScroll = r.u8() !== 0; node?.focus?.({ preventScroll }); },
    [DOM | 18]: r => nodes[r.u32()]?.blur?.(),
    [DOM | 19]: r => scrollTo(nodes[r.u32()], r.f64(), r.f64(), r.u8()),
    [DOM | 20]: r => {
      const h = r.u32(), node = nodes[r.u32()], keyframes = JSON.parse(r.str()), options = JSON.parse(r.str());
      set(h, node?.animate?.(keyframes, options));
    },
    [DOM | 21]: r => { const animation = nodes[r.u32()]; quiet(() => animation?.cancel()); },
    [DOM | 22]: r => { const animation = nodes[r.u32()]; quiet(() => animation?.finish()); },
    [DOM | 23]: r => {
      const node = nodes[r.u32()], id = r.i32(), on = r.u8() !== 0;
      quiet(() => (on ? node?.setPointerCapture(id) : node?.releasePointerCapture(id)));
    },
    [DOM | 24]: r => {
      const node = nodes[r.u32()], action = r.u8();
      quiet(() => (action === 0 ? node?.show() : action === 1 ? node?.showModal() : node?.close()));
    },
    [DOM | 25]: r => nodes[r.u32()]?.click?.(),
    [DOM | 26]: r => { const node = nodes[r.u32()], html = r.str(); if (node) node.innerHTML = html; },
    [DOM | 27]: r => nodes[r.u32()]?.select?.()
  };
}
