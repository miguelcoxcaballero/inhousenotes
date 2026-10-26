//! Driver models: Intelligent Driver Model (car following and stopping at
//! lines), the amber-light dilemma decision, MOBIL lane changes and pure
//! pursuit steering.

use forja_math::Vec2;

/// Per-driver parameters (varied a little per vehicle).
#[derive(Clone, Copy, Debug)]
pub struct DriverParams {
    /// Maximum acceleration (m/s²).
    pub a_max: f32,
    /// Comfortable deceleration (m/s²).
    pub b_comf: f32,
    /// Hardest braking a driver will use for a light (m/s²).
    pub b_max: f32,
    /// Time headway (s).
    pub headway: f32,
    /// Jam distance (m).
    pub s0: f32,
    pub delta: f32,
    /// MOBIL politeness and threshold.
    pub politeness: f32,
    pub a_thr: f32,
    /// Largest deceleration imposed on the new follower by a lane change.
    pub b_safe: f32,
    /// Fraction of the speed limit this driver aims for.
    pub speed_factor: f32,
}

impl Default for DriverParams {
    fn default() -> Self {
        Self { a_max: 1.6, b_comf: 2.0, b_max: 6.5, headway: 1.3, s0: 2.0, delta: 4.0, politeness: 0.3, a_thr: 0.2, b_safe: 3.0, speed_factor: 1.0 }
    }
}

/// Hard limit of any braking (emergency stop, m/s²).
pub const EMERGENCY: f32 = 9.0;

/// IDM free-road term.
pub fn idm_free(v: f32, v0: f32, p: &DriverParams) -> f32 {
    let v0 = v0.max(0.1);
    if v <= v0 {
        p.a_max * (1.0 - (v / v0).powf(p.delta))
    } else {
        // Above the desired speed (a new, lower limit): ease down.
        -p.b_comf * (1.0 - (v0 / v).powf(p.a_max * p.delta / p.b_comf))
    }
}

/// IDM acceleration behind a leader `gap` metres ahead (bumper to
/// bumper) approaching at `dv = v - v_leader`.
pub fn idm(v: f32, v0: f32, gap: f32, dv: f32, p: &DriverParams) -> f32 {
    let s_star = p.s0 + (v * p.headway + v * dv / (2.0 * (p.a_max * p.b_comf).sqrt())).max(0.0);
    let g = gap.max(0.05);
    let a = idm_free(v, v0, p).min(p.a_max) - p.a_max * (s_star / g).powi(2);
    a.max(-EMERGENCY)
}

/// IDM stopping at a line `d` metres ahead: the jam distance is replaced
/// by a small margin so the bumper ends just before the line.
pub fn stop_at(v: f32, v0: f32, d: f32, p: &DriverParams) -> f32 {
    let margin = 0.6;
    idm(v, v0, d - margin + p.s0, v, p)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AmberChoice {
    Stop,
    Go,
}

/// The amber dilemma: stop if it can be done comfortably; otherwise go
/// when the line is reached before red; otherwise brake hard if
/// physically possible, else go (too close to stop).
pub fn amber_decision(d: f32, v: f32, t_left: f32, p: &DriverParams) -> AmberChoice {
    if d <= 0.05 {
        return AmberChoice::Go;
    }
    let need = v * v / (2.0 * (d - 0.5).max(0.1));
    if need <= p.b_comf * 1.5 {
        AmberChoice::Stop
    } else if d <= v * t_left {
        AmberChoice::Go
    } else if need <= p.b_max {
        AmberChoice::Stop
    } else {
        AmberChoice::Go
    }
}

/// MOBIL lane change criterion. Accelerations before/after the change for
/// this vehicle, its new follower and its old follower.
#[allow(clippy::too_many_arguments)]
pub fn mobil(me_old: f32, me_new: f32, newf_old: f32, newf_new: f32, oldf_old: f32, oldf_new: f32, p: &DriverParams, bias: f32) -> bool {
    if newf_new < -p.b_safe || me_new < -p.b_safe {
        return false;
    }
    (me_new - me_old) + p.politeness * ((newf_new - newf_old) + (oldf_new - oldf_old)) > p.a_thr - bias
}

/// Pure pursuit steering angle for a car with its rear axle at `rear`,
/// heading `fwd`, wheelbase `wb`, towards the look-ahead `goal`.
pub fn pure_pursuit(rear: Vec2, fwd: Vec2, wb: f32, goal: Vec2) -> f32 {
    let to = goal - rear;
    let ld = to.length().max(0.5);
    let alpha = fwd.angle_to(to);
    (2.0 * wb * alpha.sin() / ld).atan().clamp(-0.6, 0.6)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Integrates a vehicle approaching a line with `stop_at`.
    fn approach(v0: f32, d0: f32) -> (f32, f32) {
        let p = DriverParams::default();
        let (mut s, mut v) = (0.0f32, v0);
        let mut min_a: f32 = 0.0;
        for _ in 0..2000 {
            let a = stop_at(v, 13.9, d0 - s, &p);
            min_a = min_a.min(a);
            v = (v + a * 0.05).max(0.0);
            s += v * 0.05;
        }
        (d0 - s, min_a)
    }

    #[test]
    fn idm_stops_just_before_a_line() {
        for (v, d) in [(13.9, 60.0), (8.0, 30.0), (0.0, 20.0), (13.9, 35.0)] {
            let (left, min_a) = approach(v, d);
            assert!(left > 0.0 && left < 1.5, "v {v} d {d}: stopped {left} m before");
            assert!(min_a > -EMERGENCY, "{min_a}");
        }
    }

    #[test]
    fn idm_follows_without_closing_in() {
        let p = DriverParams::default();
        // Leader at constant 8 m/s, follower starts faster and close.
        let (mut xl, mut x, mut v) = (30.0f32, 0.0f32, 14.0f32);
        let mut min_gap = f32::MAX;
        for _ in 0..4000 {
            let a = idm(v, 14.0, xl - x - 4.5, v - 8.0, &p);
            v = (v + a * 0.05).max(0.0);
            x += v * 0.05;
            xl += 8.0 * 0.05;
            min_gap = min_gap.min(xl - x - 4.5);
        }
        assert!(min_gap > p.s0, "{min_gap}");
        assert!((v - 8.0).abs() < 0.2);
    }

    #[test]
    fn amber_dilemma_zone() {
        let p = DriverParams::default();
        // Far away: plenty of room to stop.
        assert_eq!(amber_decision(60.0, 13.9, 3.0, &p), AmberChoice::Stop);
        // Close at speed: clears the line before red.
        assert_eq!(amber_decision(12.0, 13.9, 3.0, &p), AmberChoice::Go);
        // Mid-distance late in the amber: hard but possible stop.
        assert_eq!(amber_decision(22.0, 13.9, 0.5, &p), AmberChoice::Stop);
        // Too close to stop, too late to clear: goes (would run the red).
        assert_eq!(amber_decision(9.0, 13.9, 0.3, &p), AmberChoice::Go);
    }

    #[test]
    fn mobil_needs_safety_and_incentive() {
        let p = DriverParams::default();
        assert!(mobil(-1.0, 1.0, 0.0, -0.5, 0.0, 0.0, &p, 0.0));
        assert!(!mobil(-1.0, 1.0, 0.0, -4.0, 0.0, 0.0, &p, 0.0));
        assert!(!mobil(0.5, 0.6, 0.0, 0.0, 0.0, 0.0, &p, 0.0));
    }
}
