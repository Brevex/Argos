pub const ALPHA: f64 = 0.01;
pub const BETA: f64 = 0.01;
pub const A: f64 = 4.59511985013459;
pub const B: f64 = -4.59511985013459;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Continue,
    H0,
    H1,
}

#[derive(Debug, Clone)]
pub struct SprtAccumulator {
    s_n: f64,
}

impl Default for SprtAccumulator {
    fn default() -> Self {
        Self::new()
    }
}

impl SprtAccumulator {
    pub fn new() -> Self {
        Self { s_n: 0.0 }
    }

    pub fn update(&mut self, likelihood_ratio: f64) {
        self.s_n += likelihood_ratio;
    }

    pub fn decision(&self) -> Decision {
        if self.s_n >= A {
            Decision::H1
        } else if self.s_n <= B {
            Decision::H0
        } else {
            Decision::Continue
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sprt_accepts_h0() {
        let mut acc = SprtAccumulator::new();
        acc.update(B - 0.1);
        assert_eq!(acc.decision(), Decision::H0);
    }

    #[test]
    fn sprt_accepts_h1() {
        let mut acc = SprtAccumulator::new();
        acc.update(A + 0.1);
        assert_eq!(acc.decision(), Decision::H1);
    }

    #[test]
    fn sprt_continues() {
        let mut acc = SprtAccumulator::new();
        acc.update((A + B) / 2.0);
        assert_eq!(acc.decision(), Decision::Continue);
    }
}
