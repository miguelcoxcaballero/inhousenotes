// W1.3's shell-only comparison, reproducible from the branch:
//
//   IHN_SHELL_ONLY=1 node tests/v6/parity/run.mjs --quick --states auth,profile-menu,updater
//
// Home's list sections (`#drive-home .drive-section`: skeleton cards, lists, empty messages) are rendered by W4.1 and
// by the offline library, not by the shell. Until those have merged, every state that shows Home differs there. With
// the switch on, the sections are taken out of layout in both apps for the duration of each screenshot, so what is
// compared is everything else: top bar, search row, navigation, profile menu, toast, updater. The DOM dump and the
// canvas trace are taken with the sections hidden too.
//
// This is a diagnostic, not the gate: W1.3's Home parity items are met only by the run WITHOUT the switch
// (docs/v6/NOTES-W1.3.md). The harness itself is not modified; state files opt in by wrapping their script.
export const SHELL_ONLY = process.env.IHN_SHELL_ONLY === '1';
const SECTIONS = '#drive-home .drive-section';
if (SHELL_ONLY) console.log(`IHN_SHELL_ONLY=1: ${SECTIONS} is left out of every capture of the wrapped states — diagnostic run, not the gate.`);

/** Wraps a state script: with IHN_SHELL_ONLY=1 every capture (explicit or the final one) hides Home's sections. */
export function shellOnly(script) {
  if (!SHELL_ONLY) return script;
  return async t => {
    const capture = t.capture.bind(t);
    const set = value => t.page.evaluate(([selector, display]) => document.querySelectorAll(selector).forEach(node => { node.style.display = display; }), [SECTIONS, value]);
    t.capture = async label => {
      await set('none');
      try { await capture(label); } finally { await set('').catch(() => {}); }
    };
    await script(t);
  };
}
