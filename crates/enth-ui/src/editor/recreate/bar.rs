//! **Owner: lane RUI** (the Recreate UI); the minimal bar below is lane SKR's skeleton (F2r), which RUI replaces.
//!
//! The challenge bar: difficulty dots, Target ▶ / Mine ▶ / A/B, the timer, Hint and Submit (report 01 §2.6). No
//! live score by default; Submit shows the Good/Near/Bad rings, which then update live.
//!
//! # Lane SKR's skeleton
//!
//! A round played end to end through the header's Recreate hook, with the least UI that does it: **Start** (level 1),
//! then **Target ▶**, **Mine ▶** and a **Round** menu (**Submit**, **Hint**, **Reveal**, **End**, which restores
//! the player's own patch), and a one-line readout on the status line (code, phase, sound and structure %, the time). The first
//! Submit asks the **blind rating** (a **Rate yours** menu: "Rate 1" … "Rate 5", or "Skip") before the score shows, and the
//! score is held back until it is given (docs/session.md, "What lane SKR must wire" 7): [`Recreate::hold_scores`] keeps a `Scored` result from the session until the player answers, so the
//! session records the rating before the score (it refuses a rating after the first score).
//!
//! The seam (what RUI builds on, and what a shell does each frame, docs/editor.md "The Recreate hook"):
//!
//! 1. the shell hands the session's [`RoundView`] to the editor before the frame ([`Editor::set_round`]); a shell
//!    that never does (kittest of other areas, the dev window) shows no bar at all, so no other snapshot moves;
//! 2. the bar pushes [`Intent::Game`](crate::editor::cards::Intent::Game) intents, which the editor queues
//!    ([`Recreate::push`]) instead of applying: the editor holds only a `SessionEdit`;
//! 3. after the frame the shell applies them to its `GameEdit` ([`Editor::apply_game`], which calls
//!    [`apply_game`]), with a seed of its own for a round from settings (40 bits, so the code stays short);
//! 4. before handing worker results back (`Session::game_job_done`), the shell passes them through
//!    [`Editor::hold_scores`].
//!
//! [`Editor::set_round`]: crate::editor::Editor::set_round
//! [`Editor::apply_game`]: crate::editor::Editor::apply_game
//! [`Editor::hold_scores`]: crate::editor::Editor::hold_scores

use egui::{Button, RichText, Ui};
use enth_challenge::spec::{LATEST_GENERATOR, SpecPreset};
use enth_session::game::{
    EndChoice, GameEdit, GameError, JobId, JobResult, Listen, Phase, RoundSource, RoundView,
};

use crate::editor::cards::{Intent, Intents};
use crate::theme::glyph;

/// What the bar asks the round to do. Queued by the editor and applied by the shell to its `GameEdit` after the
/// frame ([`apply_game`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameIntent {
    /// Start a round from a level's preset (1–5) with the shell's fresh seed.
    Start {
        /// The level, 1–5 (code v1's presets).
        level: u8,
    },
    /// The A/B switch.
    Listen(Listen),
    /// Submit the current patch.
    Submit,
    /// The blind rating, 1–5, or `None` to skip it.
    Rate(Option<u8>),
    /// The next hint level (lane SN's coach; marks the round assisted).
    Hint,
    /// Give up and load the target.
    Reveal,
    /// End the round.
    End(EndChoice),
}

/// Apply queued game intents in order. A refused one changes nothing; the last refusal is returned (the bar shows
/// its hint). `seed` draws a round's seed (the shell's; 40 bits keep a preset's code short).
pub fn apply_game(
    game: &mut dyn GameEdit,
    intents: Vec<GameIntent>,
    seed: &mut dyn FnMut() -> u64,
) -> Option<GameError> {
    let mut refused = None;
    for intent in intents {
        let result = match intent {
            GameIntent::Start { level } => match SpecPreset::by_id(level) {
                Some(preset) => {
                    game.game_start(RoundSource::Settings(preset.spec(LATEST_GENERATOR, seed())))
                }
                None => Ok(()),
            },
            GameIntent::Listen(listen) => game.game_listen(listen),
            GameIntent::Submit => game.game_submit().map(drop),
            GameIntent::Rate(Some(rating)) => game.game_rate(rating),
            GameIntent::Rate(None) => Ok(()),
            GameIntent::Hint => game.game_hint().map(drop),
            GameIntent::Reveal => game.game_reveal(),
            GameIntent::End(choice) => game.game_end(choice),
        };
        if let Err(e) = result {
            refused = Some(e);
        }
    }
    refused
}

/// What the bar shows, built from the session's [`RoundView`] each frame ([`Recreate::view`]). Plain values, so the
/// header's view stays comparable.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "a view model: independent facts the bar shows"
)]
pub struct BarView {
    /// The shell plays rounds (it handed the editor a round view): without it the bar draws nothing.
    pub available: bool,
    /// A round is live.
    pub live: bool,
    /// Where the round is.
    pub phase: Phase,
    /// The target is ready (Target ▶ can play).
    pub ready: bool,
    /// The A/B switch.
    pub listen: Listen,
    /// Submit is possible now.
    pub can_submit: bool,
    /// The blind rating is being asked (the score is held back).
    pub asking: bool,
    /// The one-line readout: phase, scores, time.
    pub readout: String,
    /// The challenge code (the readout's tooltip).
    pub code: String,
}

/// m:ss of active time.
fn clock(ms: u64) -> String {
    let s = ms / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

/// A score in hundredths of a percent, with one decimal.
fn percent(hundredths: u16) -> String {
    format!("{:.1} %", f64::from(hundredths) / 100.0)
}

impl BarView {
    fn of(round: Option<&RoundView>, asking: bool, refused: Option<&str>) -> BarView {
        let Some(r) = round else {
            return BarView {
                available: true,
                readout: refused.unwrap_or_default().to_owned(),
                ..BarView::default()
            };
        };
        let time = clock(r.active_ms);
        let scores = r.last.map(|s| {
            format!(
                "Sound {} {} Structure {}",
                percent(s.score.sound),
                glyph::BULLET,
                percent(s.score.structure)
            )
        });
        let broken = matches!(r.phase, Phase::Broken | Phase::Unknown);
        let readout = if let Some(e) = refused {
            e.to_owned()
        } else if broken {
            let why = r.error.as_ref().map(GameError::hint).unwrap_or_default();
            format!("This round cannot go on: End it. {why}")
        } else if r.pending.preparing {
            "Preparing the target…".to_owned()
        } else if asking {
            "How close is yours? Rate it before the score".to_owned()
        } else if r.pending.scoring {
            "Scoring…".to_owned()
        } else {
            let lead = match r.phase {
                Phase::Playing => "Playing",
                Phase::Submitted => "Not yet",
                Phase::Passed => "Passed",
                Phase::Revealed => "Revealed",
                Phase::Preparing | Phase::Broken | Phase::Unknown => "Preparing",
            };
            let line = match &scores {
                Some(s) => format!("{lead} {} {s} {} {time}", glyph::BULLET, glyph::BULLET),
                None => format!("{lead} {} {time}", glyph::BULLET),
            };
            if r.pending.hinting {
                format!("{line} {} Thinking of a hint…", glyph::BULLET)
            } else if let Some(h) = &r.hint {
                format!("{line} {} Hint {}: {}", glyph::BULLET, h.level, h.text)
            } else {
                line
            }
        };
        let open = matches!(r.phase, Phase::Playing | Phase::Submitted | Phase::Passed);
        BarView {
            available: true,
            live: true,
            phase: r.phase,
            ready: r.ab.target_ready,
            listen: r.ab.listen,
            can_submit: open && r.ab.target_ready && !r.pending.scoring && !asking,
            asking,
            readout,
            code: r.code.clone(),
        }
    }
}

/// The editor's Recreate state: the round view the shell handed over, the game intents waiting for the shell, the
/// blind-rating question and the score it holds back. Survives the window closing (it is editor state).
#[derive(Debug, Default)]
pub struct Recreate {
    /// The shell handed over a round view at least once.
    available: bool,
    round: Option<RoundView>,
    /// The blind rating is being asked.
    asking: bool,
    queued: Vec<GameIntent>,
    /// Results held back while the rating is asked.
    held: Vec<(JobId, JobResult)>,
    /// The last refusal of an intent (its hint).
    refused: Option<String>,
}

impl Recreate {
    /// This frame's round (the shell's `GameEdit::round()`).
    pub fn set_round(&mut self, round: Option<RoundView>) {
        self.available = true;
        // The question ends with the round, once the rating is given, or once a score was recorded anyway.
        if round
            .as_ref()
            .is_none_or(|r| r.rating.is_some() || r.first.is_some())
        {
            self.asking = false;
        }
        self.round = round;
    }

    /// What the bar shows.
    pub fn view(&self) -> BarView {
        if !self.available {
            return BarView::default();
        }
        BarView::of(self.round.as_ref(), self.asking, self.refused.as_deref())
    }

    /// Queue a bar intent for the shell. The first Submit of a round without a rating starts the rating question;
    /// a rating (or Skip) ends it, and so do Start and End.
    pub fn push(&mut self, intent: GameIntent) {
        match intent {
            GameIntent::Submit => {
                if self
                    .round
                    .as_ref()
                    .is_some_and(|r| r.first.is_none() && r.rating.is_none())
                {
                    self.asking = true;
                }
            }
            GameIntent::Rate(_) | GameIntent::Start { .. } | GameIntent::End(_) => {
                self.asking = false;
            }
            GameIntent::Listen(_) | GameIntent::Hint | GameIntent::Reveal => {}
        }
        self.queued.push(intent);
    }

    /// The round's one-line readout for the status line's right end (code, phase, scores, time, or why an intent was
    /// refused): `None` while no round is live and nothing was refused.
    pub fn readout(&self) -> Option<String> {
        let v = self.view();
        if !v.available || (!v.live && v.readout.is_empty()) {
            return None;
        }
        Some(if v.code.is_empty() {
            v.readout
        } else {
            format!("Recreate {} {} {}", v.code, glyph::BULLET, v.readout)
        })
    }

    /// Whether the blind rating is being asked.
    pub fn asking(&self) -> bool {
        self.asking
    }

    /// The queued intents (tests).
    pub fn queued(&self) -> &[GameIntent] {
        &self.queued
    }

    /// Apply the queued intents to `game` ([`apply_game`]) and keep the last refusal's hint for the readout.
    pub fn apply(&mut self, game: &mut dyn GameEdit, seed: &mut dyn FnMut() -> u64) {
        let queued = std::mem::take(&mut self.queued);
        if queued.is_empty() {
            return;
        }
        self.refused = apply_game(game, queued, seed).map(|e| e.hint());
    }

    /// The worker results to hand back now: everything, except that a `Scored` result waits while the blind rating
    /// is asked (and goes back, with whatever else waited, once it is given or skipped).
    pub fn hold_scores(&mut self, results: Vec<(JobId, JobResult)>) -> Vec<(JobId, JobResult)> {
        self.held.extend(results);
        if self.asking {
            let (scored, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.held)
                .into_iter()
                .partition(|(_, r)| matches!(r, JobResult::Scored(_)));
            self.held = scored;
            rest
        } else {
            std::mem::take(&mut self.held)
        }
    }
}

/// The header's Recreate hook point (feature `recreate`): called by the header with its right-hand row (laid out right
/// to left, after the Add button) and inner rectangle, so the bar sits in the header row without editing the rest of
/// `regions/header.rs`. Lane SKR's skeleton: **Start** alone while no round is live; then **Target ▶**, **Mine ▶**
/// and a **Round** menu (Submit, Hint, Reveal, End), which becomes a **Rate yours** menu ("Rate 1" … "Rate 5", "Skip") while the
/// blind rating is asked. Short, so it fits between the Presets group and the Add button at the minimum window; the
/// readout goes to the status line ([`Recreate::readout`]). Nothing at all until the shell plays rounds.
pub(crate) fn header(
    ui: &mut Ui,
    t: &crate::Theme,
    _rect: egui::Rect,
    view: &crate::editor::regions::HeaderView,
    out: &mut Intents,
) {
    let bar = &view.recreate;
    if !bar.available {
        return;
    }
    let button = |text: &str| Button::new(RichText::new(text).font(t.text.body.clone()));
    let game = |out: &mut Intents, g| out.push(Intent::Game(g));
    // Right to left: the gap after Add first, then the last control first.
    ui.add_space(t.space.section);
    if !bar.live {
        if ui
            .add(button("Start"))
            .on_hover_text("A Recreate round at level 1: rebuild the sound by ear")
            .clicked()
        {
            game(out, GameIntent::Start { level: 1 });
        }
        return;
    }
    if bar.asking {
        ui.menu_button(
            RichText::new("Rate yours").font(t.text.body.clone()),
            |ui| {
                ui.label(
                    RichText::new(
                        "How close is yours? 1: nothing like it … 5: can't tell them apart",
                    )
                    .font(t.text.label.clone())
                    .color(t.color.text.label),
                );
                for rating in 1..=5u8 {
                    if ui.button(format!("Rate {rating}")).clicked() {
                        game(out, GameIntent::Rate(Some(rating)));
                        ui.close();
                    }
                }
                if ui.button("Skip").clicked() {
                    game(out, GameIntent::Rate(None));
                    ui.close();
                }
            },
        );
    } else {
        ui.menu_button(RichText::new("Round").font(t.text.body.clone()), |ui| {
            if ui
                .add_enabled(bar.can_submit, Button::new("Submit"))
                .clicked()
            {
                game(out, GameIntent::Submit);
                ui.close();
            }
            let revealable =
                matches!(bar.phase, Phase::Playing | Phase::Submitted | Phase::Passed) && bar.ready;
            if ui
                .add_enabled(revealable, Button::new("Hint"))
                .on_hover_text("The next hint (the round counts as assisted)")
                .clicked()
            {
                game(out, GameIntent::Hint);
                ui.close();
            }
            if ui
                .add_enabled(revealable, Button::new("Reveal"))
                .on_hover_text("Give up: load the target (the round counts as assisted)")
                .clicked()
            {
                game(out, GameIntent::Reveal);
                ui.close();
            }
            if ui
                .button("End")
                .on_hover_text("End the round and bring your own patch back")
                .clicked()
            {
                game(out, GameIntent::End(EndChoice::Restore));
                ui.close();
            }
        });
    }
    let on = |l| bar.listen == l;
    let flip = |l| if on(l) { Listen::Off } else { l };
    let mine = format!("Mine {}", glyph::PLAY);
    if ui
        .add_enabled(bar.ready, button(&mine).selected(on(Listen::Mine)))
        .clicked()
    {
        game(out, GameIntent::Listen(flip(Listen::Mine)));
    }
    let target = format!("Target {}", glyph::PLAY);
    if ui
        .add_enabled(bar.ready, button(&target).selected(on(Listen::Target)))
        .clicked()
    {
        game(out, GameIntent::Listen(flip(Listen::Target)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use enth_session::game::AbState;

    fn round(phase: Phase) -> RoundView {
        let mut r = RoundView::default();
        r.phase = phase;
        r.code = "1000-0000-0001-AB".to_owned();
        r.ab = AbState {
            target_ready: true,
            ..AbState::default()
        };
        r
    }

    #[test]
    fn no_shell_no_bar_and_a_shell_without_a_round_offers_start() {
        let mut recreate = Recreate::default();
        assert_eq!(recreate.view(), BarView::default());
        recreate.set_round(None);
        let v = recreate.view();
        assert!(v.available && !v.live);
    }

    #[test]
    fn the_first_submit_asks_the_blind_rating_and_holds_the_score_back() {
        let mut recreate = Recreate::default();
        recreate.set_round(Some(round(Phase::Playing)));
        recreate.push(GameIntent::Submit);
        assert!(recreate.asking());
        assert!(!recreate.view().can_submit);
        let scored = JobResult::Failed {
            message: "a stand-in".into(),
        };
        let spec = SpecPreset::by_id(1).unwrap().spec(LATEST_GENERATOR, 3);
        let prepared = enth_challenge::prepare_round(&spec).unwrap();
        let report = enth_challenge::score_round(&prepared, prepared.start()).unwrap();
        let back = recreate.hold_scores(vec![
            (JobId(1), JobResult::Scored(Box::new(report))),
            (JobId(2), scored.clone()),
        ]);
        assert_eq!(back, vec![(JobId(2), scored)], "the score waits");
        recreate.push(GameIntent::Rate(Some(4)));
        assert!(!recreate.asking());
        let back = recreate.hold_scores(Vec::new());
        assert!(matches!(
            back.as_slice(),
            [(JobId(1), JobResult::Scored(_))]
        ));
        assert_eq!(
            recreate.queued(),
            [GameIntent::Submit, GameIntent::Rate(Some(4))]
        );
        // A later submit (the first score is in) asks nothing.
        let mut scored = round(Phase::Submitted);
        scored.first = Some(enth_session::game::SubmitRecord::default());
        recreate.set_round(Some(scored));
        recreate.push(GameIntent::Submit);
        assert!(!recreate.asking());
    }

    #[test]
    fn the_readout_says_where_the_round_is() {
        let mut r = round(Phase::Submitted);
        r.active_ms = 83_000;
        r.last = Some(enth_session::game::SubmitRecord {
            score: enth_session::game::ScoreSummary {
                passed: false,
                sound: 7_245,
                structure: 6_500,
                combined: 6_870,
            },
            ..Default::default()
        });
        let v = BarView::of(Some(&r), false, None);
        assert_eq!(
            v.readout,
            "Not yet • Sound 72.5 % • Structure 65.0 % • 1:23"
        );
        assert!(v.can_submit);
        r.pending.preparing = true;
        assert_eq!(
            BarView::of(Some(&r), false, None).readout,
            "Preparing the target…"
        );
        assert_eq!(
            BarView::of(Some(&r), false, Some("not ready")).readout,
            "not ready"
        );
    }
}
