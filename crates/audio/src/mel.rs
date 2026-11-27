//! Whisper-style log-mel features (80 bins, 25 ms window, 10 ms hop), matching
//! `transformers.WhisperFeatureExtractor`, which Smart Turn was trained with.

use std::{f64::consts::PI, sync::Arc};

use rustfft::{Fft, FftPlanner, num_complex::Complex};

const N_FFT: usize = 400;
const HOP: usize = 160;
pub const N_MELS: usize = 80;

pub struct LogMel {
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    filters: Vec<Vec<f32>>,
}

impl Default for LogMel {
    fn default() -> Self {
        Self::new()
    }
}

impl LogMel {
    #[must_use]
    pub fn new() -> Self {
        let fft = FftPlanner::new().plan_fft_forward(N_FFT);
        // Periodic Hann window, as numpy/transformers use.
        let window = (0..N_FFT)
            .map(|n| (0.5 - 0.5 * (2.0 * PI * n as f64 / N_FFT as f64).cos()) as f32)
            .collect();
        Self {
            fft,
            window,
            filters: mel_filters(16_000.0, N_FFT, N_MELS),
        }
    }

    /// Features for `audio` padded with zeros (or cut) to `frames * HOP` samples.
    /// Returns `N_MELS` rows of `frames` values, row-major.
    #[must_use]
    pub fn features(&self, audio: &[f32], frames: usize) -> Vec<f32> {
        let n = frames * HOP;
        let mut x = vec![0.0_f32; n];
        let len = audio.len().min(n);
        x[..len].copy_from_slice(&audio[audio.len() - len..]);

        // center=True with reflect padding of N_FFT/2 on both sides.
        let pad = N_FFT / 2;
        let mut padded = Vec::with_capacity(n + 2 * pad);
        padded.extend((1..=pad).rev().map(|i| x[i]));
        padded.extend_from_slice(&x);
        padded.extend((n - pad - 1..n - 1).rev().map(|i| x[i]));

        let total_frames = 1 + (padded.len() - N_FFT) / HOP; // frames + 1; the last is dropped
        let bins = N_FFT / 2 + 1;
        let mut mel = vec![0.0_f32; N_MELS * frames];
        let mut buf = vec![Complex::new(0.0_f32, 0.0); N_FFT];
        for f in 0..total_frames.min(frames) {
            let start = f * HOP;
            for (i, c) in buf.iter_mut().enumerate() {
                *c = Complex::new(padded[start + i] * self.window[i], 0.0);
            }
            self.fft.process(&mut buf);
            for (m, filt) in self.filters.iter().enumerate() {
                let e: f32 = (0..bins).map(|k| filt[k] * buf[k].norm_sqr()).sum();
                mel[m * frames + f] = e.max(1e-10).log10();
            }
        }
        let max = mel.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        for v in &mut mel {
            *v = (v.max(max - 8.0) + 4.0) / 4.0;
        }
        mel
    }
}

/// Mel filterbank like `librosa.filters.mel(sr, n_fft, n_mels)` (Slaney scale and norm).
fn mel_filters(sr: f64, n_fft: usize, n_mels: usize) -> Vec<Vec<f32>> {
    let bins = n_fft / 2 + 1;
    let fft_freqs: Vec<f64> = (0..bins).map(|k| k as f64 * sr / n_fft as f64).collect();
    let (lo, hi) = (hz_to_mel(0.0), hz_to_mel(sr / 2.0));
    let mel_f: Vec<f64> = (0..n_mels + 2)
        .map(|i| mel_to_hz(lo + (hi - lo) * i as f64 / (n_mels + 1) as f64))
        .collect();
    (0..n_mels)
        .map(|i| {
            let enorm = 2.0 / (mel_f[i + 2] - mel_f[i]);
            fft_freqs
                .iter()
                .map(|&f| {
                    let lower = (f - mel_f[i]) / (mel_f[i + 1] - mel_f[i]);
                    let upper = (mel_f[i + 2] - f) / (mel_f[i + 2] - mel_f[i + 1]);
                    (lower.min(upper).max(0.0) * enorm) as f32
                })
                .collect()
        })
        .collect()
}

const F_SP: f64 = 200.0 / 3.0;
const MIN_LOG_HZ: f64 = 1000.0;

fn hz_to_mel(f: f64) -> f64 {
    let min_log_mel = MIN_LOG_HZ / F_SP;
    if f >= MIN_LOG_HZ {
        min_log_mel + (f / MIN_LOG_HZ).ln() / (6.4_f64.ln() / 27.0)
    } else {
        f / F_SP
    }
}

fn mel_to_hz(m: f64) -> f64 {
    let min_log_mel = MIN_LOG_HZ / F_SP;
    if m >= min_log_mel {
        MIN_LOG_HZ * ((6.4_f64.ln() / 27.0) * (m - min_log_mel)).exp()
    } else {
        F_SP * m
    }
}
