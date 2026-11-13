# W1.4 JSMATH — notes and deviations

## What was built

- `ihn-core/src/jsmath.rs`: `sin`, `cos`, `atan2` (+ `atan`), `exp`, `pow`, `round`, `hypot`.
- `ihn-core/src/jsfmt.rs`: `num(f64)` / `push_num(&mut String, f64)` (ES `Number::toString`) and `fixed(f64, digits)`
  (`Number.prototype.toFixed`; not in the plan, but 5.18.0 calls `toFixed` 7 times and it has the same pitfalls).
- `tests/v6/fixtures/record-math.mjs` + `tests/v6/fixtures/math/*.json` (38 872 rows, 1.6 MB).
- `scripts/check-math.mjs`: the gate, with a self-test that plants 16 banned calls on every run.
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

## Licence — needs a decision by the owner

`sin`/`cos` and their tables (`SINCOS`, `TOVERP`) are a port of glibc `s_sin.c`, `branred.c`, `sincostab.c`
(**LGPL-2.1-or-later**, FSF/IBM), because that is the only code that is bit-identical to Chromium. The file header
says so. The rest is permissive: fdlibm (Sun notice kept), ARM Optimized Routines `pow` (MIT), V8 (BSD). If LGPL
code in the wasm is not acceptable, the alternative is the fdlibm `sin`/`cos` (permissive, = Node 22), accepting
1-ULP differences from Chromium on ~3.5 % of arguments in rotation maths; that is a 60-line swap inside `jsmath.rs`.

## Other notes

- `Number::toString`: Rust's shortest formatting gives the same digits as V8 except when the value is exactly
  half-way between two shortest candidates (e.g. `1368354512933715.25`): the spec and V8 pick the even digit, Rust
  the odd one. `jsfmt` corrects this with integer arithmetic (no float parsing: that would add ~10 KB of tables).
- `toFixed`: Rust rounds exact ties to even, JS away from zero; corrected the same way.
- The gate also bans `mul_add` outside `jsmath.rs` and every other inexact f64 method (`asin`, `sinh`, `cbrt`, ...),
  ignores comments and string literals, and tells `x.log(base)` from a two-argument logger call `host.log(2, "…")`.
  Test code is not exempt (use `f64::from_bits` or `jsmath` in tests).
- `tests/v6/fixtures/` is in the local `.git/info/exclude` of this checkout: new fixture files need `git add -f`.
- Wasm check (not committed): `jsmath`/`jsfmt` compiled to `wasm32-unknown-unknown` with Rust 1.90 and driven from
  Node gave 0 differences on 526 451 rows, so the software `fma` of the wasm target behaves like the native one.
- Regenerate: `IHN_CHROMIUM=<headless_shell> node tests/v6/fixtures/record-math.mjs` (`--check` to verify,
  `--out <dir> --random 15000 --seed <n>` + `IHN_MATH_CORPUS=<dir> cargo test -p ihn-core js` for a soak).
