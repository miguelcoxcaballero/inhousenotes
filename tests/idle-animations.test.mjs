// Guards against CSS animations that keep the GPU (or worse, the main thread)
// producing frames while nothing on screen needs them. An infinite animation
// keeps running for as long as its element is rendered, even at opacity 0, so
// the rules below are about what may loop forever and when.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const html = readFileSync('index.html', 'utf8');
const css = [...html.matchAll(/<style>([\s\S]*?)<\/style>/g)].map(m => m[1]).join('\n')
  + '\n' + readFileSync('offline-v5.css', 'utf8');

// Minimal CSS reader: rules (with the @media they sit in) and @keyframes.
function parse(source) {
  const text = source.replace(/\/\*[\s\S]*?\*\//g, '');
  const rules = [], keyframes = new Map();
  const walk = (start, end, media) => {
    let i = start;
    while (i < end) {
      const open = text.indexOf('{', i);
      if (open < 0 || open >= end) break;
      const prelude = text.slice(i, open).trim();
      let depth = 1, j = open + 1;
      while (depth && j < end) { if (text[j] === '{') depth++; else if (text[j] === '}') depth--; j++; }
      const body = text.slice(open + 1, j - 1);
      if (prelude.startsWith('@keyframes')) keyframes.set(prelude.split(/\s+/)[1], body);
      else if (prelude.startsWith('@media') || prelude.startsWith('@supports')) walk(open + 1, j - 1, `${media} ${prelude}`.trim());
      else if (!prelude.startsWith('@')) rules.push({ selector: prelude.replace(/\s+/g, ' '), body, media });
      i = j;
    }
  };
  walk(0, text.length, '');
  return { rules, keyframes };
}
const { rules, keyframes } = parse(css);
const decls = body => {
  const out = new Map();
  for (const part of body.split(';')) {
    const k = part.indexOf(':'); if (k < 0) continue;
    out.set(part.slice(0, k).trim(), part.slice(k + 1).trim().replace(/\s*!important$/, ''));
  }
  return out;
};
const animatedProperties = name => {
  const body = keyframes.get(name); assert.ok(body, `@keyframes ${name} exists`);
  const props = new Set();
  for (const m of body.matchAll(/\{([^{}]*)\}/g)) for (const p of decls(m[1]).keys()) if (p !== 'animation-timing-function') props.add(p);
  return props;
};
const COMPOSITOR = new Set(['transform', 'opacity']);
// Every animation a rule starts: names with their iteration counts.
function animationsOf(d) {
  const shorthand = d.get('animation');
  if (shorthand) {
    return shorthand.split(/,(?![^(]*\))/).map(part => {
      const words = part.trim().split(/\s+(?![^(]*\))/);
      const name = words.find(w => keyframes.has(w)) || (words.includes('none') ? 'none' : null);
      const count = words.includes('infinite') ? Infinity : Number(words.find(w => /^\d+$/.test(w)) || 1);
      const time = words.find(w => /^[\d.]+m?s$/.test(w));
      const seconds = time ? parseFloat(time) / (time.endsWith('ms') ? 1000 : 1) : undefined;
      return { name, count, seconds };
    });
  }
  if (d.has('animation-name')) {
    const counts = (d.get('animation-iteration-count') || '').split(',').map(s => s.trim());
    return d.get('animation-name').split(',').map((n, i) => ({ name: n.trim(), count: counts[i] === 'infinite' ? Infinity : counts[i] ? Number(counts[i]) : undefined }));
  }
  return [];
}

test('an infinite animation only moves transform or opacity, so it never repaints', () => {
  const offenders = [];
  for (const rule of rules) {
    for (const { name, count } of animationsOf(decls(rule.body))) {
      if (!name || name === 'none' || count !== Infinity) continue;
      const paint = [...animatedProperties(name)].filter(p => !COMPOSITOR.has(p));
      if (paint.length) offenders.push(`${rule.selector} -> ${name} (${paint.join(', ')})`);
    }
  }
  assert.deepEqual(offenders, []);
});

test('placeholder shimmers and the peer search pulse are bounded', () => {
  const bounded = ['drive-skeleton-shimmer', 'drive-skeleton-slide', 'presence-peer-connecting', 'presence-peer-connecting-scale', 'presence-peer-connecting-glow'];
  let seen = 0;
  for (const rule of rules) {
    const d = decls(rule.body);
    const counts = (d.get('animation-iteration-count') || '').split(',').map(s => s.trim()).filter(Boolean);
    for (const { name, count, seconds } of animationsOf(d)) {
      if (!bounded.includes(name)) continue;
      seen++;
      const n = count ?? (counts.length ? Number(counts[0]) : 1);
      // The pulse and halo take their shared 2.6s duration from the rule above them.
      const cycle = seconds ?? 2.6;
      assert.ok(Number.isFinite(n) && n >= 1 && n * cycle <= 61, `${rule.selector}: ${name} runs ${n} x ${cycle}s`);
    }
  }
  // Pulse (::before ring, ::after halo), the Home placeholders and the
  // calendar/share placeholders all declare one.
  assert.ok(seen >= 8, `found ${seen} bounded shimmer/pulse declarations`);
  const pulse = rules.find(r => r.selector.includes('.presence-avatar-wrapper.p2p-connecting::before,') && !r.media);
  assert.ok(pulse, 'shared pulse timing rule');
  const d = decls(pulse.body);
  assert.equal(d.get('animation-iteration-count'), '23');
  assert.equal(d.get('animation-duration'), '2.6s');
  assert.equal(d.get('animation-fill-mode'), 'forwards');
});

test('the peer search pulse runs on the compositor and keeps its static reduced-motion look', () => {
  for (const name of ['presence-peer-connecting', 'presence-peer-connecting-scale', 'presence-peer-connecting-glow']) {
    for (const p of animatedProperties(name)) assert.ok(COMPOSITOR.has(p), `${name} animates ${p}`);
  }
  const ring = rules.find(r => r.selector === '.presence-avatar-wrapper.p2p-connecting::before' && !r.media);
  assert.equal(decls(ring.body).get('animation-name'), 'presence-peer-connecting');
  assert.equal(decls(ring.body).get('will-change'), undefined, 'no permanent layer hint');
  const halo = rules.find(r => r.selector === '.presence-avatar-wrapper.p2p-connecting::after' && !r.media);
  assert.equal(decls(halo.body).get('animation-name'), 'presence-peer-connecting-scale, presence-peer-connecting-glow');
  const reduced = rules.filter(r => r.media.includes('prefers-reduced-motion') && r.selector.includes('p2p-connecting'));
  const reducedRing = decls(reduced.find(r => r.selector.endsWith('::before')).body);
  assert.equal(reducedRing.get('animation'), 'none');
  assert.match(reducedRing.get('box-shadow'), /0 0 9px rgba\(66, 133, 244, 0\.42\)/);
  assert.equal(decls(reduced.find(r => r.selector.endsWith('::after')).body).get('content'), 'none');
});

test('the pull-to-refresh spinner spins only while it can be seen', () => {
  const base = rules.find(r => r.selector === '.home-pull-refresh-spinner' && !r.media);
  const [first] = animationsOf(decls(base.body));
  assert.equal(first.name, 'none', 'no animation while Home rests with the indicator at opacity 0');
  const active = rules.filter(r => decls(r.body).get('animation-name') === 'home-pull-refresh-spin');
  assert.equal(active.length, 1);
  assert.deepEqual(active[0].selector.split(',').map(s => s.trim()).sort(), [
    '#drive-home.home-is-pulling .home-pull-refresh-spinner',
    '#drive-home.home-is-refreshing .home-pull-refresh-spinner',
    '#drive-home.home-pull-resetting .home-pull-refresh-spinner'
  ]);
  // Reduced motion still slows it down: the state rule names it without resetting the duration.
  assert.ok(rules.some(r => r.media.includes('prefers-reduced-motion') && r.selector === '.home-pull-refresh-spinner' && decls(r.body).get('animation-duration') === '1.2s'));
});

test('Home placeholders slide one exact pattern period on the compositor', () => {
  assert.deepEqual([...animatedProperties('drive-skeleton-slide')], ['transform']);
  assert.match(keyframes.get('drive-skeleton-slide'), /0%\s*\{\s*transform:\s*translateX\(-240px\);\s*\}\s*100%\s*\{\s*transform:\s*translateX\(240px\);\s*\}/);
  const strips = rules.filter(r => !r.media && animationsOf(decls(r.body)).some(a => a.name === 'drive-skeleton-slide'));
  const selectors = strips.flatMap(r => r.selector.split(',').map(s => s.trim()));
  for (const s of ['.drive-card-preview.is-loading::after', '.drive-status-skeleton::after', '.drive-card-skeleton .skeleton-bar::after', '.drive-card-skeleton .skeleton-preview::after', '.drive-folder-card-skeleton .skeleton-bar::after', '.drive-folder-card-skeleton .skeleton-icon::after']) {
    assert.ok(selectors.includes(s), `${s} slides`);
  }
  for (const r of strips) {
    const d = decls(r.body);
    // Starts 480px (one gradient period) left so the pattern lines up with the
    // box, and reaches 240px past it so the -240..240px slide always covers it.
    assert.equal(d.get('left'), '-480px', r.selector);
    assert.equal(d.get('right'), '-240px', r.selector);
    assert.equal(d.get('background-size'), '480px 100%', r.selector);
  }
  // The strips are clipped by their placeholder, which no longer paints the gradient itself.
  const boxes = rules.find(r => !r.media && r.selector.startsWith('.drive-skeleton-shimmer, .drive-card-skeleton .skeleton-bar,') && !r.selector.includes('::after'));
  assert.equal(decls(boxes.body).get('overflow'), 'hidden');
  assert.equal(decls(boxes.body).get('background'), undefined);
  const preview = rules.find(r => r.selector === '.drive-card-preview' && !r.media);
  assert.equal(decls(preview.body).get('overflow'), 'hidden');
  // A moving folder scene still holds them still.
  const hold = rules.find(r => decls(r.body).get('animation-play-state') === 'paused' && r.selector.includes('.drive-folder-transition .skeleton-bar::after'));
  assert.ok(hold, 'folder transition pauses the sliding strips');
});

test('reduced motion stops every placeholder shimmer', () => {
  const reduced = rules.filter(r => r.media.includes('prefers-reduced-motion') && decls(r.body).get('animation') === 'none');
  const selectors = reduced.flatMap(r => r.selector.split(',').map(s => s.trim()));
  for (const rule of rules) {
    if (rule.media) continue;
    if (!animationsOf(decls(rule.body)).some(a => a.name === 'drive-skeleton-shimmer' || a.name === 'drive-skeleton-slide')) continue;
    for (const s of rule.selector.split(',').map(x => x.trim()).filter(x => !x.startsWith('[data-theme'))) {
      assert.ok(selectors.includes(s), `${s} has a reduced-motion stop`);
    }
  }
});
