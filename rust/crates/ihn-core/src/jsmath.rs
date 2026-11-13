//! Chromium-exact `Math.*` (ARCHITECTURE §6.5): every non-basic operation that 5.18.0 does with `Math.sin`,
//! `Math.cos`, `Math.atan2`, `Math.exp`, `Math.pow`, `Math.round` and `Math.hypot` goes through here, so geometry
//! is bit-identical to the JavaScript it replaces. `+ - * /` and `sqrt` are correctly rounded everywhere and need
//! no port. No other source file may call the std transcendental methods (`scripts/check-math.mjs`).
//!
//! What Chromium runs (V8 14.1, verified against the recorded corpus in `tests/v6/fixtures/math/`):
//! - `exp`, `atan2`: fdlibm, V8 `src/base/ieee754.cc` (Sun Microsystems licence, see below).
//! - `sin`, `cos`: the IBM Accurate Mathematical Library as bundled by V8 in `third_party/glibc`
//!   (`--use-libm-trig-functions`, the default). Node 22 still uses fdlibm here and differs by 1 ULP on ~3 % of
//!   inputs. **These two functions and their tables are a port of LGPL-2.1-or-later code** (glibc
//!   `sysdeps/ieee754/dbl-64/{s_sin.c,branred.c,sincostab.c}`, Copyright (C) 2001-2022 Free Software Foundation).
//! - `pow`: V8 `src/numbers/ieee754.cc` (`--use-std-math-pow`) = special cases + the C library `pow`, which in
//!   glibc >= 2.28 and Android bionic is ARM Optimized Routines `pow` (MIT, Copyright (c) 2018 Arm Ltd), built with
//!   FMA on current x86-64 and on arm64.
//! - `round`, `hypot`: V8 `Float64Round` and `src/builtins/math.tq` `FastMathHypot`.
//!
//! fdlibm notice: Copyright (C) 1993 by Sun Microsystems, Inc. All rights reserved. Developed at SunSoft, a Sun
//! Microsystems, Inc. business. Permission to use, copy, modify, and distribute this software is freely granted,
//! provided that this notice is preserved.

#![allow(clippy::excessive_precision, clippy::many_single_char_names, clippy::unreadable_literal)]

const fn b(bits: u64) -> f64 { f64::from_bits(bits) }
fn hi(x: f64) -> u32 { (x.to_bits() >> 32) as u32 }

// ---------------------------------------------------------------- round, hypot

/// `Math.round` (V8 `Float64Round`): ties go up, `-0.5..-0` gives `-0`.
pub fn round(x: f64) -> f64 {
    let c = x.ceil();
    if c - 0.5 > x { c - 1.0 } else { c }
}

/// `Math.hypot(a, b)` (V8 `FastMathHypot`, two arguments).
pub fn hypot(a: f64, b: f64) -> f64 {
    let (a, b) = (a.abs(), b.abs());
    if a == f64::INFINITY || b == f64::INFINITY {
        return f64::INFINITY;
    }
    if a.is_nan() || b.is_nan() {
        return f64::NAN;
    }
    let max = if a > b { a } else { b };
    if max == 0.0 {
        return 0.0;
    }
    ((a / max) * (a / max) + (b / max) * (b / max)).sqrt() * max
}

// ---------------------------------------------------------------- exp, atan, atan2 (fdlibm)

/// `Math.exp` (V8 `base::ieee754::exp`).
pub fn exp(mut x: f64) -> f64 {
    const LN2_HI: f64 = b(0x3FE62E42_FEE00000);
    const LN2_LO: f64 = b(0x3DEA39EF_35793C76);
    const INVLN2: f64 = b(0x3FF71547_652B82FE);
    const P1: f64 = b(0x3FC55555_5555553E);
    const P2: f64 = b(0xBF66C16C_16BEBD93);
    const P3: f64 = b(0x3F11566A_AF25DE2C);
    const P4: f64 = b(0xBEBBBD41_C5D26BF1);
    const P5: f64 = b(0x3E663769_72BEA4D0);
    let neg = x.is_sign_negative();
    let hx = hi(x) & 0x7FFF_FFFF;
    if hx >= 0x4086_2E42 {
        if hx >= 0x7FF0_0000 {
            return if x.is_nan() { f64::NAN } else if neg { 0.0 } else { x };
        }
        if x > b(0x40862E42_FEFA39EF) {
            return f64::INFINITY;
        }
        if x < b(0xC0874910_D52D3051) {
            return 0.0;
        }
    }
    let (mut h, mut l, mut k) = (0.0, 0.0, 0i32);
    if hx > 0x3FD6_2E42 {
        if hx < 0x3FF0_A2B2 {
            if x == 1.0 {
                return b(0x4005BF0A_8B145769); // V8 special-cases E
            }
            (h, l, k) = if neg { (x + LN2_HI, -LN2_LO, -1) } else { (x - LN2_HI, LN2_LO, 1) };
        } else {
            k = (INVLN2 * x + if neg { -0.5 } else { 0.5 }) as i32;
            let t = k as f64;
            h = x - t * LN2_HI;
            l = t * LN2_LO;
        }
        x = h - l;
    } else if hx < 0x3E30_0000 {
        return 1.0 + x;
    }
    let t = x * x;
    let c = x - t * (P1 + t * (P2 + t * (P3 + t * (P4 + t * P5))));
    if k == 0 {
        return 1.0 - ((x * c) / (c - 2.0) - x);
    }
    let y = 1.0 - ((l - (x * c) / (2.0 - c)) - h);
    let pow2 = |e: i32| b(((0x3FF + e) as u64) << 52);
    if k >= -1021 {
        if k == 1024 { y * 2.0 * pow2(1023) } else { y * pow2(k) }
    } else {
        y * pow2(k + 1000) * pow2(-1000)
    }
}

/// `Math.atan` (V8 `base::ieee754::atan`).
pub fn atan(mut x: f64) -> f64 {
    const ATANHI: [f64; 4] = [b(0x3FDDAC67_0561BB4F), b(0x3FE921FB_54442D18), b(0x3FEF730B_D281F69B), b(0x3FF921FB_54442D18)];
    const ATANLO: [f64; 4] = [b(0x3C7A2B7F_222F65E2), b(0x3C81A626_33145C07), b(0x3C700788_7AF0CBBD), b(0x3C91A626_33145C07)];
    const AT: [f64; 11] = [
        b(0x3FD55555_5555550D), b(0xBFC99999_9998EBC4), b(0x3FC24924_920083FF), b(0xBFBC71C6_FE231671),
        b(0x3FB745CD_C54C206E), b(0xBFB3B0F2_AF749A6D), b(0x3FB10D66_A0D03D51), b(0xBFADDE2D_52DEFD9A),
        b(0x3FA97B4B_24760DEB), b(0xBFA2B444_2C6A6C2F), b(0x3F90AD3A_E322DA11),
    ];
    let neg = x.is_sign_negative();
    let ix = hi(x) & 0x7FFF_FFFF;
    if ix >= 0x4410_0000 {
        if x.is_nan() {
            return f64::NAN;
        }
        let r = ATANHI[3] + ATANLO[3];
        return if neg { -r } else { r };
    }
    let id = if ix < 0x3FDC_0000 {
        if ix < 0x3E40_0000 {
            return x;
        }
        usize::MAX
    } else {
        x = x.abs();
        if ix < 0x3FF3_0000 {
            if ix < 0x3FE6_0000 {
                x = (2.0 * x - 1.0) / (2.0 + x);
                0
            } else {
                x = (x - 1.0) / (x + 1.0);
                1
            }
        } else if ix < 0x4003_8000 {
            x = (x - 1.5) / (1.0 + 1.5 * x);
            2
        } else {
            x = -1.0 / x;
            3
        }
    };
    let z = x * x;
    let w = z * z;
    let s1 = z * (AT[0] + w * (AT[2] + w * (AT[4] + w * (AT[6] + w * (AT[8] + w * AT[10])))));
    let s2 = w * (AT[1] + w * (AT[3] + w * (AT[5] + w * (AT[7] + w * AT[9]))));
    if id == usize::MAX {
        return x - x * (s1 + s2);
    }
    let z = ATANHI[id] - ((x * (s1 + s2) - ATANLO[id]) - x);
    if neg { -z } else { z }
}

/// `Math.atan2(y, x)` (V8 `base::ieee754::atan2`).
pub fn atan2(y: f64, x: f64) -> f64 {
    const PI: f64 = b(0x400921FB_54442D18);
    const PI_2: f64 = b(0x3FF921FB_54442D18);
    const PI_4: f64 = b(0x3FE921FB_54442D18);
    const PI_LO: f64 = b(0x3CA1A626_33145C07);
    if x.is_nan() || y.is_nan() {
        return f64::NAN;
    }
    if x == 1.0 {
        return atan(y);
    }
    let (xneg, yneg) = (x.is_sign_negative(), y.is_sign_negative());
    let sign = |v: f64| if yneg { -v } else { v };
    if y == 0.0 {
        return if xneg { sign(PI) } else { y };
    }
    if x == 0.0 {
        return sign(PI_2);
    }
    if x.is_infinite() {
        return sign(match (y.is_infinite(), xneg) {
            (true, false) => PI_4,
            (true, true) => 3.0 * PI_4,
            (false, false) => 0.0,
            (false, true) => PI,
        });
    }
    if y.is_infinite() {
        return sign(PI_2);
    }
    let k = ((hi(y) & 0x7FFF_FFFF) as i32 - (hi(x) & 0x7FFF_FFFF) as i32) >> 20;
    let (z, xneg) = if k > 60 {
        (PI_2 + 0.5 * PI_LO, false)
    } else if xneg && k < -60 {
        (0.0, xneg)
    } else {
        (atan((y / x).abs()), xneg)
    };
    match (xneg, yneg) {
        (false, false) => z,
        (false, true) => -z,
        (true, false) => PI - (z - PI_LO),
        (true, true) => (z - PI_LO) - PI,
    }
}

// ---------------------------------------------------------------- sin, cos (IBM Accurate Mathematical Library)

const BIG: f64 = b(0x42C8000000000000); // 1.5 * 2^45
const TOINT: f64 = b(0x4338000000000000); // 1.5 * 2^52
const SN3: f64 = b(0xbfc5555555555515);
const SN5: f64 = b(0x3f811110e829872f);
const CS2: f64 = 0.5;
const CS4: f64 = b(0xbfa5555555555535);
const CS6: f64 = b(0x3f56c16bedd9e239);

/// `(sin, sin tail, cos, cos tail)` of the table point nearest to `|x|` (`u = BIG + |x|`).
fn sincos_point(u: f64) -> [f64; 4] {
    let k = ((u.to_bits() as u32) << 2) as usize;
    [b(SINCOS[k]), b(SINCOS[k + 1]), b(SINCOS[k + 2]), b(SINCOS[k + 3])]
}

fn do_cos(x: f64, mut dx: f64) -> f64 {
    if x < 0.0 {
        dx = -dx;
    }
    let u = BIG + x.abs();
    let x = x.abs() - (u - BIG) + dx;
    let xx = x * x;
    let s = x + x * xx * (SN3 + xx * SN5);
    let c = xx * (CS2 + xx * (CS4 + xx * CS6));
    let [sn, ssn, cs, ccs] = sincos_point(u);
    let cor = (ccs - s * ssn - cs * c) - sn * s;
    cs + cor
}

fn do_sin(x: f64, mut dx: f64) -> f64 {
    let xold = x;
    if x.abs() < 0.126 {
        let xx = x * x;
        let poly = ((((S5 * xx + S4) * xx + S3) * xx + S2) * xx) + S1;
        let t = (poly * x - 0.5 * dx) * xx + dx;
        return x + t;
    }
    if x <= 0.0 {
        dx = -dx;
    }
    let u = BIG + x.abs();
    let x = x.abs() - (u - BIG);
    let xx = x * x;
    let s = x + (dx + x * xx * (SN3 + xx * SN5));
    let c = x * dx + xx * (CS2 + xx * (CS4 + xx * CS6));
    let [sn, ssn, cs, ccs] = sincos_point(u);
    let cor = (ssn + s * ccs - sn * c) + cs * s;
    (sn + cor).copysign(xold)
}

/// `x = n·π/2 + (a + da)` for `|x| < 105414350`; returns `(a, da, n mod 4)`.
fn reduce_sincos(x: f64) -> (f64, f64, u32) {
    let t = x * HPINV + TOINT;
    let xn = t - TOINT;
    let y = (x - xn * MP1) - xn * MP2;
    let n = t.to_bits() as u32 & 3;
    let t1 = xn * PP3;
    let t2 = y - t1;
    let mut db = (y - t2) - t1;
    let t1 = xn * PP4;
    let a = t2 - t1;
    db += (t2 - a) - t1;
    (a, db, n)
}

/// glibc `__branred`: the same reduction for huge arguments, with 2/π to 1800 bits.
fn branred(x: f64) -> (f64, f64, u32) {
    const SPLIT: f64 = 134217729.0;
    const BIG0: f64 = b(0x4338000000000000);
    const BIG1: f64 = b(0x4358000000000000);
    const MP2B: f64 = b(0xBE4DDE9740000000);
    const TM24: f64 = b(0x3E70000000000000);
    let x = x * b(0x1A70000000000000); // 2^-600
    let t = x * SPLIT;
    let x1 = t - (t - x);
    let x2 = x - x1;
    let part = |xp: f64| -> (f64, f64, f64) {
        let mut sum = 0.0;
        let k = ((((hi(xp) >> 20) & 2047) as i32 - 450) / 24).max(0);
        let mut gor = b(0x63F0000000000000 - (((k * 24) as u64) << 52));
        let mut r = [0.0f64; 6];
        for (i, slot) in r.iter_mut().enumerate() {
            *slot = xp * TOVERP[k as usize + i] * gor;
            gor *= TM24;
        }
        for slot in r.iter_mut().take(3) {
            let s = (*slot + BIG0) - BIG0;
            sum += s;
            *slot -= s;
        }
        let mut t = 0.0;
        for i in 0..6 {
            t += r[5 - i];
        }
        let bb = (((((r[0] - t) + r[1]) + r[2]) + r[3]) + r[4]) + r[5];
        let s = (t + BIG0) - BIG0;
        sum += s;
        t -= s;
        let bv = t + bb;
        let bb = (t - bv) + bb;
        let s = (sum + BIG1) - BIG1;
        (bv, bb, sum - s)
    };
    let (b1, bb1, sum1) = part(x1);
    let (b2, bb2, sum2) = part(x2);
    let mut sum = sum1 + sum2;
    let mut bv = b1 + b2;
    let bb = if b1.abs() > b2.abs() { (b1 - bv) + b2 } else { (b2 - bv) + b1 };
    if bv > 0.5 {
        bv -= 1.0;
        sum += 1.0;
    } else if bv < -0.5 {
        bv += 1.0;
        sum -= 1.0;
    }
    let s = bv + (bb + bb1 + bb2);
    let t = ((bv - s) + bb) + (bb1 + bb2);
    let bv = s * SPLIT;
    let t1 = bv - (bv - s);
    let t2 = s - t1;
    let bv = s * HP0;
    let bb = (((t1 * MP1 - bv) + t1 * MP2B) + t2 * MP1) + (t2 * MP2B + s * HP1 + t * HP0);
    let s = bv + bb;
    (s, (bv - s) + bb, (sum as i32 & 3) as u32)
}

fn do_sincos(a: f64, da: f64, n: u32) -> f64 {
    let r = if n & 1 != 0 { do_cos(a, da) } else { do_sin(a, da) };
    if n & 2 != 0 { -r } else { r }
}

/// `Math.sin` (V8 `libm_sin` = glibc `__sin`).
pub fn sin(x: f64) -> f64 {
    let k = hi(x) & 0x7FFF_FFFF;
    if k < 0x3E50_0000 {
        x
    } else if k < 0x3FEB_6000 {
        do_sin(x, 0.0)
    } else if k < 0x4003_68FD {
        do_cos(HP0 - x.abs(), HP1).copysign(x)
    } else if k < 0x4199_21FB {
        let (a, da, n) = reduce_sincos(x);
        do_sincos(a, da, n)
    } else if k < 0x7FF0_0000 {
        let (a, da, n) = branred(x);
        do_sincos(a, da, n)
    } else {
        f64::NAN
    }
}

/// `Math.cos` (V8 `libm_cos` = glibc `__cos`).
pub fn cos(x: f64) -> f64 {
    let k = hi(x) & 0x7FFF_FFFF;
    if k < 0x3E40_0000 {
        1.0
    } else if k < 0x3FEB_6000 {
        do_cos(x, 0.0)
    } else if k < 0x4003_68FD {
        let y = HP0 - x.abs();
        let a = y + HP1;
        do_sin(a, (y - a) + HP1)
    } else if k < 0x4199_21FB {
        let (a, da, n) = reduce_sincos(x);
        do_sincos(a, da, n + 1)
    } else if k < 0x7FF0_0000 {
        let (a, da, n) = branred(x);
        do_sincos(a, da, n + 1)
    } else {
        f64::NAN
    }
}

// ---------------------------------------------------------------- pow (V8 wrapper + ARM Optimized Routines)

/// `a * b + c` with one rounding. glibc selects its FMA build of `pow` (`__pow_fma`) on every x86-64 CPU since
/// 2013. GCC contracts the `a * b + c` expressions of that build; the fused operations below are exactly those of
/// the machine code of glibc 2.39 (read from its disassembly), which is what the corpus was recorded against.
fn fma(a: f64, b: f64, c: f64) -> f64 { a.mul_add(b, c) }

/// `log(x)` as `(y, tail)`; `ix` is the bit pattern of `x`, normalised in the subnormal range.
fn pow_log(ix: u64) -> (f64, f64) {
    let tmp = ix.wrapping_sub(0x3FE6955500000000);
    let i = 3 * ((tmp >> 45) % 128) as usize;
    let k = (tmp as i64) >> 52;
    let z = b(ix.wrapping_sub(tmp & (0xFFF << 52)));
    let kd = k as f64;
    let (invc, logc, logctail) = (b(POW_LOG[i]), b(POW_LOG[i + 1]), b(POW_LOG[i + 2]));
    let r = fma(z, invc, -1.0);
    let t1 = fma(kd, LN2HI, logc);
    let t2 = t1 + r;
    let lo1 = fma(kd, LN2LO, logctail);
    let lo2 = t1 - t2 + r;
    let ar = -0.5 * r;
    let ar2 = r * ar;
    let ar3 = r * ar2;
    let h = t2 + ar2;
    let lo3 = fma(ar, r, -ar2);
    let lo4 = t2 - h + ar2;
    let q = fma(ar2, fma(r, A6, A5), fma(r, A4, A3));
    let q = fma(ar2, q, fma(r, A2, A1));
    let lo = fma(ar3, q, lo1 + lo2 + lo3 + lo4);
    let y = h + lo;
    (y, h - y + lo)
}

/// `±exp(x + xtail)`; `sign_bias` is `SIGN_BIAS` for a negative result.
fn pow_exp(x: f64, xtail: f64, sign_bias: u64) -> f64 {
    let signed = |v: f64| if sign_bias != 0 { -v } else { v };
    let mut abstop = (x.to_bits() >> 52) as u32 & 0x7FF;
    if abstop.wrapping_sub(0x3C9) >= 0x408 - 0x3C9 {
        if abstop.wrapping_sub(0x3C9) >= 0x8000_0000 {
            return signed(1.0 + x);
        }
        if abstop >= 0x409 {
            return signed(if x < 0.0 { 0.0 } else { f64::INFINITY });
        }
        abstop = 0;
    }
    let mut kd = fma(INVLN2N, x, TOINT);
    let ki = kd.to_bits();
    kd -= TOINT;
    let r = fma(kd, NEGLN2LON, fma(kd, NEGLN2HIN, x)) + xtail;
    let idx = 2 * (ki % 128) as usize;
    let top = ki.wrapping_add(sign_bias) << 45;
    let tail = b(EXP_TAB[idx]);
    let sbits = EXP_TAB[idx + 1].wrapping_add(top);
    let r2 = r * r;
    let tmp = fma(r2 * r2, fma(r, C5, C4), fma(r2, fma(r, C3, C2), tail + r));
    if abstop != 0 {
        let scale = b(sbits);
        return fma(scale, tmp, scale);
    }
    if ki & 0x8000_0000 == 0 {
        let scale = b(sbits.wrapping_sub(1009 << 52));
        return b(0x7F00000000000000) * fma(scale, tmp, scale); // 2^1009
    }
    let sbits = sbits.wrapping_add(1022 << 52);
    let scale = b(sbits);
    let product = scale * tmp; // used twice, so this one is not fused
    let mut y = scale + product;
    if y.abs() < 1.0 {
        let one = if y < 0.0 { -1.0 } else { 1.0 };
        let lo = scale - y + product;
        let h = one + y;
        let lo = one - h + y + lo;
        y = (h + lo) - one;
        if y == 0.0 {
            y = b(sbits & 0x8000_0000_0000_0000);
        }
    }
    b(0x0010000000000000) * y // 2^-1022
}

/// 0 if `y` (bit pattern of a non-zero finite value) is not an integer, 1 if odd, 2 if even.
fn checkint(iy: u64) -> u32 {
    let e = (iy >> 52 & 0x7FF) as u32;
    if e < 0x3FF {
        return 0;
    }
    if e > 0x3FF + 52 {
        return 2;
    }
    let bit = 1u64 << (0x3FF + 52 - e);
    if iy & (bit - 1) != 0 { 0 } else if iy & bit != 0 { 1 } else { 2 }
}

/// `Math.pow` / `**` (V8 `internal::math::pow`).
pub fn pow(x: f64, y: f64) -> f64 {
    if y.is_nan() || (y.is_infinite() && x.abs() == 1.0) {
        return f64::NAN;
    }
    if y == 2.0 {
        return x * x;
    }
    if y == 0.5 {
        return if x.is_infinite() { f64::INFINITY } else { (x + 0.0).sqrt() };
    }
    const INF: u64 = 0x7FF0_0000_0000_0000;
    const ONE: u64 = 0x3FF0_0000_0000_0000;
    let zeroinfnan = |i: u64| (i << 1).wrapping_sub(1) >= 2 * INF - 1;
    let mut sign_bias = 0u64;
    let (mut ix, iy) = (x.to_bits(), y.to_bits());
    let mut topx = (ix >> 52) as u32;
    let topy = (iy >> 52) as u32;
    let ysmall_or_big = (topy & 0x7FF).wrapping_sub(0x3BE) >= 0x43E - 0x3BE;
    if topx.wrapping_sub(1) >= 0x7FF - 1 || ysmall_or_big {
        if zeroinfnan(iy) {
            if iy << 1 == 0 || ix == ONE {
                return 1.0;
            }
            if ix << 1 > 2 * INF {
                return f64::NAN;
            }
            if ix << 1 == 2 * ONE {
                return 1.0;
            }
            return if (ix << 1 < 2 * ONE) == (iy >> 63 == 0) { 0.0 } else { y * y };
        }
        if zeroinfnan(ix) {
            let mut x2 = x * x;
            if ix >> 63 != 0 && checkint(iy) == 1 {
                x2 = -x2;
            }
            return if iy >> 63 != 0 { 1.0 / x2 } else { x2 };
        }
        if ix >> 63 != 0 {
            match checkint(iy) {
                0 => return f64::NAN,
                1 => sign_bias = 0x800 << 7,
                _ => {}
            }
            ix &= !(1 << 63);
            topx &= 0x7FF;
        }
        if ysmall_or_big {
            if ix == ONE {
                return 1.0;
            }
            if topy & 0x7FF < 0x3BE {
                return if ix > ONE { 1.0 + y } else { 1.0 - y };
            }
            return if (ix > ONE) == (topy < 0x800) { f64::INFINITY } else { 0.0 };
        }
        if topx == 0 {
            ix = (x * b(0x4330000000000000)).to_bits(); // x * 2^52
            ix &= !(1 << 63);
            ix = ix.wrapping_sub(52 << 52);
        }
    }
    let (h, lo) = pow_log(ix);
    let ehi = y * h;
    let elo = fma(y, lo, fma(y, h, -ehi));
    pow_exp(ehi, elo, sign_bias)
}

// ---------------------------------------------------------------- tables

const SINCOS: [u64; 440] = [
    0x0000000000000000, 0x0000000000000000, 0x3ff0000000000000, 0x0000000000000000,
    0x3f7fffeaaaaeeeef, 0xbc1e45e2ec67b77c, 0x3fefffc000155552, 0x3c8f4a01a0196dae,
    0x3f8fffaaaaeeeed5, 0xbc02ab639a9f0777, 0x3fefff000155549f, 0x3c828a28a03a5ef3,
    0x3f97ff7001033255, 0x3bfefe2b51527336, 0x3feffdc006bff7e6, 0x3c8ae6dae86977bd,
    0x3f9ffeaaaeeee86f, 0xbc3cd406fb224ae2, 0x3feffc00155527d3, 0xbc83b54492d89b5b,
    0x3fa3feb2b12d45d5, 0x3c34ec54203d1c11, 0x3feff9c03414a7ba, 0x3c6991f4be6c59bf,
    0x3fa7fdc01032fba9, 0xbc4599bdf46e997a, 0x3feff7006bfdf99f, 0xbc78b3b560648d5f,
    0x3fabfc6d78586dac, 0x3c18e4fd03dbf236, 0x3feff3c0c8103a31, 0x3c74856dbddc0e66,
    0x3faffaaaeeed4edb, 0xbc42d16d32684b69, 0x3feff0015549f4d3, 0x3c8328387b99426f,
    0x3fb1fc343d808bef, 0xbc5f3d32e6f3be4f, 0x3fefebc222a8ef9f, 0x3c57934934f54c77,
    0x3fb3facb12d1755b, 0xbc5921915299468c, 0x3fefe7034129ef6f, 0xbc6cbf4337c96f97,
    0x3fb5f911fd10b737, 0xbc50184f02be9102, 0x3fefe1c4c3c873eb, 0xbc35a9c9057c4a02,
    0x3fb7f701032550e4, 0x3c3afc2d1800501a, 0x3fefdc06bf7e6b9b, 0x3c831902b535f8db,
    0x3fb9f4902d55d1f9, 0x3c52696d7eac1dc1, 0x3fefd5c94b43e000, 0xbc62e768cb4f92f9,
    0x3fbbf1b78568391d, 0x3c5e91841dea4cc8, 0x3fefcf0c800e99b1, 0x3c6ea3d786d186ac,
    0x3fbdee6f16c1cce6, 0xbc450f8e2fb71673, 0x3fefc7d078d1bc88, 0x3c8075d2447db685,
    0x3fbfeaaeee86ee36, 0xbc4afcb2bcc6f03b, 0x3fefc015527d5bd3, 0x3c8b68f35094efb8,
    0x3fc0f3378ddd71d1, 0x3c6d8468724f0f9e, 0x3fefb7db2bfe0695, 0x3c821dadf4f65ab1,
    0x3fc1f0d3d7afceaf, 0xbc66ef95099769a5, 0x3fefaf22263c4bd3, 0xbc552ace133a2769,
    0x3fc2ee285e4ab88f, 0xbc6e4d0f05dee058, 0x3fefa5ea641c36f2, 0x3c404da6ed17cc7c,
    0x3fc3eb312c5d66cb, 0x3c647d666b66cb91, 0x3fef9c340a7cc428, 0x3c8c5b6b063b7462,
    0x3fc4e7ea4dc5f27b, 0x3c5949db2ac072fc, 0x3fef91ff40374d01, 0xbc67d03f4d3a9e4c,
    0x3fc5e44fcfa126f3, 0xbc66f443063f89b6, 0x3fef874c2e1eecf6, 0xbc8c6514e1332b16,
    0x3fc6e05dc05a4d4c, 0xbbd32c5c8b81c940, 0x3fef7c1afeffde24, 0xbc78f55bc47540b1,
    0x3fc7dc102fbaf2b5, 0x3c45ab50e23c97c3, 0x3fef706bdf9ece1c, 0xbc8698c80c36dcb4,
    0x3fc8d7632efaa944, 0xbc620fa262cbb953, 0x3fef643efeb82acd, 0x3c76b00ac1fe28ac,
    0x3fc9d252d0cec312, 0x3c59c43d80b1137d, 0x3fef57948cff6797, 0x3c6e3a0d3e03b1d5,
    0x3fcaccdb297a0765, 0xbc59883b57d6cdeb, 0x3fef4a6cbd1e3a79, 0x3c813df0edaebb57,
    0x3fcbc6f84edc6199, 0x3c69c1a56a7b0cab, 0x3fef3cc7c3b3d16e, 0xbc621a3ad28a3494,
    0x3fccc0a6588289a3, 0xbc6868d09bc87c6b, 0x3fef2ea5d753ffed, 0x3c8cc4215f56d583,
    0x3fcdb9e15fb5a5d0, 0xbc632e20d6cc6fc2, 0x3fef20073086649f, 0x3c7b940416c1984b,
    0x3fceb2a57f8ae5a3, 0xbc60be06af572ceb, 0x3fef10ec09c5873b, 0x3c8d9072762c1283,
    0x3fcfaaeed4f31577, 0xbc615d88508e32b8, 0x3fef01549f7deea1, 0x3c8d3c1e99e5cafd,
    0x3fd0515cbf65155c, 0xbc79b8c29dfd8ec8, 0x3feef141300d2f26, 0xbc82aa1b08ded372,
    0x3fd0cd00cef36436, 0xbc79fb0a0c93e2b5, 0x3feee0b1fbc0f11c, 0xbc4bfd2380bbc3b1,
    0x3fd14861aa94ddeb, 0xbc6be881b5b615a4, 0x3feecfa744d5efa1, 0xbc556d0a4af541d0,
    0x3fd1c37d64c6b876, 0x3c746076fe0dcff5, 0x3feebe214f76efa8, 0xbc802f9f12ba543e,
    0x3fd23e52111aaf36, 0xbc74f080334eff18, 0x3feeac2061bbaf4f, 0x3c62c1d53e94658d,
    0x3fd2b8ddc43eb49f, 0x3c61553899f2d807, 0x3fee99a4c3a7cd83, 0xbc82264b1bc53ce8,
    0x3fd3331e94049f87, 0x3c7e0cb6b40c302c, 0x3fee86aebf29a9ed, 0x3c89397afdbb58a7,
    0x3fd3ad129769d3d8, 0x3c003d5504878398, 0x3fee733ea0193d40, 0xbc86428b3546ce13,
    0x3fd426b7e69ee697, 0xbc7f09c75705c59f, 0x3fee5f54b436e9d0, 0x3c87eb0fd02fc8bc,
    0x3fd4a00c9b0f3d20, 0x3c7823ba6bb08ead, 0x3fee4af14b2a449c, 0xbc868ca02e8a6833,
    0x3fd5190ecf68a77a, 0x3c7b357155eef0f3, 0x3fee3614b680d6a5, 0xbc727793aa015237,
    0x3fd591bc9fa2f597, 0x3c67c74bac3fe0cb, 0x3fee20bf49acd6c1, 0xbc5660aec7ef636c,
    0x3fd60a1429078775, 0x3c5b1fd80ba89133, 0x3fee0af15a03dbce, 0x3c5fe8e702771ae6,
    0x3fd682138a38d7f7, 0xbc7d889202444aad, 0x3fedf4ab3ebd875e, 0xbc8e2d8a7e6736c4,
    0x3fd6f9b8e33a0255, 0x3c742bc14ee9da0d, 0x3feddded50f228d6, 0xbc6e80c8d42ba2bf,
    0x3fd7710255764214, 0xbc66ead7314bb6ce, 0x3fedc6b7eb995912, 0x3c54b364776dcd35,
    0x3fd7e7ee03c86d4e, 0xbc7b63bcdabf5af2, 0x3fedaf0b6b888e83, 0x3c8a249e2b5e5cea,
    0x3fd85e7a12826949, 0x3c78a40e9b5face0, 0x3fed96e82f71a9dc, 0x3c8ff61bd5d2039d,
    0x3fd8d4a4a774992f, 0x3c744a02ea766326, 0x3fed7e4e97e17b4a, 0xbc63b770352bed94,
    0x3fd94a6be9f546c5, 0xbc769ce13e683f58, 0x3fed653f073e4040, 0xbc876236434bec37,
    0x3fd9bfce02e80510, 0x3c709e39a320b0a4, 0x3fed4bb9e1c619e0, 0x3c8f34bb77858f61,
    0x3fda34c91cc50cca, 0xbc5a310e3b50cecd, 0x3fed31bf8d8d7c06, 0x3c7e60dd3089cbdd,
    0x3fdaa95b63a09277, 0xbc66293eb13c0381, 0x3fed1750727d94f0, 0x3c80d52b1ec1a48e,
    0x3fdb1d8305321617, 0xbc7ae242cb99f519, 0x3fecfc6cfa52ad9f, 0x3c88b5b5508f2a0d,
    0x3fdb913e30dbac43, 0xbc7e38ad2f6c3ff1, 0x3fece115909a82e5, 0x3c81f139bb31109a,
    0x3fdc048b17b140a3, 0x3c619fe6757e9fa7, 0x3fecc54aa2b2972e, 0x3c64ee162ba83a98,
    0x3fdc7767ec7fd19e, 0xbc5eb14d1a3d5826, 0x3feca90c9fc67d0b, 0xbc646a81485e3462,
    0x3fdce9d2e3d4a51f, 0xbc62fc8a12dae298, 0x3fec8c5bf8ce1a84, 0x3c7ab3d1a1590123,
    0x3fdd5bca34047661, 0x3c728a44a75fc29c, 0x3fec6f39208be53b, 0xbc8741dbfbaadb42,
    0x3fddcd4c15329c9a, 0x3c70d4c6e171fd9a, 0x3fec51a48b8b175e, 0xbc61bbb43b9aa880,
    0x3fde3e56c1582a69, 0xbc50a4821099f88f, 0x3fec339eb01ddd81, 0xbc8caaf5ee82c5c0,
    0x3fdeaee8744b05f0, 0xbc5789b43c9b027d, 0x3fec1528065b7d50, 0xbc8892111312e828,
    0x3fdf1eff6bc4f97b, 0x3c717212f8a7525c, 0x3febf641081e7536, 0x3c8b7bd71628a9a1,
    0x3fdf8e99e76abc97, 0x3c59d950af2d00a3, 0x3febd6ea310294f5, 0x3c731bbcc88c109d,
    0x3fdffdb628d2f57a, 0x3c6f4a992e905b6a, 0x3febb723fe630f32, 0x3c772bd2452d0a39,
    0x3fe0362939c69955, 0xbc82d8cd78397b01, 0x3feb96eeef58840e, 0x3c545a3cc78fade0,
    0x3fe06d3686946e5b, 0x3c83f5ae4538ff1b, 0x3feb764b84b704c2, 0xbc8f5848c21b389b,
    0x3fe0a4021e9e1001, 0xbc86f643a13914f6, 0x3feb553a410c104e, 0x3c58ff7947027a16,
    0x3fe0da8b26b5672e, 0xbc8a58def0bee909, 0x3feb33bba89c8948, 0x3c8ea6a51d1f6ca9,
    0x3fe110d0c4b69c3b, 0x3c8d918998809981, 0x3feb11d04162a4c6, 0x3c71dd561efbc0c2,
    0x3fe146d21f8b7f82, 0x3c7bf9535e2739a8, 0x3feaef78930bd275, 0xbc7f836279746f94,
    0x3fe17c8e5f2eedb0, 0x3c635e57102e2488, 0x3feaccb526f69de5, 0x3c88fb6a8dd6b6cc,
    0x3fe1b204acb02fdd, 0xbc5f190c70cbb5ff, 0x3feaa98688308913, 0xbc0b83d607cd5070,
    0x3fe1e7343236574c, 0x3c722a3fa4f41d5a, 0x3fea85ed4373e02d, 0x3c69be06385ec792,
    0x3fe21c1c1b0394cf, 0x3c5e5b324b23aa31, 0x3fea61e9e72586af, 0x3c858330e2fd453f,
    0x3fe250bb93788bbb, 0x3c7ea3d02457bcce, 0x3fea3d7d0352bdcf, 0xbc868dbaeca19669,
    0x3fe28511c917a067, 0xbc801df1d9a16b70, 0x3fea18a729aee445, 0x3c395e25736c0358,
    0x3fe2b91dea88421e, 0xbc8fa371db216ab0, 0x3fe9f368ed912f85, 0xbc81d200c5791606,
    0x3fe2ecdf279a3082, 0x3c8d3557e0e7e37e, 0x3fe9cdc2e3f25e5c, 0x3c83f99112993f62,
    0x3fe32054b148bc4f, 0x3c8f6b42095a135b, 0x3fe9a7b5a36a6514, 0x3c8722cfcc9fa7a9,
    0x3fe3537db9be0367, 0x3c6b327e7af040f0, 0x3fe98141c42e1310, 0x3c8d1ff80488f08d,
    0x3fe386597456282b, 0xbc710fada93b07a8, 0x3fe95a67e00cb1fd, 0xbc80befda21f862d,
    0x3fe3b8e715a2840a, 0xbc797653a7d2f07b, 0x3fe93328926d9e92, 0xbc8bb77003600cda,
    0x3fe3eb25d36cd53a, 0xbc5be570e1570fc0, 0x3fe90b84784ddaf7, 0xbc70feb10ab93b87,
    0x3fe41d14e4ba6790, 0x3c84608fd287ecf5, 0x3fe8e37c303d9ad1, 0xbc6463a4b53d4bf8,
    0x3fe44eb381cf386b, 0xbc83ed6c1e6a5505, 0x3fe8bb105a5dc900, 0x3c8863e03e9474c1,
    0x3fe48000e431159f, 0xbc8b194a7463ed10, 0x3fe89241985d871f, 0x3c8c48d9c413ed84,
    0x3fe4b0fc46aab761, 0x3c20da05738cc59a, 0x3fe869108d77a6c6, 0x3c7338ffe2bfe9dd,
    0x3fe4e1a4e54ed51b, 0xbc8a492f89b7c76a, 0x3fe83f7dde701ca0, 0xbc4152cf609bc6e8,
    0x3fe511f9fd7b351c, 0xbc85c0e861c48831, 0x3fe8158a31916d5d, 0xbc6de8b90b8228de,
    0x3fe541facddbb724, 0x3c7232c28520d391, 0x3fe7eb362eaa1488, 0x3c5a1d65a4a5959f,
    0x3fe571a6966d59b3, 0x3c5c843b4d0fb198, 0x3fe7c0827f09e54f, 0xbc6c73d6d72aee68,
    0x3fe5a0fc98813a12, 0xbc8d82e2b7d4227b, 0x3fe7956fcd7f6543, 0xbc8ab276e9d45ae4,
    0x3fe5cffc16bf8f0d, 0x3c896cb370eb578a, 0x3fe769fec655211f, 0xbc6827d5cf8c68c5,
    0x3fe5fea4552a9e57, 0x3c80b6cef7ee20b7, 0x3fe73e30174efba1, 0xbc65d3ae3d94ad5f,
    0x3fe62cf49921ac79, 0xbc8edd9855b6241a, 0x3fe712046fa77678, 0x3c8425b0a5029c81,
    0x3fe65aec2963e755, 0x3c8126f96b71053c, 0x3fe6e57c800cf55e, 0x3c860286dedbd0a6,
    0x3fe6888a4e134b2f, 0xbc86b7d37644d5e6, 0x3fe6b898fa9efb5d, 0x3c715ac786ccf4b2,
    0x3fe6b5ce50b7821a, 0xbc65d5158f702e0f, 0x3fe68b5a92eb6253, 0xbc89a91ad985f89c,
    0x3fe6e2b77c40bde1, 0xbc70e729857fad53, 0x3fe65dc1fdeb8cba, 0xbc597c1b47337c77,
    0x3fe70f451d0a8c40, 0x3c697ede3885770d, 0x3fe62fcff20191c7, 0x3c6d9143895756ef,
    0x3fe73b7680dea578, 0xbc72248306dc12a2, 0x3fe6018526f563df, 0x3c846ca5e0e432d0,
    0x3fe7674af6f7b524, 0x3c7e9d3f94ac84a8, 0x3fe5d2e255f1f17a, 0x3c80314104c8892b,
    0x3fe792c1d0041d52, 0xbc8abf05eeb354eb, 0x3fe5a3e839824077, 0x3c8428aa2759be62,
    0x3fe7bdda5e28b3c2, 0x3c4ad1197ccd0393, 0x3fe574978d8e83f2, 0x3c8f4714af282d23,
    0x3fe7e893f5037959, 0x3c80eefbaa650c4c, 0x3fe544f10f592ca5, 0xbc8e7ae8e6c7a62f,
    0x3fe812ede9ae4ba4, 0xbc87830adf402dda, 0x3fe514f57d7bf3da, 0x3c747a108073c259,
];
const TOVERP: [f64; 75] = [
    10680707.0, 7228996.0, 1387004.0, 2578385.0, 16069853.0, 12639074.0, 9804092.0, 4427841.0,
    16666979.0, 11263675.0, 12935607.0, 2387514.0, 4345298.0, 14681673.0, 3074569.0, 13734428.0,
    16653803.0, 1880361.0, 10960616.0, 8533493.0, 3062596.0, 8710556.0, 7349940.0, 6258241.0,
    3772886.0, 3769171.0, 3798172.0, 8675211.0, 12450088.0, 3874808.0, 9961438.0, 366607.0,
    15675153.0, 9132554.0, 7151469.0, 3571407.0, 2607881.0, 12013382.0, 4155038.0, 6285869.0,
    7677882.0, 13102053.0, 15825725.0, 473591.0, 9065106.0, 15363067.0, 6271263.0, 9264392.0,
    5636912.0, 4652155.0, 7056368.0, 13614112.0, 10155062.0, 1944035.0, 9527646.0, 15080200.0,
    6658437.0, 6231200.0, 6832269.0, 16767104.0, 5075751.0, 3212806.0, 1398474.0, 7579849.0,
    6349435.0, 12618859.0, 4703257.0, 12806093.0, 14477321.0, 2786137.0, 12875403.0, 9837734.0,
    14528324.0, 13719321.0, 343717.0,
];
const POW_LOG: [u64; 384] = [
    0x3ff6a00000000000, 0xbfd62c82f2b9c800, 0x3cfab42428375680,
    0x3ff6800000000000, 0xbfd5d1bdbf580800, 0xbd1ca508d8e0f720,
    0x3ff6600000000000, 0xbfd5767717455800, 0xbd2362a4d5b6506d,
    0x3ff6400000000000, 0xbfd51aad872df800, 0xbce684e49eb067d5,
    0x3ff6200000000000, 0xbfd4be5f95777800, 0xbd041b6993293ee0,
    0x3ff6000000000000, 0xbfd4618bc21c6000, 0x3d13d82f484c84cc,
    0x3ff5e00000000000, 0xbfd404308686a800, 0x3cdc42f3ed820b3a,
    0x3ff5c00000000000, 0xbfd3a64c55694800, 0x3d20b1c686519460,
    0x3ff5a00000000000, 0xbfd347dd9a988000, 0x3d25594dd4c58092,
    0x3ff5800000000000, 0xbfd2e8e2bae12000, 0x3d267b1e99b72bd8,
    0x3ff5600000000000, 0xbfd2895a13de8800, 0x3d15ca14b6cfb03f,
    0x3ff5600000000000, 0xbfd2895a13de8800, 0x3d15ca14b6cfb03f,
    0x3ff5400000000000, 0xbfd22941fbcf7800, 0xbd165a242853da76,
    0x3ff5200000000000, 0xbfd1c898c1699800, 0xbd1fafbc68e75404,
    0x3ff5000000000000, 0xbfd1675cababa800, 0x3d1f1fc63382a8f0,
    0x3ff4e00000000000, 0xbfd1058bf9ae4800, 0xbd26a8c4fd055a66,
    0x3ff4c00000000000, 0xbfd0a324e2739000, 0xbd0c6bee7ef4030e,
    0x3ff4a00000000000, 0xbfd0402594b4d000, 0xbcf036b89ef42d7f,
    0x3ff4a00000000000, 0xbfd0402594b4d000, 0xbcf036b89ef42d7f,
    0x3ff4800000000000, 0xbfcfb9186d5e4000, 0x3d0d572aab993c87,
    0x3ff4600000000000, 0xbfcef0adcbdc6000, 0x3d2b26b79c86af24,
    0x3ff4400000000000, 0xbfce27076e2af000, 0xbd172f4f543fff10,
    0x3ff4200000000000, 0xbfcd5c216b4fc000, 0x3d21ba91bbca681b,
    0x3ff4000000000000, 0xbfcc8ff7c79aa000, 0x3d27794f689f8434,
    0x3ff4000000000000, 0xbfcc8ff7c79aa000, 0x3d27794f689f8434,
    0x3ff3e00000000000, 0xbfcbc286742d9000, 0x3d194eb0318bb78f,
    0x3ff3c00000000000, 0xbfcaf3c94e80c000, 0x3cba4e633fcd9066,
    0x3ff3a00000000000, 0xbfca23bc1fe2b000, 0xbd258c64dc46c1ea,
    0x3ff3a00000000000, 0xbfca23bc1fe2b000, 0xbd258c64dc46c1ea,
    0x3ff3800000000000, 0xbfc9525a9cf45000, 0xbd2ad1d904c1d4e3,
    0x3ff3600000000000, 0xbfc87fa06520d000, 0x3d2bbdbf7fdbfa09,
    0x3ff3400000000000, 0xbfc7ab890210e000, 0x3d2bdb9072534a58,
    0x3ff3400000000000, 0xbfc7ab890210e000, 0x3d2bdb9072534a58,
    0x3ff3200000000000, 0xbfc6d60fe719d000, 0xbd10e46aa3b2e266,
    0x3ff3000000000000, 0xbfc5ff3070a79000, 0xbd1e9e439f105039,
    0x3ff3000000000000, 0xbfc5ff3070a79000, 0xbd1e9e439f105039,
    0x3ff2e00000000000, 0xbfc526e5e3a1b000, 0xbd20de8b90075b8f,
    0x3ff2c00000000000, 0xbfc44d2b6ccb8000, 0x3d170cc16135783c,
    0x3ff2c00000000000, 0xbfc44d2b6ccb8000, 0x3d170cc16135783c,
    0x3ff2a00000000000, 0xbfc371fc201e9000, 0x3cf178864d27543a,
    0x3ff2800000000000, 0xbfc29552f81ff000, 0xbd248d301771c408,
    0x3ff2600000000000, 0xbfc1b72ad52f6000, 0xbd2e80a41811a396,
    0x3ff2600000000000, 0xbfc1b72ad52f6000, 0xbd2e80a41811a396,
    0x3ff2400000000000, 0xbfc0d77e7cd09000, 0x3d0a699688e85bf4,
    0x3ff2400000000000, 0xbfc0d77e7cd09000, 0x3d0a699688e85bf4,
    0x3ff2200000000000, 0xbfbfec9131dbe000, 0xbd2575545ca333f2,
    0x3ff2000000000000, 0xbfbe27076e2b0000, 0x3d2a342c2af0003c,
    0x3ff2000000000000, 0xbfbe27076e2b0000, 0x3d2a342c2af0003c,
    0x3ff1e00000000000, 0xbfbc5e548f5bc000, 0xbd1d0c57585fbe06,
    0x3ff1c00000000000, 0xbfba926d3a4ae000, 0x3d253935e85baac8,
    0x3ff1c00000000000, 0xbfba926d3a4ae000, 0x3d253935e85baac8,
    0x3ff1a00000000000, 0xbfb8c345d631a000, 0x3d137c294d2f5668,
    0x3ff1a00000000000, 0xbfb8c345d631a000, 0x3d137c294d2f5668,
    0x3ff1800000000000, 0xbfb6f0d28ae56000, 0xbd269737c93373da,
    0x3ff1600000000000, 0xbfb51b073f062000, 0x3d1f025b61c65e57,
    0x3ff1600000000000, 0xbfb51b073f062000, 0x3d1f025b61c65e57,
    0x3ff1400000000000, 0xbfb341d7961be000, 0x3d2c5edaccf913df,
    0x3ff1400000000000, 0xbfb341d7961be000, 0x3d2c5edaccf913df,
    0x3ff1200000000000, 0xbfb16536eea38000, 0x3d147c5e768fa309,
    0x3ff1000000000000, 0xbfaf0a30c0118000, 0x3d2d599e83368e91,
    0x3ff1000000000000, 0xbfaf0a30c0118000, 0x3d2d599e83368e91,
    0x3ff0e00000000000, 0xbfab42dd71198000, 0x3d1c827ae5d6704c,
    0x3ff0e00000000000, 0xbfab42dd71198000, 0x3d1c827ae5d6704c,
    0x3ff0c00000000000, 0xbfa77458f632c000, 0xbd2cfc4634f2a1ee,
    0x3ff0c00000000000, 0xbfa77458f632c000, 0xbd2cfc4634f2a1ee,
    0x3ff0a00000000000, 0xbfa39e87b9fec000, 0x3cf502b7f526feaa,
    0x3ff0a00000000000, 0xbfa39e87b9fec000, 0x3cf502b7f526feaa,
    0x3ff0800000000000, 0xbf9f829b0e780000, 0xbd2980267c7e09e4,
    0x3ff0800000000000, 0xbf9f829b0e780000, 0xbd2980267c7e09e4,
    0x3ff0600000000000, 0xbf97b91b07d58000, 0xbd288d5493faa639,
    0x3ff0400000000000, 0xbf8fc0a8b0fc0000, 0xbcdf1e7cf6d3a69c,
    0x3ff0400000000000, 0xbf8fc0a8b0fc0000, 0xbcdf1e7cf6d3a69c,
    0x3ff0200000000000, 0xbf7fe02a6b100000, 0xbd19e23f0dda40e4,
    0x3ff0200000000000, 0xbf7fe02a6b100000, 0xbd19e23f0dda40e4,
    0x3ff0000000000000, 0x0000000000000000, 0x0000000000000000,
    0x3ff0000000000000, 0x0000000000000000, 0x0000000000000000,
    0x3fefc00000000000, 0x3f80101575890000, 0xbd10c76b999d2be8,
    0x3fef800000000000, 0x3f90205658938000, 0xbd23dc5b06e2f7d2,
    0x3fef400000000000, 0x3f98492528c90000, 0xbd2aa0ba325a0c34,
    0x3fef000000000000, 0x3fa0415d89e74000, 0x3d0111c05cf1d753,
    0x3feec00000000000, 0x3fa466aed42e0000, 0xbd2c167375bdfd28,
    0x3fee800000000000, 0x3fa894aa149fc000, 0xbd197995d05a267d,
    0x3fee400000000000, 0x3faccb73cdddc000, 0xbd1a68f247d82807,
    0x3fee200000000000, 0x3faeea31c006c000, 0xbd0e113e4fc93b7b,
    0x3fede00000000000, 0x3fb1973bd1466000, 0xbd25325d560d9e9b,
    0x3feda00000000000, 0x3fb3bdf5a7d1e000, 0x3d2cc85ea5db4ed7,
    0x3fed600000000000, 0x3fb5e95a4d97a000, 0xbd2c69063c5d1d1e,
    0x3fed400000000000, 0x3fb700d30aeac000, 0x3cec1e8da99ded32,
    0x3fed000000000000, 0x3fb9335e5d594000, 0x3d23115c3abd47da,
    0x3fecc00000000000, 0x3fbb6ac88dad6000, 0xbd1390802bf768e5,
    0x3feca00000000000, 0x3fbc885801bc4000, 0x3d2646d1c65aacd3,
    0x3fec600000000000, 0x3fbec739830a2000, 0xbd2dc068afe645e0,
    0x3fec400000000000, 0x3fbfe89139dbe000, 0xbd2534d64fa10afd,
    0x3fec000000000000, 0x3fc1178e8227e000, 0x3d21ef78ce2d07f2,
    0x3febe00000000000, 0x3fc1aa2b7e23f000, 0x3d2ca78e44389934,
    0x3feba00000000000, 0x3fc2d1610c868000, 0x3d039d6ccb81b4a1,
    0x3feb800000000000, 0x3fc365fcb0159000, 0x3cc62fa8234b7289,
    0x3feb400000000000, 0x3fc4913d8333b000, 0x3d25837954fdb678,
    0x3feb200000000000, 0x3fc527e5e4a1b000, 0x3d2633e8e5697dc7,
    0x3feae00000000000, 0x3fc6574ebe8c1000, 0x3d19cf8b2c3c2e78,
    0x3feac00000000000, 0x3fc6f0128b757000, 0xbd25118de59c21e1,
    0x3feaa00000000000, 0x3fc7898d85445000, 0xbd1c661070914305,
    0x3fea600000000000, 0x3fc8beafeb390000, 0xbd073d54aae92cd1,
    0x3fea400000000000, 0x3fc95a5adcf70000, 0x3d07f22858a0ff6f,
    0x3fea000000000000, 0x3fca93ed3c8ae000, 0xbd28724350562169,
    0x3fe9e00000000000, 0x3fcb31d8575bd000, 0xbd0c358d4eace1aa,
    0x3fe9c00000000000, 0x3fcbd087383be000, 0xbd2d4bc4595412b6,
    0x3fe9a00000000000, 0x3fcc6ffbc6f01000, 0xbcf1ec72c5962bd2,
    0x3fe9600000000000, 0x3fcdb13db0d49000, 0xbd2aff2af715b035,
    0x3fe9400000000000, 0x3fce530effe71000, 0x3cc212276041f430,
    0x3fe9200000000000, 0x3fcef5ade4dd0000, 0xbcca211565bb8e11,
    0x3fe9000000000000, 0x3fcf991c6cb3b000, 0x3d1bcbecca0cdf30,
    0x3fe8c00000000000, 0x3fd07138604d5800, 0x3cf89cdb16ed4e91,
    0x3fe8a00000000000, 0x3fd0c42d67616000, 0x3d27188b163ceae9,
    0x3fe8800000000000, 0x3fd1178e8227e800, 0xbd2c210e63a5f01c,
    0x3fe8600000000000, 0x3fd16b5ccbacf800, 0x3d2b9acdf7a51681,
    0x3fe8400000000000, 0x3fd1bf99635a6800, 0x3d2ca6ed5147bdb7,
    0x3fe8200000000000, 0x3fd214456d0eb800, 0x3d0a87deba46baea,
    0x3fe7e00000000000, 0x3fd2bef07cdc9000, 0x3d2a9cfa4a5004f4,
    0x3fe7c00000000000, 0x3fd314f1e1d36000, 0xbd28e27ad3213cb8,
    0x3fe7a00000000000, 0x3fd36b6776be1000, 0x3d116ecdb0f177c8,
    0x3fe7800000000000, 0x3fd3c25277333000, 0x3d183b54b606bd5c,
    0x3fe7600000000000, 0x3fd419b423d5e800, 0x3d08e436ec90e09d,
    0x3fe7400000000000, 0x3fd4718dc271c800, 0xbd2f27ce0967d675,
    0x3fe7200000000000, 0x3fd4c9e09e173000, 0xbd2e20891b0ad8a4,
    0x3fe7000000000000, 0x3fd522ae0738a000, 0x3d2ebe708164c759,
    0x3fe6e00000000000, 0x3fd57bf753c8d000, 0x3d1fadedee5d40ef,
    0x3fe6c00000000000, 0x3fd5d5bddf596000, 0xbd0a0b2a08a465dc,
];
const EXP_TAB: [u64; 256] = [
    0x0000000000000000, 0x3ff0000000000000, 0x3c9b3b4f1a88bf6e, 0x3feff63da9fb3335,
    0xbc7160139cd8dc5d, 0x3fefec9a3e778061, 0xbc905e7a108766d1, 0x3fefe315e86e7f85,
    0x3c8cd2523567f613, 0x3fefd9b0d3158574, 0xbc8bce8023f98efa, 0x3fefd06b29ddf6de,
    0x3c60f74e61e6c861, 0x3fefc74518759bc8, 0x3c90a3e45b33d399, 0x3fefbe3ecac6f383,
    0x3c979aa65d837b6d, 0x3fefb5586cf9890f, 0x3c8eb51a92fdeffc, 0x3fefac922b7247f7,
    0x3c3ebe3d702f9cd1, 0x3fefa3ec32d3d1a2, 0xbc6a033489906e0b, 0x3fef9b66affed31b,
    0xbc9556522a2fbd0e, 0x3fef9301d0125b51, 0xbc5080ef8c4eea55, 0x3fef8abdc06c31cc,
    0xbc91c923b9d5f416, 0x3fef829aaea92de0, 0x3c80d3e3e95c55af, 0x3fef7a98c8a58e51,
    0xbc801b15eaa59348, 0x3fef72b83c7d517b, 0xbc8f1ff055de323d, 0x3fef6af9388c8dea,
    0x3c8b898c3f1353bf, 0x3fef635beb6fcb75, 0xbc96d99c7611eb26, 0x3fef5be084045cd4,
    0x3c9aecf73e3a2f60, 0x3fef54873168b9aa, 0xbc8fe782cb86389d, 0x3fef4d5022fcd91d,
    0x3c8a6f4144a6c38d, 0x3fef463b88628cd6, 0x3c807a05b0e4047d, 0x3fef3f49917ddc96,
    0x3c968efde3a8a894, 0x3fef387a6e756238, 0x3c875e18f274487d, 0x3fef31ce4fb2a63f,
    0x3c80472b981fe7f2, 0x3fef2b4565e27cdd, 0xbc96b87b3f71085e, 0x3fef24dfe1f56381,
    0x3c82f7e16d09ab31, 0x3fef1e9df51fdee1, 0xbc3d219b1a6fbffa, 0x3fef187fd0dad990,
    0x3c8b3782720c0ab4, 0x3fef1285a6e4030b, 0x3c6e149289cecb8f, 0x3fef0cafa93e2f56,
    0x3c834d754db0abb6, 0x3fef06fe0a31b715, 0x3c864201e2ac744c, 0x3fef0170fc4cd831,
    0x3c8fdd395dd3f84a, 0x3feefc08b26416ff, 0xbc86a3803b8e5b04, 0x3feef6c55f929ff1,
    0xbc924aedcc4b5068, 0x3feef1a7373aa9cb, 0xbc9907f81b512d8e, 0x3feeecae6d05d866,
    0xbc71d1e83e9436d2, 0x3feee7db34e59ff7, 0xbc991919b3ce1b15, 0x3feee32dc313a8e5,
    0x3c859f48a72a4c6d, 0x3feedea64c123422, 0xbc9312607a28698a, 0x3feeda4504ac801c,
    0xbc58a78f4817895b, 0x3feed60a21f72e2a, 0xbc7c2c9b67499a1b, 0x3feed1f5d950a897,
    0x3c4363ed60c2ac11, 0x3feece086061892d, 0x3c9666093b0664ef, 0x3feeca41ed1d0057,
    0x3c6ecce1daa10379, 0x3feec6a2b5c13cd0, 0x3c93ff8e3f0f1230, 0x3feec32af0d7d3de,
    0x3c7690cebb7aafb0, 0x3feebfdad5362a27, 0x3c931dbdeb54e077, 0x3feebcb299fddd0d,
    0xbc8f94340071a38e, 0x3feeb9b2769d2ca7, 0xbc87deccdc93a349, 0x3feeb6daa2cf6642,
    0xbc78dec6bd0f385f, 0x3feeb42b569d4f82, 0xbc861246ec7b5cf6, 0x3feeb1a4ca5d920f,
    0x3c93350518fdd78e, 0x3feeaf4736b527da, 0x3c7b98b72f8a9b05, 0x3feead12d497c7fd,
    0x3c9063e1e21c5409, 0x3feeab07dd485429, 0x3c34c7855019c6ea, 0x3feea9268a5946b7,
    0x3c9432e62b64c035, 0x3feea76f15ad2148, 0xbc8ce44a6199769f, 0x3feea5e1b976dc09,
    0xbc8c33c53bef4da8, 0x3feea47eb03a5585, 0xbc845378892be9ae, 0x3feea34634ccc320,
    0xbc93cedd78565858, 0x3feea23882552225, 0x3c5710aa807e1964, 0x3feea155d44ca973,
    0xbc93b3efbf5e2228, 0x3feea09e667f3bcd, 0xbc6a12ad8734b982, 0x3feea012750bdabf,
    0xbc6367efb86da9ee, 0x3fee9fb23c651a2f, 0xbc80dc3d54e08851, 0x3fee9f7df9519484,
    0xbc781f647e5a3ecf, 0x3fee9f75e8ec5f74, 0xbc86ee4ac08b7db0, 0x3fee9f9a48a58174,
    0xbc8619321e55e68a, 0x3fee9feb564267c9, 0x3c909ccb5e09d4d3, 0x3feea0694fde5d3f,
    0xbc7b32dcb94da51d, 0x3feea11473eb0187, 0x3c94ecfd5467c06b, 0x3feea1ed0130c132,
    0x3c65ebe1abd66c55, 0x3feea2f336cf4e62, 0xbc88a1c52fb3cf42, 0x3feea427543e1a12,
    0xbc9369b6f13b3734, 0x3feea589994cce13, 0xbc805e843a19ff1e, 0x3feea71a4623c7ad,
    0xbc94d450d872576e, 0x3feea8d99b4492ed, 0x3c90ad675b0e8a00, 0x3feeaac7d98a6699,
    0x3c8db72fc1f0eab4, 0x3feeace5422aa0db, 0xbc65b6609cc5e7ff, 0x3feeaf3216b5448c,
    0x3c7bf68359f35f44, 0x3feeb1ae99157736, 0xbc93091fa71e3d83, 0x3feeb45b0b91ffc6,
    0xbc5da9b88b6c1e29, 0x3feeb737b0cdc5e5, 0xbc6c23f97c90b959, 0x3feeba44cbc8520f,
    0xbc92434322f4f9aa, 0x3feebd829fde4e50, 0xbc85ca6cd7668e4b, 0x3feec0f170ca07ba,
    0x3c71affc2b91ce27, 0x3feec49182a3f090, 0x3c6dd235e10a73bb, 0x3feec86319e32323,
    0xbc87c50422622263, 0x3feecc667b5de565, 0x3c8b1c86e3e231d5, 0x3feed09bec4a2d33,
    0xbc91bbd1d3bcbb15, 0x3feed503b23e255d, 0x3c90cc319cee31d2, 0x3feed99e1330b358,
    0x3c8469846e735ab3, 0x3feede6b5579fdbf, 0xbc82dfcd978e9db4, 0x3feee36bbfd3f37a,
    0x3c8c1a7792cb3387, 0x3feee89f995ad3ad, 0xbc907b8f4ad1d9fa, 0x3feeee07298db666,
    0xbc55c3d956dcaeba, 0x3feef3a2b84f15fb, 0xbc90a40e3da6f640, 0x3feef9728de5593a,
    0xbc68d6f438ad9334, 0x3feeff76f2fb5e47, 0xbc91eee26b588a35, 0x3fef05b030a1064a,
    0x3c74ffd70a5fddcd, 0x3fef0c1e904bc1d2, 0xbc91bdfbfa9298ac, 0x3fef12c25bd71e09,
    0x3c736eae30af0cb3, 0x3fef199bdd85529c, 0x3c8ee3325c9ffd94, 0x3fef20ab5fffd07a,
    0x3c84e08fd10959ac, 0x3fef27f12e57d14b, 0x3c63cdaf384e1a67, 0x3fef2f6d9406e7b5,
    0x3c676b2c6c921968, 0x3fef3720dcef9069, 0xbc808a1883ccb5d2, 0x3fef3f0b555dc3fa,
    0xbc8fad5d3ffffa6f, 0x3fef472d4a07897c, 0xbc900dae3875a949, 0x3fef4f87080d89f2,
    0x3c74a385a63d07a7, 0x3fef5818dcfba487, 0xbc82919e2040220f, 0x3fef60e316c98398,
    0x3c8e5a50d5c192ac, 0x3fef69e603db3285, 0x3c843a59ac016b4b, 0x3fef7321f301b460,
    0xbc82d52107b43e1f, 0x3fef7c97337b9b5f, 0xbc892ab93b470dc9, 0x3fef864614f5a129,
    0x3c74b604603a88d3, 0x3fef902ee78b3ff6, 0x3c83c5ec519d7271, 0x3fef9a51fbc74c83,
    0xbc8ff7128fd391f0, 0x3fefa4afa2a490da, 0xbc8dae98e223747d, 0x3fefaf482d8e67f1,
    0x3c8ec3bc41aa2008, 0x3fefba1bee615a27, 0x3c842b94c3a9eb32, 0x3fefc52b376bba97,
    0x3c8a64a931d185ee, 0x3fefd0765b6e4540, 0xbc8e37bae43be3ed, 0x3fefdbfdad9cbe14,
    0x3c77893b4d91cd9d, 0x3fefe7c1819e90d8, 0x3c5305c14160cc89, 0x3feff3c22b8f71f1,
];
const LN2HI: f64 = f64::from_bits(0x3fe62e42fefa3800);
const LN2LO: f64 = f64::from_bits(0x3d2ef35793c76730);
const NEGLN2HIN: f64 = f64::from_bits(0xbf762e42fefa0000);
const NEGLN2LON: f64 = f64::from_bits(0xbd0cf79abc9e3b3a);
const C2: f64 = f64::from_bits(0x3fdffffffffffdbd);
const C3: f64 = f64::from_bits(0x3fc555555555543c);
const C4: f64 = f64::from_bits(0x3fa55555cf172b91);
const C5: f64 = f64::from_bits(0x3f81111167a4d017);
const S1: f64 = f64::from_bits(0xbfc5555555555555);
const S2: f64 = f64::from_bits(0x3f81111111110ece);
const S3: f64 = f64::from_bits(0xbf2a01a019db08b8);
const S4: f64 = f64::from_bits(0x3ec71de27b9a7ed9);
const S5: f64 = f64::from_bits(0xbe5addffc2fcdf59);
const HP0: f64 = f64::from_bits(0x3ff921fb54442d18);
const HP1: f64 = f64::from_bits(0x3c91a62633145c07);
const MP1: f64 = f64::from_bits(0x3ff921fb58000000);
const MP2: f64 = f64::from_bits(0xbe4dde973c000000);
const PP3: f64 = f64::from_bits(0xbc8cb3b398000000);
const PP4: f64 = f64::from_bits(0xbacd747f23e32ed7);
const HPINV: f64 = f64::from_bits(0x3fe45f306dc9c883);
const INVLN2N: f64 = f64::from_bits(0x40671547652b82fe);
const A1: f64 = f64::from_bits(0xbfe5555555555560);
const A2: f64 = f64::from_bits(0x3fe0000000000006);
const A3: f64 = f64::from_bits(0x3fe999999959554e);
const A4: f64 = f64::from_bits(0xbfe555555529a47a);
const A5: f64 = f64::from_bits(0xbff2495b9b4845e9);
const A6: f64 = f64::from_bits(0x3ff0002b8b263fc3);

// ---------------------------------------------------------------- test hooks

/// 16-digit hex bit patterns in `text`, in order.
pub(crate) fn hex_args(text: &str) -> Vec<f64> {
    text.split(|c: char| !c.is_ascii_hexdigit())
        .filter(|t| t.len() == 16)
        .filter_map(|t| u64::from_str_radix(t, 16).ok())
        .map(f64::from_bits)
        .collect()
}

/// `core.jsmath.<sin|cos|atan|exp|round|atan2|pow|hypot>`: args = JSON array of 16-digit hex bit patterns (one
/// per call, or pairs for the two-argument functions); returns the results in the same encoding.
pub fn test_cmd(name: &str, args: &str) -> Option<String> {
    let v = hex_args(args);
    let out: Vec<f64> = match name {
        "sin" | "cos" | "atan" | "exp" | "round" => {
            let f = match name { "sin" => sin, "cos" => cos, "atan" => atan, "exp" => exp, _ => round };
            v.iter().map(|&x| f(x)).collect()
        }
        "atan2" | "pow" | "hypot" => {
            let f = match name { "atan2" => atan2, "pow" => pow, _ => hypot };
            v.chunks_exact(2).map(|p| f(p[0], p[1])).collect()
        }
        _ => return None,
    };
    let cells: Vec<String> = out.iter().map(|r| format!("\"{:016x}\"", r.to_bits())).collect();
    Some(format!("[{}]", cells.join(",")))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Rows of `tests/v6/fixtures/math/<name>.json` (or of `$IHN_MATH_CORPUS/<name>.json`): one row per line,
    /// `"<field> <field> ..."`, as written by `tests/v6/fixtures/record-math.mjs`.
    pub(crate) fn corpus(name: &str) -> Vec<Vec<String>> {
        let dir = std::env::var("IHN_MATH_CORPUS")
            .unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../../../tests/v6/fixtures/math").to_string());
        let text = std::fs::read_to_string(format!("{dir}/{name}.json")).unwrap_or_else(|e| panic!("{dir}/{name}.json: {e}"));
        let rows: Vec<Vec<String>> = text
            .lines()
            .filter_map(|l| l.trim().trim_end_matches(',').strip_prefix('"')?.strip_suffix('"'))
            .map(|l| l.split(' ').map(str::to_string).collect())
            .collect();
        assert!(rows.len() >= 1000, "{name}: corpus too small ({})", rows.len());
        rows
    }

    pub(crate) fn bits(field: &str) -> f64 { f64::from_bits(u64::from_str_radix(field, 16).expect("hex")) }

    fn check(name: &str, f: &dyn Fn(&[f64]) -> f64) {
        let rows = corpus(name);
        let mut bad = Vec::new();
        for row in &rows {
            let v: Vec<f64> = row.iter().map(|s| bits(s)).collect();
            let (args, want) = v.split_at(v.len() - 1);
            let got = f(args);
            // JavaScript has one NaN; everything else must match bit for bit (including the sign of zero).
            if !(got.to_bits() == want[0].to_bits() || got.is_nan() && want[0].is_nan()) {
                bad.push(format!("{name}({args:?}) = {got:?} [{:016x}], Chromium {:?} [{}]", got.to_bits(), want[0], row[row.len() - 1]));
            }
        }
        assert!(bad.is_empty(), "{} of {} differ:\n{}", bad.len(), rows.len(), bad[..bad.len().min(12)].join("\n"));
    }

    #[test]
    fn sin_matches_chromium() { check("sin", &|a| sin(a[0])); }
    #[test]
    fn cos_matches_chromium() { check("cos", &|a| cos(a[0])); }
    #[test]
    fn exp_matches_chromium() { check("exp", &|a| exp(a[0])); }
    #[test]
    fn round_matches_chromium() { check("round", &|a| round(a[0])); }
    #[test]
    fn atan2_matches_chromium() { check("atan2", &|a| atan2(a[0], a[1])); }
    #[test]
    fn pow_matches_chromium() { check("pow", &|a| pow(a[0], a[1])); }
    #[test]
    fn hypot_matches_chromium() { check("hypot", &|a| hypot(a[0], a[1])); }

    #[test]
    fn test_cmd_round_trips_bit_patterns() {
        assert_eq!(test_cmd("round", r#"["4004000000000000","bfe0000000000000"]"#).unwrap(), r#"["4008000000000000","8000000000000000"]"#);
        assert_eq!(test_cmd("pow", r#"["4000000000000000","4024000000000000"]"#).unwrap(), r#"["4090000000000000"]"#);
        assert_eq!(test_cmd("atan", r#"["0000000000000000"]"#).unwrap(), r#"["0000000000000000"]"#);
        assert!(test_cmd("tan", "[]").is_none());
    }
}
