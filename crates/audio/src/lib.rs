//! Audio in and out: read the call from `BlackHole 16ch` (or a mic), play the
//! agent's voice to `BlackHole 2ch` and your headphones, Silero VAD and Smart Turn.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "signal math converts between sample counts, rates and float samples on purpose"
)]

pub mod capture;
pub mod listener;
pub mod mel;
pub mod onnx;
pub mod playback;
pub mod resample;
pub mod turn;
pub mod vad;

/// Sample rate of everything the speech models see.
pub const SAMPLE_RATE: u32 = 16_000;
/// 32 ms at 16 kHz: the step Silero VAD works in.
pub const CHUNK: usize = 512;
