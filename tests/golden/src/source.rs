//! Where a case's samples come from.
//!
//! Lane S2a adds the real source here: a `Patch` variant carrying a patch file, a performance and a seed, rendered by
//! `enth_render::render`. Cases are plain text (`cases/<name>.toml`), so adding one needs no code change.

use serde::Deserialize;

/// The `[source]` table of a case file, tagged by `kind`.
#[derive(Debug, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Source {
    /// Stand-in until lane S2a: a sine through pinned `libm`, so the harness is proven end to end and the hash is
    /// the same on every platform (std's `sin` is not).
    Sine { frequency_hz: f64, amplitude: f64 },
}

impl Source {
    /// Render `frames` mono samples at `sample_rate`.
    ///
    /// # Errors
    /// A source whose parameters make no sense (negative or non-finite values).
    pub fn render(&self, sample_rate: u32, frames: usize) -> Result<Vec<f32>, String> {
        match *self {
            Self::Sine {
                frequency_hz,
                amplitude,
            } => {
                if !(frequency_hz.is_finite() && frequency_hz > 0.0 && amplitude.is_finite()) {
                    return Err(format!(
                        "sine needs a finite positive frequency_hz and a finite amplitude, got {frequency_hz} and {amplitude}"
                    ));
                }
                let w = core::f64::consts::TAU * frequency_hz / f64::from(sample_rate);
                Ok((0..frames)
                    .map(|n| (amplitude * libm::sin(w * n as f64)) as f32)
                    .collect())
            }
        }
    }
}
