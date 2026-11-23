// DOM op applier (ARCHITECTURE §3.3 family 1): node handles, listeners and event encoding. A dumb executor of
// rust/crates/ihn-app/src/dom.rs; ops on a missing node are ignored, like 5.18.0's `element?.…`.

const DOM = 1 << 8;
const KIND_DOM = 1, KIND_POINTER = 2;
const POINTER_TYPE = { mouse: 1, pen: 2, touch: 3 };
const HAS_LINEAR = typeof CSS !== 'undefined' && CSS.supports?.('animation-timing-function', 'linear(0, 1)');
const SPRING_OPEN = HAS_LINEAR
  ? 'linear(0,0.0311,0.1083,0.2117,0.3265,0.4422,0.5519,0.6511,0.7375,0.8104,0.87,0.9173,0.9536,0.9805,0.9996,1.0123,1.0201,1.0241,1.0254,1.0247,1.0229,1.0203,1.0174,1.0145,1.0116,1.0091,1.0069,1.005,1.0034,1.0022,1.0013,1.0006,1.0001,0.9997,0.9995,0.9994,0.9994,0.9994,0.9994,0.9995,1)'
  : 'cubic-bezier(0.22, 1.08, 0.32, 1)';
const SPRING_CLOSE = HAS_LINEAR
  ? 'linear(0,0.0285,0.0987,0.1923,0.2963,0.4014,0.5018,0.5937,0.6752,0.7456,0.8049,0.854,0.8938,0.9253,0.9499,0.9685,0.9824,0.9923,0.9992,1.0037,1.0065,1.008,1.0085,1.0084,1.0079,1.0071,1.0062,1.0053,1.0045,1.0037,1.0029,1.0023,1.0018,1.0013,1.001,1.0007,1.0005,1.0003,1.0002,1.0001,1)'
  : 'cubic-bezier(0.22, 1.03, 0.30, 1)';
const SPRING_SHEETS = HAS_LINEAR
  ? 'linear(0,0.038,0.1324,0.2583,0.3966,0.5333,0.6594,0.7691,0.86,0.9316,0.985,1.0223,1.0459,1.0587,1.0632,1.0617,1.0563,1.0486,1.0399,1.0312,1.0231,1.0159,1.0099,1.0051,1.0015,0.999,0.9973,0.9964,0.996,0.9961,0.9964,0.9968,0.9974,0.9979,0.9984,0.9989,0.9993,0.9996,0.9999,1,1)'
  : 'cubic-bezier(0.20, 1.15, 0.32, 1)';

export default function install(host) {
  // Handles are chosen by Rust: 1..4 fixed, 16.. the ids of index.html, 1024.. dynamic.
  const nodes = [null, document, window, document.documentElement, document.body];
  let longStyle = null; // the last long STYLE value (see the STYLE op)
  // listener id → Map(event type → entry): one id may serve several event types of one node (the document title's
  // focus/blur/keydown/pointerdown/click); listening again with an id and type replaces that pair only.
  const listeners = new Map();
  const observers = new Map();
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
    | (found.length && nodes[found[0]] === event.target ? 512 : 0)
    | (Array.from((event.dataTransfer || event.clipboardData)?.types || []).includes('Files') ? 1024 : 0);
  const sample = (w, e) => w.f64(e.clientX).f64(e.clientY).f64(e.timeStamp).f32(e.pressure).f32(e.tiltX).f32(e.tiltY).u32(e.buttons);
  const touches = (w, list) => {
    w.u32(list ? list.length : 0);
    for (let i = 0; list && i < list.length; i += 1) w.i32(list[i].identifier).f64(list[i].clientX).f64(list[i].clientY);
  };

  function encodePointer(id, h, kind, event) {
    let coalesced = null;
    if (kind === 2) { try { coalesced = event.getCoalescedEvents?.(); } catch (_) { /* v5: synthetic input */ } }
    const count = coalesced ? coalesced.length : 0;
    // Moves carry no target path, except a pen move with its side button held: with the button pressed in the air
    // first, touching the page arrives as a move (chorded buttons), and Rust starts the eraser contact from it.
    const found = (kind === 2 || kind === 9) && !(event.pointerType === 'pen' && event.buttons & 2) ? [] : path(event.target);
    const w = host.writer.begin(76 + count * 40 + found.length * 4);
    w.u32(id).u32(h).u8(kind).u8(POINTER_TYPE[event.pointerType] || 0).i32(event.pointerId).i32(event.button).u32(flags(event, found));
    sample(w, event);
    w.u32(count);
    for (let i = 0; i < count; i += 1) sample(w, coalesced[i]);
    w.u32(found.length);
    for (const handle of found) w.u32(handle);
    w.u32(0); // predicted samples: none (6.2.1, the predicted tip was removed)
    return w;
  }

  // `ty` = the `abi::ety` code Rust sent with the listen (or observe) for this event type.
  function encodeDom(id, h, event, ty) {
    const found = path(event.target), related = path(event.relatedTarget);
    const target = event.target;
    const key = event.key || '', code = event.code || '';
    // A contenteditable target (the document title) reports its text: it has no `value`.
    const raw = target && typeof target.value === 'string' ? target.value : target?.isContentEditable ? target.textContent : event.animationName || event.propertyName || '';
    const value = typeof raw === 'string' ? raw : '';
    const data = event.dataTransfer?.files?.length ? event.dataTransfer.files
      : event.clipboardData || (target?.files?.length ? target.files : null);
    const count = (event.touches?.length || 0) + (event.changedTouches?.length || 0);
    const w = host.writer.begin(112 + (key.length + code.length + value.length) * 3 + (found.length + related.length) * 4 + count * 20);
    w.u32(id).u32(h).u8(ty).u32(flags(event, found)).f64(event.timeStamp ?? performance.now())
      .i32(event.button ?? 0).u32(event.buttons || 0)
      .f64(event.clientX || 0).f64(event.clientY || 0).f64(event.deltaX || 0).f64(event.deltaY || 0)
      .u32(event.deltaMode || 0).i32(event.detail | 0).u8(POINTER_TYPE[event.pointerType] || 0).u32(host.hold(data))
      .str(key).str(code).str(value).u32(found.length);
    for (const handle of found) w.u32(handle);
    w.u32(related.length);
    for (const handle of related) w.u32(handle);
    touches(w, event.touches);
    touches(w, event.changedTouches);
    return w;
  }

  const dispatch = (id, h, kind, event, code) => host.enter(() => {
    const w = kind ? encodePointer(id, h, kind, event) : encodeDom(id, h, event, code);
    return host.ex.ihn_event(kind ? KIND_POINTER : KIND_DOM, w.base, w.length);
  });

  // `code` > 128: a pointer type (`abi::ptr` kind + 128), encoded as a pointer batch.
  function listen(h, type, id, bits, code) {
    const node = nodes[h];
    if (!node) return;
    const kind = code > 128 ? code - 128 : 0;
    const options = { capture: !!(bits & 1), once: !!(bits & 4) };
    if (bits & 2) options.passive = true;
    else if (bits & 8) options.passive = false;
    const fn = event => {
      if (options.once) listeners.get(id)?.delete(type);
      const result = dispatch(id, h, kind, event, code);
      if (!result) return;
      if (result & 1) event.preventDefault();
      if (result & 2) event.stopPropagation();
      if (result & 4) event.stopImmediatePropagation();
      if (result & 8 && event.dataTransfer) event.dataTransfer.dropEffect = 'copy';
    };
    let byType = listeners.get(id);
    if (!byType) listeners.set(id, byType = new Map());
    if (type === 'ready') {
      // An Animation's `ready` promise (there is no event): one dispatch when it resolves, unless unlistened before.
      const entry = { node, fn: null, capture: false };
      byType.set(type, entry);
      node.ready?.then(() => {
        if (listeners.get(id)?.get(type) !== entry) return;
        listeners.get(id).delete(type);
        dispatch(id, h, 0, { timeStamp: performance.now() }, code);
      }, () => {});
      return;
    }
    const old = byType.get(type);
    if (old) old.node.removeEventListener(type, old.fn, { capture: old.capture });
    byType.set(type, { node, fn, capture: options.capture });
    node.addEventListener(type, fn, options);
  }
  function unlisten(id) {
    const byType = listeners.get(id);
    if (!byType) return;
    for (const [type, entry] of byType) entry.node.removeEventListener(type, entry.fn, { capture: entry.capture });
    listeners.delete(id);
  }
  // One observer per listener id; further `observe` ops with that id add nodes to it. Each callback is one event.
  function observe(h, id, what, filter, code) {
    const node = nodes[h];
    if (!node) return;
    let observer = observers.get(id);
    if (!observer) {
      const Observer = what ? window.MutationObserver : window.ResizeObserver;
      if (!Observer) return; // v5: `typeof ResizeObserver !== 'undefined'`
      observer = new Observer(entries => dispatch(id, h, 0, { target: entries[0]?.target }, code));
      observers.set(id, observer);
    }
    if (!what) return observer.observe(node);
    const options = { attributes: !!(what & 1), childList: !!(what & 2), subtree: !!(what & 4), characterData: !!(what & 8) };
    if (filter) options.attributeFilter = filter.split(',');
    observer.observe(node, options);
  }
  function unobserve(id) {
    observers.get(id)?.disconnect();
    observers.delete(id);
  }
  // `Host::ask` (ops 16.. of the `host_intl` import): synchronous platform reads answered as text.
  host.ask = (op, fields) => {
    switch (op) {
      case 16: {
        const node = nodes[fields[0]];
        if (!(node instanceof Element)) return '';
        const style = getComputedStyle(node);
        return fields[1].includes('-') ? style.getPropertyValue(fields[1]) : String(style[fields[1]] ?? '');
      }
      case 17: return matchMedia(fields[0]).matches ? '1' : '0';
      case 18: return path(document.elementFromPoint(Number(fields[0]), Number(fields[1]))).join(',');
      case 19: return path(document.activeElement).join(',');
      case 20: return nodes[fields[0]]?.hasPointerCapture?.(Number(fields[1])) ? '1' : '0';
      default: return null;
    }
  };
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
    [DOM | 14]: r => {
      const node = nodes[r.u32()], name = r.str(), len = r.host.view().getUint32(r.p, true);
      // abi.rs SAME_LONG_STYLE: the last value of at least LONG_STYLE (16 KiB) bytes again.
      if (len === 0xfffffffe) { r.p += 4; return style(node, name, longStyle); }
      const value = r.ostr();
      if (value !== null && len >= 16384) longStyle = value;
      style(node, name, value);
    },
    [DOM | 15]: r => listen(r.u32(), r.str(), r.u32(), r.u8(), r.u8()),
    [DOM | 16]: r => unlisten(r.u32()),
    [DOM | 17]: r => { const node = nodes[r.u32()], preventScroll = r.u8() !== 0; node?.focus?.({ preventScroll }); },
    [DOM | 18]: r => nodes[r.u32()]?.blur?.(),
    [DOM | 19]: r => scrollTo(nodes[r.u32()], r.f64(), r.f64(), r.u8()),
    [DOM | 20]: r => {
      const h = r.u32(), node = nodes[r.u32()];
      let keyframes = JSON.parse(r.str()), options = JSON.parse(r.str());
      const ghost = node?.classList?.contains('drive-folder-ghost') ? node : node?.closest?.('.drive-folder-ghost');
      if (ghost) {
        const opening = ghost.dataset.direction !== 'close';
        const dur = opening ? 560 : 500;
        if (node === ghost) {
          options = { ...options, duration: dur, easing: opening ? SPRING_OPEN : SPRING_CLOSE };
          if (opening) {
            const c = ghost.querySelector(':scope > .drive-folder-contents');
            if (c) c.style.opacity = '0';
          }
          const sheets = ghost.querySelector(':scope > .drive-folder-sheets');
          if (sheets) {
            const slot = document.createElement('div');
            slot.className = 'drive-folder-sheets-slot';
            sheets.parentNode.insertBefore(slot, sheets);
            slot.appendChild(sheets);
            ghost._sheetsAnim = sheets.animate(
              opening
                ? [
                    { transform: 'translateY(0%)', offset: 0, easing: 'cubic-bezier(0.32, 0, 0.2, 1)' },
                    { transform: 'translateY(60%)', offset: 0.44 },
                    { transform: 'translateY(60%)', offset: 1 }
                  ]
                : [
                    { transform: 'translateY(60%)', offset: 0 },
                    { transform: 'translateY(60%)', offset: 0.52, easing: SPRING_SHEETS },
                    { transform: 'translateY(0%)', offset: 1 }
                  ],
              { duration: dur, fill: 'both' }
            );
          }
        } else if (node.classList.contains('drive-folder-cover')) {
          const closedT = (opening ? keyframes?.[0]?.transform : keyframes?.[1]?.transform) || 'rotateY(0deg)';
          keyframes = opening
            ? [
                { transform: closedT, offset: 0, easing: 'cubic-bezier(0.28, 0, 0.46, 1)' },
                { transform: 'rotateY(-92deg)', offset: 0.64 },
                { transform: 'rotateY(-92deg)', offset: 1 }
              ]
            : [
                { transform: 'rotateY(-92deg)', offset: 0 },
                { transform: 'rotateY(-92deg)', offset: 0.30, easing: 'cubic-bezier(0.20, 1.06, 0.28, 1)' },
                { transform: closedT, offset: 1 }
              ];
          options = { duration: dur, fill: 'both' };
        } else if (node.classList.contains('drive-folder-print') || node.classList.contains('drive-card-menu')) {
          set(h, null);
          return;
        } else if (node.classList.contains('drive-folder-view')) {
          const contents = node.parentElement;
          const bg = node.style.getPropertyValue('--folder-color');
          if (contents?.classList?.contains('drive-folder-contents')) {
            if (bg) contents.style.setProperty('--contents-bg', bg);
            if (opening) {
              keyframes = [{ opacity: 1 }, { opacity: 1 }];
              options = { ...options, duration: dur };
            } else {
              ghost._contentsAnim = contents.animate(
                [{ offset: 0, opacity: 1 }, { offset: 0.36, opacity: 0 }, { offset: 1, opacity: 0 }],
                { duration: dur, easing: 'linear', fill: 'both' }
              );
              set(h, null);
              return;
            }
          }
        }
      }
      set(h, node?.animate?.(keyframes, options));
    },
    [DOM | 21]: r => {
      const animation = nodes[r.u32()];
      quiet(() => {
        const ghost = animation?.effect?.target;
        if (ghost?._sheetsAnim) { ghost._sheetsAnim.cancel(); ghost._sheetsAnim = null; }
        if (ghost?._contentsAnim) { ghost._contentsAnim.cancel(); ghost._contentsAnim = null; }
        animation?.cancel();
      });
    },
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
    [DOM | 27]: r => nodes[r.u32()]?.select?.(),
    [DOM | 28]: r => observe(r.u32(), r.u32(), r.u8(), r.str(), r.u8()),
    [DOM | 29]: r => unobserve(r.u32()),
    [DOM | 30]: r => {
      const animation = nodes[r.u32()], play = r.u8(), keyframes = r.ostr(), sync = nodes[r.u32()];
      quiet(() => {
        const target = animation?.effect?.target;
        const ghost = target?.classList?.contains('drive-folder-view') ? target.closest('.drive-folder-ghost[data-direction="open"]') : null;
        if (ghost && keyframes !== null) {
          const contents = target.parentElement;
          const elapsed = sync ? Math.max(0, Math.min(1, (Number(sync.currentTime) || 0) / 560)) : 0;
          const from = Math.min(0.82, Math.max(0.16, elapsed + 0.02));
          const to = Math.min(1, Math.max(0.54, from + 0.20));
          const kf = [{ offset: 0, opacity: 0 }, { offset: from, opacity: 0 }, { offset: to, opacity: 1 }, { offset: 1, opacity: 1 }];
          animation.effect.setKeyframes([{ opacity: 1 }, { opacity: 1 }]);
          if (play === 1) animation.play();
          if (sync) animation.startTime = sync.startTime;
          if (contents?.classList?.contains('drive-folder-contents')) {
            if (ghost._contentsAnim) ghost._contentsAnim.cancel();
            const cAnim = contents.animate(kf, { duration: 560, easing: 'linear', fill: 'both' });
            if (sync && sync.startTime != null) cAnim.startTime = sync.startTime;
            ghost._contentsAnim = cAnim;
          }
          return;
        }
        if (keyframes !== null) animation?.effect?.setKeyframes(JSON.parse(keyframes));
        if (play === 1) animation?.play();
        else if (play === 2) animation?.pause();
        if (sync && animation) animation.startTime = sync.startTime;
      });
    }
  };
}
