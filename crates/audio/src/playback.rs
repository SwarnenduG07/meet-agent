//! Plays the agent's voice on one or more outputs at once (e.g. `BlackHole 2ch`
//! into Meet plus your headphones), and stops instantly when someone interrupts.

use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex, PoisonError,
        mpsc::{self},
    },
    thread,
    time::Duration,
};

use anyhow::{Result, anyhow, bail};
use cpal::{
    SampleFormat,
    traits::{DeviceTrait, StreamTrait},
};

use crate::{capture::find_device, resample::Resampler};

struct Output {
    queue: Arc<Mutex<VecDeque<f32>>>,
    rate: u32,
    resampler: Mutex<Resampler>,
    _stop: mpsc::Sender<()>,
}

pub struct Player {
    outputs: Vec<Output>,
    source_rate: u32,
    pub devices: Vec<String>,
}

impl Player {
    /// Opens every output in `names` (`None` = the system default output).
    /// With no names, audio goes to a silent output that still plays in real
    /// time, so pacing and interruptions behave as with a real device.
    /// `source_rate` is the rate of the audio passed to [`Player::play`].
    ///
    /// # Errors
    /// Fails when a device is missing or doesn't take 32-bit floats.
    pub fn open(names: &[Option<String>], source_rate: u32) -> Result<Self> {
        let mut outputs = Vec::new();
        let mut devices = Vec::new();
        for name in names {
            let (out, label) = open_output(name.clone(), source_rate)?;
            outputs.push(out);
            devices.push(label);
        }
        if names.is_empty() {
            outputs.push(silent_output(source_rate));
            devices.push("silent (muted)".into());
        }
        Ok(Self {
            outputs,
            source_rate,
            devices,
        })
    }

    /// Queues `samples` (mono, at `source_rate`) on every output.
    pub fn play(&self, samples: &[f32]) {
        for out in &self.outputs {
            let converted = out
                .resampler
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .process(samples);
            out.queue
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .extend(converted);
        }
    }

    /// Drops everything not yet played (someone started talking).
    pub fn stop(&self) {
        for out in &self.outputs {
            out.queue
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clear();
        }
    }

    /// True while any output still has audio queued.
    #[must_use]
    pub fn is_playing(&self) -> bool {
        self.outputs.iter().any(|o| {
            !o.queue
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .is_empty()
        })
    }

    /// Seconds of audio still queued on the first output.
    #[must_use]
    pub fn queued_secs(&self) -> f32 {
        self.outputs.first().map_or(0.0, |o| {
            o.queue.lock().unwrap_or_else(PoisonError::into_inner).len() as f32 / o.rate as f32
        })
    }

    /// Blocks until everything queued has played (or `stop` was called).
    pub fn wait(&self) {
        while self.is_playing() {
            thread::sleep(Duration::from_millis(20));
        }
    }

    #[must_use]
    pub fn source_rate(&self) -> u32 {
        self.source_rate
    }
}

/// Drains its queue at `rate` samples per second without making a sound.
fn silent_output(rate: u32) -> Output {
    let queue: Arc<Mutex<VecDeque<f32>>> = Arc::default();
    let (stop, stop_rx) = mpsc::channel::<()>();
    let shared = Arc::clone(&queue);
    thread::spawn(move || {
        let tick = Duration::from_millis(20);
        let per_tick = rate as usize / 50;
        while stop_rx
            .recv_timeout(tick)
            .is_err_and(|e| e == mpsc::RecvTimeoutError::Timeout)
        {
            let mut pending = shared.lock().unwrap_or_else(PoisonError::into_inner);
            let n = per_tick.min(pending.len());
            pending.drain(..n);
        }
    });
    Output {
        queue,
        rate,
        resampler: Mutex::new(Resampler::new(rate, rate)),
        _stop: stop,
    }
}

fn open_output(name: Option<String>, source_rate: u32) -> Result<(Output, String)> {
    let queue: Arc<Mutex<VecDeque<f32>>> = Arc::default();
    let (stop, stop_rx) = mpsc::channel::<()>();
    let (ready_tx, ready_rx) = mpsc::channel::<Result<(String, u32)>>();
    let q = Arc::clone(&queue);

    // cpal streams must stay on the thread that made them.
    thread::spawn(move || {
        let setup = || -> Result<(cpal::Stream, String, u32)> {
            let device = find_device(name.as_deref(), false)?;
            let label = device
                .description()
                .map(|d| d.name().to_owned())
                .unwrap_or_default();
            let config = device.default_output_config()?;
            if config.sample_format() != SampleFormat::F32 {
                bail!("{label} takes {:?}, expected f32", config.sample_format());
            }
            let channels = usize::from(config.channels());
            let rate = config.sample_rate();
            let stream = device.build_output_stream(
                config.into(),
                move |data: &mut [f32], _: &_| {
                    let mut queue = q.lock().unwrap_or_else(PoisonError::into_inner);
                    for frame in data.chunks_mut(channels) {
                        let s = queue.pop_front().unwrap_or(0.0);
                        frame.fill(s);
                    }
                },
                |e| eprintln!("[audio] output stream error: {e}"),
                None,
            )?;
            stream.play()?;
            Ok((stream, label, rate))
        };
        match setup() {
            Ok((stream, label, rate)) => {
                let _ = ready_tx.send(Ok((label, rate)));
                let _ = stop_rx.recv();
                drop(stream);
            }
            Err(e) => {
                let _ = ready_tx.send(Err(e));
            }
        }
    });

    let (label, rate) = ready_rx
        .recv()
        .map_err(|_| anyhow!("audio thread died"))??;
    let out = Output {
        queue,
        rate,
        resampler: Mutex::new(Resampler::new(source_rate, rate)),
        _stop: stop,
    };
    Ok((out, label))
}
