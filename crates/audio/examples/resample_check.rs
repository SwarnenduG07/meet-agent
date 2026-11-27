//! Feeds audio to the resampler in small blocks (like a live device) and checks
//! the output length matches one-shot processing.

#![allow(clippy::cast_precision_loss, reason = "test signal")]

fn main() {
    for (from, to, block) in [
        (48_000, 16_000, 512),
        (44_100, 16_000, 470),
        (24_000, 48_000, 256),
        (16_000, 16_000, 100),
    ] {
        let input: Vec<f32> = (0..from * 2)
            .map(|i| ((i % 100) as f32 / 100.0) - 0.5)
            .collect();
        let whole = audio::resample::Resampler::new(from, to).process(&input);
        let mut r = audio::resample::Resampler::new(from, to);
        let blocks: usize = input.chunks(block).map(|b| r.process(b).len()).sum();
        println!(
            "{from}->{to} block {block}: one-shot {} samples, blocks {} samples",
            whole.len(),
            blocks
        );
        assert!(
            whole.len().abs_diff(blocks) <= 2,
            "block processing drifted"
        );
    }
    println!("resampler ok");
}
