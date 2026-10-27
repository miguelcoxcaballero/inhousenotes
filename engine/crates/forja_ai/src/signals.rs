//! Fixed-cycle signal controllers: one per signalled junction, a green /
//! amber / all-red sequence per phase, a cycle offset for green waves and
//! pedestrian WALK / flashing / DON'T WALK heads derived from the same
//! clock. The visible lights read the same state (see
//! [`crate::network::HeadBinding`]).

/// Vehicle signal aspect.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Aspect {
    Red,
    Amber,
    Green,
}

/// Pedestrian signal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PedSignal {
    DontWalk,
    Walk,
    /// Green man flashing: do not start crossing.
    Flash,
}

/// A group of approaches that move together and the crosswalks that may
/// be used meanwhile (turning traffic gives way to them).
#[derive(Clone, Debug)]
pub struct Phase {
    pub approaches: Vec<u32>,
    pub crosswalks: Vec<u32>,
    pub green: f32,
}

#[derive(Clone, Debug)]
pub struct SignalController {
    pub node: u32,
    pub phases: Vec<Phase>,
    pub amber: f32,
    pub all_red: f32,
    pub offset: f32,
    /// Holds one phase green forever (tests, scripted scenes).
    pub forced: Option<usize>,
}

impl SignalController {
    pub fn cycle(&self) -> f32 {
        self.phases.iter().map(|p| p.green + self.amber + self.all_red).sum::<f32>().max(1.0)
    }

    /// Start of phase `p` within the cycle.
    pub fn phase_start(&self, p: usize) -> f32 {
        self.phases[..p].iter().map(|q| q.green + self.amber + self.all_red).sum()
    }

    /// Time within the cycle (offset applied).
    pub fn cycle_time(&self, t: f64) -> f32 {
        ((t + self.offset as f64).rem_euclid(self.cycle() as f64)) as f32
    }

    /// `(phase, time since its start)`.
    fn locate(&self, t: f64) -> (usize, f32) {
        let mut c = self.cycle_time(t);
        for (i, p) in self.phases.iter().enumerate() {
            let len = p.green + self.amber + self.all_red;
            if c < len {
                return (i, c);
            }
            c -= len;
        }
        (self.phases.len() - 1, 0.0)
    }

    /// Aspect shown to phase `p` and the seconds it will keep showing.
    pub fn aspect(&self, p: usize, t: f64) -> (Aspect, f32) {
        if let Some(f) = self.forced {
            return (if f == p { Aspect::Green } else { Aspect::Red }, f32::INFINITY);
        }
        let (cur, local) = self.locate(t);
        let ph = &self.phases[p];
        if cur == p {
            if local < ph.green {
                return (Aspect::Green, ph.green - local);
            }
            if local < ph.green + self.amber {
                return (Aspect::Amber, ph.green + self.amber - local);
            }
        }
        // Red until this phase's next start.
        let start = self.phase_start(p);
        let c = self.cycle_time(t);
        let mut left = start - c;
        if left <= 0.0 {
            left += self.cycle();
        }
        (Aspect::Red, left)
    }

    /// Pedestrian head of a crosswalk served by phase `p`; flashing for
    /// the last `flash` seconds of the green and during the amber.
    pub fn ped(&self, p: usize, flash: f32, t: f64) -> PedSignal {
        if let Some(f) = self.forced {
            return if f == p { PedSignal::Walk } else { PedSignal::DontWalk };
        }
        let (cur, local) = self.locate(t);
        if cur != p {
            return PedSignal::DontWalk;
        }
        let g = self.phases[p].green;
        let walk_end = (g - flash).max(g * 0.3);
        if local < walk_end {
            PedSignal::Walk
        } else if local < g + self.amber {
            PedSignal::Flash
        } else {
            PedSignal::DontWalk
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycle_runs_green_amber_red_with_offset() {
        let c = SignalController {
            node: 0,
            phases: vec![Phase { approaches: vec![0], crosswalks: vec![1], green: 20.0 }, Phase { approaches: vec![1], crosswalks: vec![0], green: 10.0 }],
            amber: 3.0,
            all_red: 2.0,
            offset: 0.0,
            forced: None,
        };
        assert_eq!(c.cycle(), 40.0);
        assert_eq!(c.aspect(0, 5.0).0, Aspect::Green);
        assert_eq!(c.aspect(1, 5.0).0, Aspect::Red);
        assert_eq!(c.aspect(0, 21.0).0, Aspect::Amber);
        assert_eq!(c.aspect(0, 24.0).0, Aspect::Red);
        // All red: nobody moves.
        assert_eq!(c.aspect(1, 24.0).0, Aspect::Red);
        assert_eq!(c.aspect(1, 26.0).0, Aspect::Green);
        assert!((c.aspect(1, 24.0).1 - 1.0).abs() < 1e-4);
        assert_eq!(c.ped(1, 5.0, 26.0), PedSignal::Walk);
        assert_eq!(c.ped(1, 5.0, 34.0), PedSignal::Flash);
        assert_eq!(c.ped(1, 5.0, 39.0), PedSignal::DontWalk);
        assert_eq!(c.ped(0, 5.0, 30.0), PedSignal::DontWalk);
        let mut o = c.clone();
        o.offset = 25.0;
        assert_eq!(o.aspect(1, 1.0).0, Aspect::Green);
    }
}
