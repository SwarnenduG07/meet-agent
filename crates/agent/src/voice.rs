//! The agent's ears and mouth: capture + turn detection, Whisper, Kokoro, playback.
//! Shared by `meet-agent listen` and the meeting orchestrator.

use std::{
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::Result;
use audio::{
    capture,
    listener::{self, Heard},
    playback::Player,
    turn::SmartTurn,
    vad::Vad,
};
use speech::{stt::Stt, tts::Tts};
use tokio::sync::mpsc;

use crate::config::{Config, expand, snapshot_dir};

pub struct Voice {
    stt: Arc<Mutex<Stt>>,
    tts: Arc<Mutex<Tts>>,
    player: Arc<Player>,
    /// Set while the agent talks on speakers, so it doesn't hear itself.
    muted: Arc<AtomicBool>,
    mute_while_speaking: bool,
}

pub struct Ears {
    pub heard: mpsc::UnboundedReceiver<Heard>,
    pub device: String,
}

/// Where the agent listens.
pub enum Input {
    /// A device by name; `None` = the system default mic.
    Device(Option<String>),
    /// Recorded 16 kHz samples, played in real time (tests).
    Samples(Vec<f32>),
}

/// Loads the speech models and opens the audio devices.
/// `outputs`: device names, `None` = the system default output.
pub async fn load(
    cfg: &Config,
    input: Input,
    outputs: Vec<Option<String>>,
    mute_while_speaking: bool,
) -> Result<(Voice, Ears)> {
    let vad = Vad::load(&cfg.model_path("vad")?)?;
    let turn = SmartTurn::load(&cfg.model_path("smart_turn")?)?;
    let whisper_path = cfg.model_path("whisper")?;
    let tts_dir = snapshot_dir(&expand(&cfg.paths.hf_cache), &cfg.models["tts"].repo)?;
    let voice_name = cfg.speech.voice.clone();
    let threads = cfg.speech.tts_threads;
    let words = cfg.speech.expect_words.clone();

    // Load Whisper (GPU) and Kokoro (CPU) side by side, each warmed up once.
    let stt = tokio::task::spawn_blocking(move || -> Result<Stt> {
        let mut stt = Stt::load(&whisper_path, &words)?;
        stt.transcribe(&vec![0.0; 16_000])?;
        Ok(stt)
    });
    let tts = tokio::task::spawn_blocking(move || -> Result<Tts> {
        let mut tts = Tts::load(&tts_dir, &voice_name, threads)?;
        tts.synthesize(
            "Hello everyone, thanks for joining. Let me walk you through the architecture.",
        )?;
        Ok(tts)
    });
    let (stt, tts) = (stt.await??, tts.await??);

    let player = Arc::new(Player::open(&outputs, speech::tts::SAMPLE_RATE)?);
    let capture = match input {
        Input::Device(name) => capture::open(name.as_deref())?,
        Input::Samples(samples) => capture::from_samples(samples),
    };
    let device = capture.device.clone();
    let muted = Arc::new(AtomicBool::new(false));
    let (tx, heard) = mpsc::unbounded_channel();
    listener::spawn(capture, vad, turn, Arc::clone(&muted), move |h| {
        let _ = tx.send(h);
    });

    let voice = Voice {
        stt: Arc::new(Mutex::new(stt)),
        tts: Arc::new(Mutex::new(tts)),
        player,
        muted,
        mute_while_speaking,
    };
    Ok((voice, Ears { heard, device }))
}

impl Voice {
    /// Text for one turn of 16 kHz audio.
    pub async fn transcribe(&self, audio: Vec<f32>) -> Result<String> {
        let stt = Arc::clone(&self.stt);
        tokio::task::spawn_blocking(move || {
            stt.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .transcribe(&audio)
        })
        .await?
    }

    /// Speaks `text`. Long sentences are split at commas: the first part plays
    /// while the rest is synthesized. Returns the time until the first audio was
    /// queued, once everything is queued.
    pub async fn say(&self, text: &str) -> Result<Duration> {
        let start = std::time::Instant::now();
        let mut first = None;
        for part in split_for_speech(text) {
            let tts = Arc::clone(&self.tts);
            let wave = tokio::task::spawn_blocking(move || {
                tts.lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .synthesize(&part)
            })
            .await??;
            if self.mute_while_speaking {
                self.muted.store(true, Ordering::Relaxed);
            }
            self.player.play(&wave);
            first.get_or_insert_with(|| start.elapsed());
        }
        if self.mute_while_speaking {
            // Listen again shortly after the last sound has played.
            let (player, muted) = (Arc::clone(&self.player), Arc::clone(&self.muted));
            tokio::spawn(async move {
                while player.is_playing() {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                tokio::time::sleep(Duration::from_millis(300)).await;
                if !player.is_playing() {
                    muted.store(false, Ordering::Relaxed);
                }
            });
        }
        Ok(first.unwrap_or_default())
    }

    /// Stops talking immediately (someone interrupted).
    pub fn stop(&self) {
        self.player.stop();
    }

    #[must_use]
    pub fn is_speaking(&self) -> bool {
        self.player.is_playing()
    }

    /// Waits until everything queued has been played.
    pub async fn finish(&self) {
        while self.player.is_playing() {
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    }

    #[must_use]
    pub fn outputs(&self) -> &[String] {
        &self.player.devices
    }
}

/// Splits a long sentence at commas into parts of at least ~40 characters, so the
/// first part can be heard sooner. Short sentences stay whole.
fn split_for_speech(text: &str) -> Vec<String> {
    const MIN: usize = 40;
    let mut parts: Vec<String> = Vec::new();
    let mut current = String::new();
    for piece in text.split_inclusive([',', ';']) {
        current.push_str(piece);
        if current.trim().len() >= MIN {
            parts.push(current.trim().to_owned());
            current.clear();
        }
    }
    if !current.trim().is_empty() {
        match parts.last_mut() {
            // Don't leave a tiny tail on its own.
            Some(last) if current.trim().len() < MIN / 2 => {
                last.push(' ');
                last.push_str(current.trim());
            }
            _ => parts.push(current.trim().to_owned()),
        }
    }
    parts
}
