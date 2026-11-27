//! Reads an input device (a mic or `BlackHole 16ch`) as 16 kHz mono chunks.

use std::{
    sync::mpsc::{self, Receiver},
    thread,
};

use anyhow::{Context, Result, anyhow, bail};
use cpal::{
    SampleFormat,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};

use crate::{CHUNK, SAMPLE_RATE, resample::Resampler};

/// A running input stream. Chunks of [`CHUNK`] samples arrive on `chunks`;
/// dropping the capture stops the device.
pub struct Capture {
    pub chunks: Receiver<Vec<f32>>,
    pub device: String,
    stop: mpsc::Sender<()>,
}

impl Drop for Capture {
    fn drop(&mut self) {
        let _ = self.stop.send(());
    }
}

/// Finds a device by exact name, or the default device when `name` is `None`.
pub(crate) fn find_device(name: Option<&str>, input: bool) -> Result<cpal::Device> {
    let host = cpal::default_host();
    match name {
        None if input => host
            .default_input_device()
            .context("no default input device"),
        None => host
            .default_output_device()
            .context("no default output device"),
        Some(want) => {
            let mut names = Vec::new();
            for d in host.devices()? {
                let Ok(desc) = d.description() else { continue };
                let has_dir = if input {
                    d.default_input_config().is_ok()
                } else {
                    d.default_output_config().is_ok()
                };
                if desc.name() == want && has_dir {
                    return Ok(d);
                }
                names.push(desc.name().to_owned());
            }
            bail!("audio device {want:?} not found; devices: {names:?}")
        }
    }
}

/// Starts reading `name` (or the default input).
///
/// # Errors
/// Fails when the device is missing or doesn't deliver 32-bit float samples.
pub fn open(name: Option<&str>) -> Result<Capture> {
    let (chunk_tx, chunks) = mpsc::channel();
    let (stop, stop_rx) = mpsc::channel::<()>();
    let (ready_tx, ready_rx) = mpsc::channel::<Result<String>>();
    let name = name.map(str::to_owned);

    // cpal streams must stay on the thread that made them.
    thread::spawn(move || {
        let setup = || -> Result<(cpal::Stream, String)> {
            let device = find_device(name.as_deref(), true)?;
            let label = device
                .description()
                .map(|d| d.name().to_owned())
                .unwrap_or_default();
            let config = device.default_input_config()?;
            if config.sample_format() != SampleFormat::F32 {
                bail!(
                    "{label} delivers {:?}, expected f32",
                    config.sample_format()
                );
            }
            let channels = usize::from(config.channels());
            let mut resampler = Resampler::new(config.sample_rate(), SAMPLE_RATE);
            let mut pending: Vec<f32> = Vec::with_capacity(CHUNK * 2);
            let stream = device.build_input_stream(
                config.into(),
                move |data: &[f32], _: &_| {
                    let mono: Vec<f32> = data
                        .chunks(channels)
                        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
                        .collect();
                    pending.extend(resampler.process(&mono));
                    while pending.len() >= CHUNK {
                        let chunk: Vec<f32> = pending.drain(..CHUNK).collect();
                        let _ = chunk_tx.send(chunk);
                    }
                },
                |e| eprintln!("[audio] input stream error: {e}"),
                None,
            )?;
            stream.play()?;
            Ok((stream, label))
        };
        match setup() {
            Ok((stream, label)) => {
                let _ = ready_tx.send(Ok(label));
                let _ = stop_rx.recv();
                drop(stream);
            }
            Err(e) => {
                let _ = ready_tx.send(Err(e));
            }
        }
    });

    let device = ready_rx
        .recv()
        .map_err(|_| anyhow!("audio thread died"))??;
    Ok(Capture {
        chunks,
        device,
        stop,
    })
}

/// A "device" that plays back recorded 16 kHz samples in real time, followed by
/// two seconds of silence so the last turn can end. For testing without a mic.
#[must_use]
pub fn from_samples(samples: Vec<f32>) -> Capture {
    let (chunk_tx, chunks) = mpsc::channel();
    let (stop, stop_rx) = mpsc::channel::<()>();
    thread::spawn(move || {
        let silence = vec![0.0; SAMPLE_RATE as usize * 2];
        for chunk in samples.chunks(CHUNK).chain(silence.chunks(CHUNK)) {
            if stop_rx.try_recv().is_ok()
                || chunk.len() < CHUNK
                || chunk_tx.send(chunk.to_vec()).is_err()
            {
                break;
            }
            thread::sleep(std::time::Duration::from_millis(32));
        }
    });
    Capture {
        chunks,
        device: "recording".into(),
        stop,
    }
}
