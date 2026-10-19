//! Where a case's samples come from.
//!
//! - `kind = "patch"`: a patch rendered by `enth_render::render` with a performance and a render seed. The patch is
//!   a `[source.patch]` table in the shape of a `*.patch.json` file (`enth_patch::file::PatchFile`, written in
//!   TOML); notes and automation are in seconds. Stereo.
//! - `kind = "sine"`: the harness's own smoke test, a `libm` sine (mono), independent of the engine.
//!
//! Cases are plain text (`cases/<name>.toml`), so adding one needs no code change:
//!
//! ```toml
//! description = "One saw note."
//! sample_rate = 48000
//! seconds = 1.0
//!
//! [source]
//! kind = "patch"
//! seed = 236412653                       # optional; enth_render::DEFAULT_SEED
//! notes = [{ key = 60, velocity = 0.8, on_s = 0.0, off_s = 0.5 }]
//! automation = [{ time_s = 0.25, addr = "osc1.level", value = 50.0 }]   # optional
//!
//! [source.patch]
//! schema = 1
//! engine = "0.1.0"
//! voice = { amp_env = "env1" }
//! generators = [{ id = "osc1", kind = "osc", params = { wave = "saw" } }]
//! modulators = [{ id = "env1", kind = "adsr", params = { attack = "5ms" } }]
//! ```

use enth_patch::file::PatchFile;
use enth_render::{Automation, Note, Performance, RenderSpec};
use serde::Deserialize;

/// The `[source]` table of a case file, tagged by `kind`.
#[derive(Debug, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Source {
    /// A patch, rendered in stereo by `enth_render::render`.
    Patch {
        /// The render seed; `enth_render::DEFAULT_SEED` when absent.
        #[serde(default)]
        seed: Option<u64>,
        /// The notes.
        notes: Vec<CaseNote>,
        /// Parameter changes.
        #[serde(default)]
        automation: Vec<CaseAutomation>,
        /// The patch, as a patch file.
        patch: Box<PatchFile>,
    },
    /// The harness's smoke test: a sine through pinned `libm` (std's `sin` is not the same on every platform).
    Sine { frequency_hz: f64, amplitude: f64 },
}

/// A note in a case: MIDI key, velocity 0–1, times in seconds.
#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CaseNote {
    pub key: u8,
    #[serde(default = "default_velocity")]
    pub velocity: f32,
    pub on_s: f64,
    pub off_s: f64,
}

fn default_velocity() -> f32 {
    0.8
}

/// An automation point in a case.
#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CaseAutomation {
    pub time_s: f64,
    pub addr: String,
    pub value: f32,
}

/// Rendered samples: `channels` interleaved.
#[derive(Debug, Clone, PartialEq)]
pub struct Rendered {
    pub channels: u16,
    pub samples: Vec<f32>,
}

impl Rendered {
    /// The mono mix (the mean of the channels), for the summary.
    #[must_use]
    pub fn mono(&self) -> Vec<f32> {
        if self.channels <= 1 {
            return self.samples.clone();
        }
        let c = usize::from(self.channels);
        self.samples
            .chunks(c)
            .map(|f| f.iter().sum::<f32>() / c as f32)
            .collect()
    }

    /// Frames per channel.
    #[must_use]
    pub fn frames(&self) -> usize {
        self.samples.len() / usize::from(self.channels.max(1))
    }
}

impl Source {
    /// Render `frames` frames at `sample_rate`.
    ///
    /// # Errors
    /// A source whose parameters make no sense, or a patch or performance the engine rejects.
    pub fn render(&self, sample_rate: u32, frames: usize) -> Result<Rendered, String> {
        match self {
            Self::Patch {
                seed,
                notes,
                automation,
                patch,
            } => {
                let patch = crate::patch::patch_from_file(patch)?;
                let automation = automation
                    .iter()
                    .map(|a| {
                        Ok(Automation {
                            time_s: a.time_s,
                            addr: a.addr.parse().map_err(|e| format!("automation: {e}"))?,
                            value: a.value,
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                let perf = Performance {
                    notes: notes
                        .iter()
                        .map(|n| Note::new(n.key, n.velocity, n.on_s, n.off_s))
                        .collect(),
                    automation,
                    duration_s: frames as f64 / f64::from(sample_rate),
                };
                let spec = RenderSpec::new(sample_rate, seed.unwrap_or(enth_render::DEFAULT_SEED));
                let out = enth_render::render(&patch, &perf, &spec)
                    .map_err(|e| format!("render: {e} ({})", e.code()))?;
                if out.stats.dropped_events > 0 {
                    return Err(format!("{} events were dropped", out.stats.dropped_events));
                }
                if out.frames() != frames {
                    return Err(format!(
                        "rendered {} frames, expected {frames}",
                        out.frames()
                    ));
                }
                if let Some(x) = out.left.iter().chain(&out.right).find(|x| !x.is_finite()) {
                    return Err(format!("the render holds a non-finite sample ({x}): a bug"));
                }
                Ok(Rendered {
                    channels: 2,
                    samples: out.interleaved(),
                })
            }
            &Self::Sine {
                frequency_hz,
                amplitude,
            } => {
                if !(frequency_hz.is_finite() && frequency_hz > 0.0 && amplitude.is_finite()) {
                    return Err(format!(
                        "sine needs a finite positive frequency_hz and a finite amplitude, got {frequency_hz} and {amplitude}"
                    ));
                }
                let w = core::f64::consts::TAU * frequency_hz / f64::from(sample_rate);
                Ok(Rendered {
                    channels: 1,
                    samples: (0..frames)
                        .map(|n| (amplitude * libm::sin(w * n as f64)) as f32)
                        .collect(),
                })
            }
        }
    }
}
