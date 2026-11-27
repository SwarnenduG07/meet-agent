//! Streaming mono resampler: a windowed-sinc low-pass when going down in rate,
//! then linear interpolation. Plenty for speech; no state beyond a few samples.

use std::f64::consts::PI;

pub struct Resampler {
    /// Input samples per output sample.
    step: f64,
    pos: f64,
    pending: Vec<f32>,
    taps: Vec<f32>,
    fir_history: Vec<f32>,
}

impl Resampler {
    #[must_use]
    pub fn new(from_hz: u32, to_hz: u32) -> Self {
        let step = f64::from(from_hz) / f64::from(to_hz);
        let taps = if to_hz < from_hz {
            low_pass(0.45 / step, 63)
        } else {
            Vec::new()
        };
        let fir_history = vec![0.0; taps.len().saturating_sub(1)];
        Self {
            step,
            pos: 0.0,
            pending: Vec::new(),
            taps,
            fir_history,
        }
    }

    /// Converts the next block of input; output length varies per call.
    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        if (self.step - 1.0).abs() < f64::EPSILON {
            return input.to_vec();
        }
        if self.taps.is_empty() {
            self.pending.extend_from_slice(input);
        } else {
            self.fir_history.extend_from_slice(input);
            let n = self.taps.len();
            for i in 0..=self.fir_history.len() - n {
                let window = &self.fir_history[i..i + n];
                self.pending
                    .push(window.iter().zip(&self.taps).map(|(x, t)| x * t).sum());
            }
            let keep = self.fir_history.len() - (n - 1);
            self.fir_history.drain(..keep);
        }

        let mut out = Vec::with_capacity((self.pending.len() as f64 / self.step) as usize + 1);
        while self.pos + 1.0 < self.pending.len() as f64 {
            let i = self.pos as usize;
            let frac = (self.pos - i as f64) as f32;
            out.push(self.pending[i] * (1.0 - frac) + self.pending[i + 1] * frac);
            self.pos += self.step;
        }
        // When going down in rate the position can step past the buffered input;
        // drop what's been read and carry the rest of the step to the next block.
        let used = (self.pos as usize).min(self.pending.len());
        self.pending.drain(..used);
        self.pos -= used as f64;
        out
    }
}

/// Hann-windowed sinc low-pass; `cutoff` is a fraction of the input sample rate.
fn low_pass(cutoff: f64, len: usize) -> Vec<f32> {
    let m = (len - 1) as f64;
    let taps: Vec<f64> = (0..len)
        .map(|n| {
            let x = n as f64 - m / 2.0;
            let sinc = if x == 0.0 {
                2.0 * cutoff
            } else {
                (2.0 * PI * cutoff * x).sin() / (PI * x)
            };
            sinc * (0.5 - 0.5 * (2.0 * PI * n as f64 / m).cos())
        })
        .collect();
    let sum: f64 = taps.iter().sum();
    taps.into_iter().map(|t| (t / sum) as f32).collect()
}
