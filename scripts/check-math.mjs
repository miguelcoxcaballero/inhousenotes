// The exact-maths gate of ARCHITECTURE §6.5: `ihn-core` and `ihn-app` may not call the std f64/f32 transcendental
// methods, whose wasm implementation (musl libm) differs from Chromium's in the last bit. Everything goes through
// `ihn_core::jsmath` (the only file exempt). `npm run build` runs this before cargo; it also checks itself by
// planting a `.sin()` in a temporary tree on every run.
//
//   node scripts/check-math.mjs [dir ...]     default: rust/crates/ihn-core/src rust/crates/ihn-app/src
import { mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const DEFAULT_DIRS = ['rust/crates/ihn-core/src', 'rust/crates/ihn-app/src'].map(dir => join(root, dir));
const EXEMPT = 'jsmath.rs';
// f64 methods that are not correctly rounded by IEEE 754 (`sqrt`, `abs`, `floor`, `ceil`, `trunc`, `round` are).
// `mul_add` is banned too: Rust must never contract `a * b + c`, and nothing outside jsmath needs a fused multiply.
const BANNED = ['sin', 'cos', 'tan', 'asin', 'acos', 'atan', 'atan2', 'sin_cos', 'sinh', 'cosh', 'tanh', 'asinh', 'acosh', 'atanh',
  'exp', 'exp2', 'exp_m1', 'ln', 'ln_1p', 'log', 'log2', 'log10', 'powf', 'powi', 'hypot', 'cbrt', 'mul_add'];
const CALL = new RegExp(`(?:\\.\\s*|\\bf(?:64|32)\\s*::\\s*)(${BANNED.join('|')})\\s*\\(`, 'g');

// Blanks out comments and fills string and char literals with `_` (keeping newlines), so `// v5: Math.sin(a)` and
// "a.cos(b)" in a message do not trip the gate.
export function stripRust(source) {
  let out = '';
  let i = 0;
  while (i < source.length) {
    const rest = source.slice(i);
    let m;
    let fill = '_'; // a literal stays an argument
    if (rest.startsWith('//')) { m = /^\/\/[^\n]*/.exec(rest)[0]; fill = ' '; }
    else if (rest.startsWith('/*')) {
      let depth = 0, j = 0;
      do {
        if (rest.startsWith('/*', j)) { depth++; j += 2; } else if (rest.startsWith('*/', j)) { depth--; j += 2; } else j++;
      } while (depth && j < rest.length);
      m = rest.slice(0, j);
      fill = ' ';
    } else if ((m = /^b?r(#*)"/.exec(rest))) {
      const end = rest.indexOf(`"${m[1]}`, m[0].length);
      m = end < 0 ? rest : rest.slice(0, end + 1 + m[1].length);
    } else if ((m = /^b?"(?:[^"\\]|\\[\s\S])*"/.exec(rest))) m = m[0];
    else if ((m = /^b?'(?:[^'\\\n]|\\(?:x[0-9a-fA-F]{2}|u\{[0-9a-fA-F_]+\}|[^xu]))'/.exec(rest))) m = m[0];
    else { out += source[i++]; continue; }
    out += m.replace(/[^\n]/g, fill);
    i += m.length;
  }
  return out;
}

// `x.log(base)` is banned, `host.log(level, "text")` is a logger: tell them apart by the argument count.
function oneArgument(code, open) {
  let depth = 0;
  for (let i = open; i < code.length; i++) {
    const c = code[i];
    if ('([{'.includes(c)) depth++;
    else if (')]}'.includes(c) && --depth === 0) return true;
    else if (c === ',' && depth === 1 && !/^\s*\)/.test(code.slice(i + 1))) return false;
  }
  return true;
}

export function findCalls(source) {
  const code = stripRust(source);
  const hits = [];
  for (const match of code.matchAll(CALL)) {
    if (match[1] === 'log' && !oneArgument(code, match.index + match[0].length - 1)) continue;
    const line = code.slice(0, match.index).split('\n').length;
    hits.push({ line, call: match[1], text: source.split('\n')[line - 1].trim() });
  }
  return hits;
}

function rustFiles(dir) {
  return readdirSync(dir, { withFileTypes: true }).sort((a, b) => (a.name < b.name ? -1 : 1)).flatMap(entry => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return rustFiles(path);
    return entry.name.endsWith('.rs') && entry.name !== EXEMPT ? [path] : [];
  });
}

export function check(dirs) {
  return dirs.flatMap(dir => rustFiles(dir).flatMap(file => findCalls(readFileSync(file, 'utf8')).map(hit => ({ file, ...hit }))));
}

// The gate must reject planted calls and accept look-alikes, or it protects nothing.
function selfTest() {
  const dir = mkdtempSync(join(tmpdir(), 'ihn-check-math-'));
  try {
    writeFileSync(join(dir, 'jsmath.rs'), 'pub fn sin(x: f64) -> f64 { x.sin() }\n');
    writeFileSync(join(dir, 'clean.rs'), [
      '// v5: Math.sin(a) + b.cos()', '/* x.exp() /* nested x.ln() */ y.tan() */', 'fn f(h: &H, x: f64) -> f64 {',
      '    h.log(2, "a.powf(b) .hypot(");', '    let _ = (r#"x.sin()"#, \'(\', b\'"\', "\\".cos(");', "    let _l: &'static str = \"\";",
      '    crate::jsmath::sin(x) + jsmath::pow(x, 2.0) + x.sqrt().abs().floor() + x.expect_it().login(1)', '}', ''].join('\n'));
    if (check([dir]).length) throw new Error(`self-test: clean code rejected: ${JSON.stringify(check([dir]))}`);
    const planted = ['a.sin()', 'a . cos ()', 'a\n        .tan()', 'f64::atan2(a, b)', 'a.exp()', 'a.ln()', 'a.log(10.0)', 'a.log2()', 'a.log10()',
      'a.powf(b)', 'a.powi(2)', 'a.hypot(b)', 'f32::sin(a)', 'a.mul_add(b, c)', '(a * 2.0).sin_cos()', 'a.log(b.max(c),)'];
    for (const call of planted) {
      writeFileSync(join(dir, 'planted.rs'), `fn f(a: f64, b: f64, c: f64) {\n    let _ = "ok"; // fine\n    let _ = ${call};\n}\n`);
      const hits = check([dir]);
      if (hits.length !== 1 || hits[0].line < 3) throw new Error(`self-test: planted \`${call}\` not rejected (${JSON.stringify(hits)})`);
    }
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  selfTest();
  const dirs = process.argv.length > 2 ? process.argv.slice(2) : DEFAULT_DIRS;
  const hits = check(dirs);
  for (const hit of hits) {
    console.error(`${relative(process.cwd(), hit.file)}:${hit.line}: .${hit.call}() is not Chromium-exact — use ihn_core::jsmath (ARCHITECTURE §6.5)\n    ${hit.text}`);
  }
  if (hits.length) process.exit(1);
  console.log(`check-math: ok (${dirs.length} dirs, no std transcendental calls outside ${EXEMPT})`);
}
