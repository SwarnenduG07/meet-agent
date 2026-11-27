//! Turns a stream of audio chunks into "someone started talking" and "they
//! finished" events, using Silero VAD for speech and Smart Turn for the end.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Instant,
};

use crate::{CHUNK, SAMPLE_RATE, capture::Capture, turn::SmartTurn, vad::Vad};

const CHUNK_MS: u32 = 32;
/// Pauses (ms) after which Smart Turn is asked whether the speaker is done.
const CHECKS_MS: [u32; 3] = [512, 800, 1088];
/// After this much silence the turn ends regardless.
const FORCE_END_MS: u32 = 2000;

#[derive(Debug)]
pub enum Heard {
    /// Speech began (used to interrupt the agent).
    SpeechStarted,
    /// A finished turn: 16 kHz audio and when the speaker stopped.
    Turn {
        audio: Vec<f32>,
        ended: Instant,
        pause_ms: u32,
        done_prob: f32,
    },
}

/// Listens on `capture` until the process ends. While `muted` is set, incoming
/// audio is ignored (the agent is talking on speakers and would hear itself).
pub fn spawn(
    capture: Capture,
    mut vad: Vad,
    mut turn: SmartTurn,
    muted: Arc<AtomicBool>,
    on_event: impl Fn(Heard) + Send + 'static,
) {
    thread::spawn(move || {
        let mut speech: Vec<f32> = Vec::new();
        let mut talking = false;
        let mut silent_ms = 0_u32;
        let mut last_speech = Instant::now();
        for chunk in &capture.chunks {
            if muted.load(Ordering::Relaxed) {
                if talking {
                    speech.clear();
                    talking = false;
                    vad.reset();
                }
                continue;
            }
            if chunk.len() != CHUNK {
                continue;
            }
            let p = vad.speech_prob(&chunk).unwrap_or(0.0);
            if p > 0.5 {
                if !talking {
                    talking = true;
                    on_event(Heard::SpeechStarted);
                }
                silent_ms = 0;
                last_speech = Instant::now();
            }
            if !talking {
                continue;
            }
            speech.extend_from_slice(&chunk);
            if p <= 0.5 {
                silent_ms += CHUNK_MS;
            }
            let check = CHECKS_MS.contains(&silent_ms) || silent_ms >= FORCE_END_MS;
            if !check {
                continue;
            }
            let done = turn.done_prob(&speech).unwrap_or(1.0);
            if done > 0.5 || silent_ms >= FORCE_END_MS {
                // Ignore clicks and coughs shorter than 0.3 s.
                if speech.len() > SAMPLE_RATE as usize * 3 / 10 {
                    on_event(Heard::Turn {
                        audio: std::mem::take(&mut speech),
                        ended: last_speech,
                        pause_ms: silent_ms,
                        done_prob: done,
                    });
                }
                speech.clear();
                talking = false;
                silent_ms = 0;
                vad.reset();
            }
        }
    });
}
