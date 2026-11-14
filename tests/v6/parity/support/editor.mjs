// Expected-difference rules shared by the editor states (TESTING §3.5). Each rule is `{tags, expected}`; `rules()`
// merges several, so every differing pixel must lie in the rectangles of one of them.

/** CSS-pixel rectangles of the elements matched by `selector`, padded. */
export const boxesOf = (selector, pad = 0) => t => t.page.evaluate(([s, p]) => [...document.querySelectorAll(s)].map(n => n.getBoundingClientRect()).filter(r => r.width)
  .map(r => ({ x: r.x - p, y: r.y - p, width: r.width + 2 * p, height: r.height + 2 * p })), [selector, pad]);

/** The version label of an open profile menu (v5.18.0 → v6.0.0). */
export const VERSION = { tags: ['VERSION-LABEL'], expected: boxesOf('[data-app-version]') };

/**
 * N-1: after an edit 5.18.0 uploads the PDF within ~0.4 s (save dot `saved`, green) while v6 seals the edit in the
 * relay once the device is idle (`paused` until then). Only the dot may differ; states that end after an edit
 * advance past the relay flush instead (`settleSave`).
 */
export const SAVE_DOT = { tags: ['N-1'], expected: boxesOf('#save-indicator', 2) };

/** Both apps settled after an edit (5.18.0 upload done, v6 relay sealed: SYNC §9.2 flush 5 s after the last batch). */
export const settleSave = t => t.advance(6000);

/** Several rules in one state. */
export function rules(...list) {
  return {
    tags: list.flatMap(r => r.tags),
    expected: async t => (await Promise.all(list.map(r => r.expected(t)))).flat(),
  };
}
