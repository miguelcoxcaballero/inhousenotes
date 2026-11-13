// The exact-maths gate of ARCHITECTURE §6.5: no crate may reach the std f64/f32 functions whose result differs from
// Chromium's — the transcendental ones (the wasm target links musl libm, last-bit differences) and the ones whose
// definition differs from JavaScript's (`round`: Rust rounds ties away from zero, `Math.round` towards +infinity;
// `to_degrees`/`to_radians`: Rust multiplies by a constant, 5.18.0 writes `a * 180 / Math.PI`). Everything goes
// through `ihn_core::jsmath`; the only exempt file is rust/crates/ihn-core/src/jsmath.rs (that exact path, not the
// name). `npm run build` runs this before cargo; it also checks itself on every run by planting every known
// spelling of a banned call in a temporary tree.
//
//   node scripts/check-math.mjs [dir ...]     default: the src directory of every crate in rust/crates
import { existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const DEFAULT_DIRS = ['ihn-core', 'ihn-app', 'ihn-sync', 'ihn-wasm'].map(crate => join(root, 'rust/crates', crate, 'src'));
const EXEMPT = resolve(root, 'rust/crates/ihn-core/src/jsmath.rs');
// Not correctly rounded by IEEE 754 (`sqrt`, `abs`, `floor`, `ceil`, `trunc` are, and stay allowed). `mul_add` is
// banned too: Rust must never contract `a * b + c`, and nothing outside jsmath needs a fused multiply.
const INEXACT = ['sin', 'cos', 'tan', 'asin', 'acos', 'atan', 'atan2', 'sin_cos', 'sinh', 'cosh', 'tanh', 'asinh', 'acosh', 'atanh',
  'exp', 'exp2', 'exp_m1', 'ln', 'ln_1p', 'log', 'log2', 'log10', 'powf', 'powi', 'hypot', 'cbrt', 'mul_add'];
// Exact, but not what the JavaScript of 5.18.0 computes.
const NOT_JS = {
  round: '`Math.round` rounds ties towards +infinity, Rust away from zero — use jsmath::round',
  round_ties_even: '`Math.round` rounds ties towards +infinity — use jsmath::round',
  to_degrees: 'JS computes `a * 180 / Math.PI` — write `a * 180.0 / PI`',
  to_radians: 'JS computes `a * Math.PI / 180` — write `a * PI / 180.0`',
};
const BANNED = [...INEXACT, ...Object.keys(NOT_JS)];
const NAMES = BANNED.join('|');
// `x.sin()`: a method call. (A method cannot be named without calling it.)
const METHOD = new RegExp(`\\.\\s*(${NAMES})\\s*\\(`, 'g');
// `f64::sin`, `F::sin` (alias), `<f64>::sin`, `<f64 as Float>::sin`: a path, called or not (`.map(f64::sin)`,
// `let g = f64::powf;`, `f64::exp as fn(f64) -> f64`). `jsmath::sin` and `crate::jsmath::sin` are not matched.
const QUALIFIERS = ['f64', 'f32', 'Float', 'FloatCore', 'Real'];
const pathCall = qualifiers => new RegExp(`(?:(?<![A-Za-z0-9_])(?:${qualifiers.join('|')})|>)\\s*::\\s*(${NAMES})(?![A-Za-z0-9_])`, 'g');
// Other doors to a libm: the `libm` crate, the unstable `core::f64::math` module, the `sinf64`-style intrinsics.
const OTHER = new RegExp(`(?<![A-Za-z0-9_])(libm\\s*::|f(?:64|32)\\s*::\\s*math(?![A-Za-z0-9_])|(?:${NAMES}|pow|fma)f(?:64|32)(?![A-Za-z0-9_]))`, 'g');
// Names that mean f64/f32 in this tree: `type F = f64;`, `use std::primitive::f64 as F;`, `<f64 as Trait>`.
const ALIAS = /(?<![A-Za-z0-9_])type\s+([A-Za-z_]\w*)\s*=\s*(?:[\w:\s]*::\s*)?f(?:64|32)\s*;|(?<![A-Za-z0-9_])f(?:64|32)\s+as\s+([A-Za-z_]\w*)/g;

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

export function aliases(source) {
  return [...stripRust(source).matchAll(ALIAS)].map(match => match[1] || match[2]);
}

export function findCalls(source, extraQualifiers = []) {
  const code = stripRust(source);
  const lines = source.split('\n');
  const hits = [];
  const add = (index, call, what, why) => {
    const line = code.slice(0, index).split('\n').length;
    hits.push({ line, call, what, why: why || NOT_JS[call] || 'not Chromium-exact — use ihn_core::jsmath', text: lines[line - 1].trim() });
  };
  for (const match of code.matchAll(METHOD)) {
    if (match[1] === 'log' && !oneArgument(code, match.index + match[0].length - 1)) continue;
    add(match.index, match[1], `.${match[1]}()`);
  }
  const qualifiers = [...new Set([...QUALIFIERS, ...extraQualifiers, ...aliases(source)])];
  for (const match of code.matchAll(pathCall(qualifiers))) add(match.index, match[1], match[0].replace(/\s+/g, ''));
  for (const match of code.matchAll(OTHER)) add(match.index, match[1], match[1].replace(/\s+/g, ''), 'a libm outside jsmath is not Chromium-exact — use ihn_core::jsmath');
  return hits.sort((a, b) => a.line - b.line);
}

function rustFiles(dir, exempt) {
  return readdirSync(dir, { withFileTypes: true }).sort((a, b) => (a.name < b.name ? -1 : 1)).flatMap(entry => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return rustFiles(path, exempt);
    return entry.name.endsWith('.rs') && resolve(path) !== exempt ? [path] : [];
  });
}

// An alias declared in one file can be used in another (`crate::F::ln`), so aliases are collected over the whole
// scan first (the exempt file included: it could export one).
export function check(dirs, exempt = EXEMPT) {
  const files = dirs.flatMap(dir => rustFiles(dir, null)).map(file => ({ file, source: readFileSync(file, 'utf8') }));
  const shared = files.flatMap(({ source }) => aliases(source));
  return files.filter(({ file }) => resolve(file) !== exempt).flatMap(({ file, source }) => findCalls(source, shared).map(hit => ({ file, ...hit })));
}

// The gate must reject planted calls and accept look-alikes, or it protects nothing.
function selfTest() {
  const dir = mkdtempSync(join(tmpdir(), 'ihn-check-math-'));
  const body = call => `fn f(a: f64, b: f64, c: f64, v: &[f64]) {\n    let _ = "ok"; // fine\n    ${call.includes(';') ? call : `let _ = ${call};`}\n}\n`;
  try {
    writeFileSync(join(dir, 'clean.rs'), [
      '// v5: Math.sin(a) + b.cos() + f64::sin + libm::sin(a) + a.round()', '/* x.exp() /* nested x.ln() */ y.tan() */',
      'use crate::jsmath::{self, round};', 'type Px = f64;', 'struct S { round: f64, sin: f64 }',
      'fn f(h: &H, x: f64, s: &S, v: &[f64]) -> f64 {',
      '    h.log(2, "a.powf(b) .hypot( f64::cos");', '    let _ = (r#"x.sin() f64::powf"#, \'(\', b\'"\', "\\".cos(");', "    let _l: &'static str = \"\";",
      '    let g = jsmath::cos; let _ = v.iter().copied().map(jsmath::sin).map(crate::jsmath::round);',
      '    let _ = (s.round, s.sin, x as f64, f64::from_bits(1), f64::sqrt(x), Px::EPSILON, <f64>::abs(x), f64::MAX_EXP, f64::consts::LN_2);',
      '    let _ = (x.rounded(), x.round_up(), x.exponent(), mathf64, sinful(x), Self::login(), exp, log2);',
      '    g(x) + round(x) + crate::jsmath::sin(x) + jsmath::pow(x, 2.0) + x.sqrt().abs().floor().ceil().trunc() + x.expect_it().login(1)', '}', ''].join('\n'));
    if (check([dir]).length) throw new Error(`self-test: clean code rejected: ${JSON.stringify(check([dir]))}`);
    const planted = ['a.sin()', 'a . cos ()', 'a\n        .tan()', 'f64::atan2(a, b)', 'a.exp()', 'a.ln()', 'a.log(10.0)', 'a.log2()', 'a.log10()',
      'a.powf(b)', 'a.powi(2)', 'a.hypot(b)', 'f32::sin(a)', 'a.mul_add(b, c)', '(a * 2.0).sin_cos()', 'a.log(b.max(c),)',
      // every spelling that is not `.name(` or `f64::name(`
      'std::primitive::f64::tan(a)', 'v.iter().copied().map(f64::sin).sum::<f64>()', 'let g = f64::powf;', 'f64::exp as fn(f64) -> f64',
      '<f64>::cos(a)', '<f64 as Float>::cos(a)', '< f64 > :: cos (a)', 'libm::sin(a)', 'libm :: sin(a)', 'core::f64::math::exp(a)',
      'type F = f64; let _ = F::ln(a);', 'type F = core::primitive::f64; let _ = F::ln(a);', 'use std::primitive::f64 as G; let _ = G::ln(a);',
      'f64::log(a, 10.0)', 'f64 :: hypot', 'Float::sin(a)', 'unsafe { core::intrinsics::sinf64(a) }', 'unsafe { fmaf64(a, b, c) }',
      // exact in Rust, but not what JavaScript computes
      'a.round()', 'f64::round(a)', 'v.iter().copied().map(f64::round).sum::<f64>()', 'a.round_ties_even()', 'a.to_degrees()', 'a.to_radians()',
      'f64::to_degrees(a)', '(a as f32).round()'];
    for (const call of planted) {
      writeFileSync(join(dir, 'planted.rs'), body(call));
      const hits = check([dir]);
      if (hits.length !== 1 || hits[0].line < 3) throw new Error(`self-test: planted \`${call}\` not rejected (${JSON.stringify(hits)})`);
    }
    rmSync(join(dir, 'planted.rs'));
    // An alias declared in one file and used in another.
    writeFileSync(join(dir, 'alias.rs'), 'pub type Scalar = f64;\n');
    writeFileSync(join(dir, 'user.rs'), body('crate::alias::Scalar::cos(a)'));
    if (check([dir]).length !== 1) throw new Error('self-test: cross-file alias not rejected');
    rmSync(join(dir, 'alias.rs'));
    rmSync(join(dir, 'user.rs'));
    // Only the one real path is exempt: a file that is merely called jsmath.rs is checked like any other.
    mkdirSync(join(dir, 'sub'));
    for (const file of ['jsmath.rs', 'sub/jsmath.rs']) {
      writeFileSync(join(dir, file), 'pub fn sin(x: f64) -> f64 { x.sin() }\n');
      if (check([dir]).length !== 1) throw new Error(`self-test: ${file} outside ihn-core/src was exempt`);
      if (check([dir], resolve(dir, file)).length !== 0) throw new Error(`self-test: the exempt path ${file} was checked`);
      rmSync(join(dir, file));
    }
    if (!existsSync(EXEMPT)) throw new Error(`self-test: the exempt file ${EXEMPT} does not exist`);
    for (const crate of DEFAULT_DIRS) if (!existsSync(crate)) throw new Error(`self-test: ${crate} does not exist`);
    return planted.length;
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const planted = selfTest();
  const dirs = process.argv.length > 2 ? process.argv.slice(2) : DEFAULT_DIRS;
  const hits = check(dirs);
  for (const hit of hits) {
    console.error(`${relative(process.cwd(), hit.file)}:${hit.line}: ${hit.what}: ${hit.why} (ARCHITECTURE §6.5)\n    ${hit.text}`);
  }
  if (hits.length) process.exit(1);
  console.log(`check-math: ok (${dirs.length} dirs, self-test rejected ${planted} planted spellings, no banned f64 calls outside ihn-core/src/jsmath.rs)`);
}
