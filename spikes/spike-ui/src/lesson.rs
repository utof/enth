//! The reference panel's lesson: about 1,500 words with headings, bold text, one figure and a
//! list (docs/benchmarks.md). Placeholder content in the shape lesson Markdown will have.

use egui::{Color32, ColorImage, TextureHandle, TextureOptions};
use enth_ui::Theme;
use enth_ui::widgets::{Block, Span};

use crate::shapes;

const fn t(text: &'static str) -> Span<'static> {
    Span {
        text,
        strong: false,
    }
}

const fn b(text: &'static str) -> Span<'static> {
    Span { text, strong: true }
}

pub const HEADER: &str = "Lesson 3 of 12 · Filters";

/// Figure size in texels (drawn at the pane's width).
pub const FIGURE_SIZE: [usize; 2] = [296, 120];

/// The lesson's blocks; `figure` is the texture of [`figure_image`].
pub fn blocks(figure: egui::TextureId) -> Vec<Block<'static>> {
    let size = egui::vec2(FIGURE_SIZE[0] as f32, FIGURE_SIZE[1] as f32);
    let mut blocks = vec![
        Block::Heading("What a filter does"),
        Block::Paragraph(
            const {
                &[
                    t(
                        "Every sound you have made so far came straight from an oscillator. A saw wave is bright and buzzy because it contains a long row of ",
                    ),
                    b("harmonics"),
                    t(", each one a sine at a whole-number multiple of the note's pitch. A "),
                    b("filter"),
                    t(
                        " changes that row: it lets some harmonics through and turns others down. That is why subtractive synthesis is called subtractive. You start with a rich wave and carve away what you don't want, the way a sculptor removes stone.",
                    ),
                ]
            },
        ),
        Block::Paragraph(
            const {
                &[
                    t("The filter in this patch is a "),
                    b("low-pass"),
                    t(
                        " filter. It passes the harmonics below its cutoff frequency and attenuates the ones above it. Turn the cutoff down and the sound gets darker, rounder and quieter; turn it up and the buzz returns. Try it now with the Cutoff knob in the Filter card while a note is held, and watch the response curve move in the card's display.",
                    ),
                ]
            },
        ),
        Block::Figure {
            texture: figure,
            size,
            caption: "Figure 1. A low-pass response: flat below the cutoff, falling above it, with a resonant peak at the corner.",
        },
        Block::Heading("Cutoff and slope"),
        Block::Paragraph(
            const {
                &[
                    t("The "),
                    b("cutoff"),
                    t(
                        " is the frequency where the filter starts to bite. It is measured in hertz, and because pitch is logarithmic the knob is too: the first quarter of its travel covers 20 Hz to about 110 Hz, the last quarter covers 3.5 kHz to 20 kHz. Each step of the knob is the same musical distance, which is why small turns near the top still sound like real changes.",
                    ),
                ]
            },
        ),
        Block::Paragraph(
            const {
                &[
                    t("Above the cutoff the level falls at a fixed rate called the "),
                    b("slope"),
                    t(
                        ". This filter is a 24 dB per octave design: one octave above the cutoff, a harmonic is 24 dB quieter than it would be without the filter, two octaves above it is 48 dB quieter. A steeper slope sounds more obviously filtered; a gentler one sounds more natural. The type menu in the card's title switches between them.",
                    ),
                ]
            },
        ),
        Block::Heading("Resonance"),
        Block::Paragraph(
            const {
                &[
                    t("Raise "),
                    b("Resonance"),
                    t(
                        " and the filter starts to emphasise the harmonics right at its cutoff. At low settings that adds a little bite and presence. Halfway up you hear a distinct vowel-like peak that follows the cutoff as you move it. Near the top the peak becomes so strong that the filter almost whistles on its own. Many classic bass and lead sounds live in that range, especially when the cutoff moves while the note plays.",
                    ),
                ]
            },
        ),
        Block::Paragraph(
            const {
                &[
                    t("Resonance is also a good way to "),
                    b("find"),
                    t(
                        " the cutoff by ear. With the resonance up, sweep the cutoff slowly and listen for the peak travelling through the harmonics. When you recreate a sound later, this is how you will match the brightness of the target: first the cutoff, then the resonance, then the movement.",
                    ),
                ]
            },
        ),
        Block::Heading("Movement: envelopes and LFOs"),
        Block::Paragraph(
            const {
                &[
                    t(
                        "A filter that stands still is only half the story. Real instruments change colour as they play: a plucked string starts bright and darkens as it rings, a brass note brightens as the player pushes more air. You get the same effect by ",
                    ),
                    b("modulating"),
                    t(
                        " the cutoff. The Filter env card is an envelope that runs every time you press a key, and its Amount knob sets how far it pushes the cutoff. A short decay with a large amount gives the classic pluck; a slow attack gives a swell.",
                    ),
                ]
            },
        ),
        Block::Paragraph(
            const {
                &[
                    t("An "),
                    b("LFO"),
                    t(
                        ", a low-frequency oscillator, moves the cutoff in a repeating cycle instead. In this patch LFO 1 is routed to the cutoff and to seven other controls. Look at the orange arcs on those knobs: each one shows where the modulation is taking the value right now. The matrix under this lesson lists every route with its amount, so you can see the whole picture at once.",
                    ),
                ]
            },
        ),
        Block::Heading("Listening exercises"),
        Block::Paragraph(
            const {
                &[t(
                    "Work through these with a single held note, then with a short melody. Keep each change small and listen before you move on; the goal is to connect what you hear with what the knobs and displays show.",
                )]
            },
        ),
        Block::Bullets(&[
            "Set Resonance to zero and sweep Cutoff from bottom to top. Notice where the sound stops being muffled.",
            "Leave Cutoff at the middle and raise Resonance step by step. Name the point where it starts to whistle.",
            "Turn Env amount up and play short notes. Compare a decay of 50 ms with one of 500 ms.",
            "Mute LFO 1 in the matrix and listen again. What movement is missing?",
            "Move Key track to the top and play low and high notes. Does the brightness stay even across the keyboard?",
            "Set Drive to half and compare loud and quiet notes. Where does the filter's character change?",
            "Recreate the first preset of this chapter by ear, then check your cutoff against the hint.",
            "Write down three sounds you know that use a moving filter, and guess the settings for each.",
        ]),
        Block::Heading("Common mistakes"),
        Block::Paragraph(
            const {
                &[t(
                    "The most common mistake is to set the cutoff so low that the oscillator's character disappears, then to make up the lost level with the volume. If a sound is dull and loud at the same time, open the filter first. The second most common is too much resonance with a fast LFO: the peak sweeps through the harmonics so quickly that it reads as a wobble rather than as tone. Slow the LFO down or lower its depth until the movement serves the note.",
                )]
            },
        ),
        Block::Paragraph(
            const {
                &[
                    t("A third, subtler one is forgetting that the "),
                    b("envelope and the knob add up"),
                    t(
                        ". When the Filter env Amount is high, the Cutoff knob sets where the sweep ends, not where it starts. If the attack sounds too bright, lower the amount before you lower the cutoff.",
                    ),
                ]
            },
        ),
        Block::Heading("Other filter types"),
        Block::Paragraph(
            const {
                &[
                    t("Low-pass is the workhorse, but the type menu offers others. A "),
                    b("high-pass"),
                    t(
                        " filter does the opposite: it removes the low end and keeps the top, which thins a sound out and makes room for a bass part. A ",
                    ),
                    b("band-pass"),
                    t(
                        " keeps only a region around the cutoff, the way a telephone or a small radio does. A ",
                    ),
                    b("notch"),
                    t(
                        " removes a narrow region and keeps everything else; swept slowly, it gives a hollow, phasing movement. Each type answers a different question about which harmonics should stay.",
                    ),
                ]
            },
        ),
        Block::Paragraph(
            const {
                &[t(
                    "A useful habit is to describe what you hear before you reach for a knob. Is the sound too dull or too bright? Is the low end muddy, or does a narrow band ring? The answer usually names the filter type and tells you which direction to turn the cutoff, which is half the work of any recreation.",
                )]
            },
        ),
        Block::Heading("Key tracking and drive"),
        Block::Paragraph(
            const {
                &[
                    b("Key track"),
                    t(
                        " makes the cutoff follow the keyboard. At zero the cutoff stays where you set it, so high notes, whose harmonics start higher, sound duller than low ones. At full tracking the cutoff rises one octave for every octave you play, and the brightness stays even across the range. Most acoustic instruments sit somewhere in between, which is why the default is half.",
                    ),
                ]
            },
        ),
        Block::Paragraph(
            const {
                &[
                    b("Drive"),
                    t(
                        " pushes the signal harder into the filter before it is shaped. A little drive thickens the sound and makes the resonance softer and rounder; a lot of drive adds distortion that the filter then tames. Because drive adds harmonics of its own, it interacts with the cutoff: a driven sound needs a lower cutoff to feel equally warm. Listen for that trade-off when you balance the two.",
                    ),
                ]
            },
        ),
        Block::Heading("Words to know"),
        Block::Bullets(&[
            "Harmonic: one of the sine components of a periodic sound, at a whole-number multiple of its pitch.",
            "Cutoff: the frequency where a filter starts to attenuate.",
            "Slope: how fast the level falls past the cutoff, in dB per octave.",
            "Resonance: emphasis of the harmonics at the cutoff.",
            "Envelope: a shape that runs once per note and moves a parameter over time.",
            "LFO: a slow oscillator that moves a parameter in a repeating cycle.",
            "Modulation depth: how far a source moves its target.",
        ]),
        Block::Heading("Before the challenge"),
        Block::Paragraph(
            const {
                &[
                    t(
                        "In the challenge that follows, you will hear a target sound and start from a patch with the filter wide open. Only the Filter card and the Filter env card are scored; everything else is dimmed but still works, so you can listen to it. Aim for the cutoff first, within a few hundred hertz, then match the resonance, then the envelope. The score ring on each knob tells you how close you are: ",
                    ),
                    b("✓"),
                    t(" for close enough, "),
                    b("~"),
                    t(" for nearly, "),
                    b("✗"),
                    t(" for not yet."),
                ]
            },
        ),
        Block::Paragraph(
            const {
                &[t(
                    "Take your time. Moving one knob at a time, listening, and moving it back is slower than turning everything at once, but it is how the connection between hearing and doing gets built, and that connection is what the rest of the course depends on.",
                )]
            },
        ),
    ];
    blocks.shrink_to_fit();
    blocks
}

/// The lesson figure: the filter card's own low-pass response, drawn into a texture in theme
/// colours (a lesson figure is a module display rendered statically).
pub fn figure_image(theme: &Theme) -> ColorImage {
    /// The response shown: cutoff at 60 % of the axis, resonance at 60 %.
    const CUTOFF: f32 = 0.6;
    const RESONANCE: f32 = 0.6;
    let [w, h] = FIGURE_SIZE;
    let c = &theme.color;
    let under = c.surface.recessed.blend(c.accent.value_wash);
    let mut pixels = vec![c.surface.recessed; w * h];
    let grid_every = w / usize::from(theme.size.display.grid_columns);
    let stroke_rows = theme.stroke.thin as usize;
    for x in 0..w {
        let level = shapes::low_pass(x as f32 / (w - 1) as f32, CUTOFF, RESONANCE);
        let curve_y = ((1.0 - level) * (h - 1) as f32) as usize;
        for y in 0..h {
            let ink: Color32 = match y {
                y if y.abs_diff(curve_y) <= stroke_rows => c.accent.value,
                y if y > curve_y => under,
                _ if x % grid_every == 0 => c.line.grid,
                _ => continue,
            };
            pixels[y * w + x] = ink;
        }
    }
    ColorImage::new(FIGURE_SIZE, pixels)
}

/// Uploads the figure once.
pub fn load_figure(ctx: &egui::Context, theme: &Theme) -> TextureHandle {
    ctx.load_texture("lesson-figure", figure_image(theme), TextureOptions::LINEAR)
}

/// Words in the lesson (to keep it near the benchmark's ~1,500).
pub fn word_count() -> usize {
    blocks(egui::TextureId::default())
        .iter()
        .map(|block| match block {
            Block::Heading(text) => text.split_whitespace().count(),
            Block::Paragraph(spans) => spans
                .iter()
                .map(|s| s.text.split_whitespace().count())
                .sum(),
            Block::Figure { caption, .. } => caption.split_whitespace().count(),
            Block::Bullets(items) => items.iter().map(|i| i.split_whitespace().count()).sum(),
        })
        .sum()
}

#[cfg(test)]
mod tests {
    #[test]
    fn lesson_is_about_1500_words() {
        let words = super::word_count();
        assert!((1200..=1800).contains(&words), "{words} words");
    }
}
