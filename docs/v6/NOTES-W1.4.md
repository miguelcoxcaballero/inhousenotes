# W1.4 JSMATH — notes and deviations

## What was built

- `ihn-core/src/jsmath.rs`: `sin`, `cos`, `atan2` (+ `atan`), `exp`, `pow`, `round`, `hypot`.
- `ihn-core/src/jsfmt.rs`: `num(f64)` / `push_num(&mut String, f64)` (ES `Number::toString`) and `fixed(f64, digits)`
  (`Number.prototype.toFixed`; not in the plan, but 5.18.0 calls `toFixed` 7 times and it has the same pitfalls).
- `tests/v6/fixtures/record-math.mjs` + `tests/v6/fixtures/math/*.json` (36 872 rows, 1.6 MB).
- `scripts/check-math.mjs`: the gate, with a self-test that plants 42 banned spellings on every run.
- Test API: `core.jsmath.<sin|cos|atan|exp|round|atan2|pow|hypot>` and `core.jsfmt.num`, args = JSON array of
  16-digit hex bit patterns.

## Deviation 1 — the reference is Chromium, not Node 22

The plan says "bit-identical to Node 22" and "ports of V8's `ieee754.cc` (fdlibm) sin, cos, atan2, exp, pow". That
would not give pixel parity: 5.18.0 runs in Chromium/WebView, and Chromium 141 (V8 14.1) does **not** compute like
Node 22 (V8 12.4):

| Function | Node 22 | Chromium 141 | Rows where they differ |
|---|---|---|---|
| `sin`, `cos` | fdlibm (`ieee754.cc`) | glibc IBM Accurate Mathematical Library, bundled in V8 `third_party/glibc` (`--use-libm-trig-functions`) | ~3.5 % |
| `pow` | fdlibm `e_pow` | `v8::internal::math::pow` = special cases + C library `pow` (`--use-std-math-pow`) | ~6 % |
| `exp`, `atan2`, `round`, `hypot`, `toString`, `toFixed` | same | same | 0 |

So the corpus is recorded from Chromium (Playwright, `IHN_CHROMIUM`), and `jsmath` ports what Chromium runs. The
recorder also evaluates everything in the running Node and prints the disagreement count per function.
`ARCHITECTURE.md` §6.5 and `TESTING.md` §1 still say "Node 22"/"fdlibm"; they should be updated by the lead.

## Deviation 2 — `pow` depends on the platform's C library

Chromium's `Math.pow` calls the system `pow`. On Linux that is glibc (ARM Optimized Routines `pow`, FMA build on
x86-64 with GCC's contraction); `jsmath::pow` reproduces the machine code of glibc 2.39 `__pow_fma` (the fused
operations were read from its disassembly) and matches the headless shell used by the parity gate on every row
tried (≈ 350 000). Android (bionic, clang, arm64), Windows and macOS link other `pow` builds, which may differ from
this one in the last bit on rare inputs. No single implementation can match all of them; the parity reference
(Linux Chromium) is the one matched. 5.18.0 uses `Math.pow` in three places (luminance 9258, pull-to-refresh easing
10210, inertia decay 15875), none of which reaches a pixel at 1-ULP sensitivity in practice.

## Licence — OPEN: needs a decision by the owner before this unit is merged

`sin`/`cos` and their tables (`SINCOS`, `TOVERP`) are a port of glibc `s_sin.c`, `branred.c`, `sincostab.c`
(**LGPL-2.1-or-later**, FSF/IBM), because that is the only code that is bit-identical to Chromium. The rest is
permissive: fdlibm (Sun notice kept), ARM Optimized Routines `pow` (MIT), V8 (BSD).

State of the branch (so that it is honest about what it contains, **not** a decision):
- the LGPL code is alone in `ihn-core/src/jsmath_glibc.rs` (a private submodule of `jsmath.rs`, which re-exports
  `sin`/`cos`), with the full glibc notice;
- `COPYING.LGPL-2.1` (the licence text) and `THIRD-PARTY-NOTICES.txt` (all four notices, and where the source is)
  are at the repository root, which is the site root, so they are served at `https://inhousenotes.com/…`.

**The owner has not answered. Do not merge W1.4 into the v6 integration branch until they do.**
- If accepted: nothing else to do in this unit. Shipping LGPL code inside `ihn.wasm` also means the source of the
  module must stay available to whoever receives it (it is, while the repository is public) — the owner should be
  aware of that obligation. The Android APK loads the website, so it carries no copy of the wasm of its own unless
  the offline assets are bundled; if they are, the two files must be bundled with them.
- If refused: delete `jsmath_glibc.rs`, `COPYING.LGPL-2.1` and section 1 of the notices; put the fdlibm
  `__kernel_sin`/`__kernel_cos`/`__ieee754_rem_pio2` of V8 `ieee754.cc` behind `jsmath::{sin, cos}` (= Node 22);
  re-record `sin.json`/`cos.json` from Node instead of Chromium; record in `BEHAVIOUR-CHANGES.md` that rotation
  maths differs from Chromium by 1 ULP on ~3.5 % of arguments.

## Other notes

- `Number::toString`: Rust's shortest formatting gives the same digits as V8 except when the value is exactly
  half-way between two shortest candidates (e.g. `1368354512933715.25`): the spec and V8 pick the even digit, Rust
  the odd one. `jsfmt` corrects this with integer arithmetic (no float parsing: that would add ~10 KB of tables).
- `toFixed`: Rust rounds exact ties to even, JS away from zero; corrected the same way.
- The gate scans the `src` of all four crates and exempts exactly `rust/crates/ihn-core/src/jsmath.rs` (by path, not
  by name). It bans the inexact f64/f32 functions (`sin` … `cbrt`, `mul_add`) and the ones that are exact but not
  what JS computes: `round`, `round_ties_even` (use `jsmath::round`; Rust rounds ties away from zero, `Math.round`
  towards +infinity) and `to_degrees`/`to_radians` (write `a * 180.0 / PI` like 5.18.0). It matches method calls
  (`x.sin()`), paths whether called or not (`f64::sin`, `.map(f64::sin)`, `<f64>::cos`, `<f64 as T>::cos`, type
  aliases of f64/f32 declared anywhere in the scan), `libm::`, `f64::math::` and the `sinf64` intrinsics. It ignores
  comments and string literals, and tells `x.log(base)` from a two-argument logger call `host.log(2, "…")`.
  Test code is not exempt (use `f64::from_bits` or `jsmath` in tests). A type with its own `.round()` method would
  be reported too: name it differently.
- `tests/v6/fixtures/` is in the local `.git/info/exclude` of this checkout: new fixture files need `git add -f`.
- Wasm check (not committed): `jsmath`/`jsfmt` compiled to `wasm32-unknown-unknown` with Rust 1.90 and driven from
  Node gave 0 differences on 526 451 rows, so the software `fma` of the wasm target behaves like the native one.
- Regenerate: `IHN_CHROMIUM=<headless_shell> node tests/v6/fixtures/record-math.mjs` (`--check` to verify,
  `--out <dir> --random 15000 --seed <n>` + `IHN_MATH_CORPUS=<dir> cargo test -p ihn-core js` for a soak).
