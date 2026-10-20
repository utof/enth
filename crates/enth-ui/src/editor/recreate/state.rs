//! **Owner: lane RUI** (the Recreate UI). The editor's Recreate state and the round intents.
//!
//! [`Recreate`] is editor state (it survives the window closing): the round view the shell handed over this frame,
//! whether the header is in Recreate mode, the level and code the bar starts from, the settings page, the blind
//! rating's question and the score it holds back, the results drawer's toggle, the dispute note, and the round
//! intents waiting for the shell. Lane SKR's seam is unchanged (docs/editor.md, "The Recreate hook"):
//!
//! 1. the shell hands the session's `RoundView` over before the frame ([`Recreate::set_round`]);
//! 2. the controls push [`Intent::Game`](crate::editor::cards::Intent::Game); the editor hands each to
//!    [`Recreate::push`] at the end of the frame, which keeps view changes ([`ViewIntent`]) to itself and queues
//!    the round's intents for the shell;
//! 3. the shell applies the queue to its `GameEdit` ([`Recreate::apply`], [`apply_game`]) with a fresh seed;
//! 4. worker results go through [`Recreate::hold_scores`] first: a score waits while the blind rating is asked.

use enth_challenge::spec::{LATEST_GENERATOR, SpecPreset};
use enth_challenge::{Agreement, ChallengeCode, RandomSpec};
use enth_session::game::{
    EndChoice, GameEdit, GameError, HINT_LEVELS, JobId, JobResult, Listen, Phase, RoundSource,
    RoundView,
};

use super::overlay::RoundOverlay;
use super::settings::Settings;
use crate::theme::glyph;

/// What the Recreate controls ask for. Round intents are queued by the editor and applied by the shell to its
/// `GameEdit` after the frame ([`apply_game`]); [`GameIntent::View`] changes only the editor's own state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GameIntent {
    /// Start a round from a level's preset (1–5) with the shell's fresh seed.
    Start {
        /// The level, 1–5 (code v1's presets).
        level: u8,
    },
    /// Start a round from these settings with the shell's fresh seed (the spec's own seed is replaced): the settings
    /// page's Start, and "New round" (the round's settings again).
    New(RandomSpec),
    /// Start a round from a code: a pasted one, or "Try again" (the same challenge from its start).
    Code(ChallengeCode),
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
    /// The agreement vote: the score felt too low, about right or too high.
    Vote(Agreement),
    /// "Disagree…": the dispute with its note (logged at once).
    Dispute(String),
    /// End the round.
    End(EndChoice),
    /// A change to the editor's own Recreate state; never reaches the shell.
    View(ViewIntent),
}

/// A change to the editor's Recreate state ([`GameIntent::View`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewIntent {
    /// Enter (true) or leave Recreate mode: the challenge bar replaces the header's Presets group.
    Mode(bool),
    /// Pick a level for the next round (its preset's settings).
    Level(u8),
    /// The code field's text.
    CodeText(String),
    /// Start: from the code field if it holds text (decoded here; a code that does not decode says why), else from
    /// the settings.
    Start,
    /// Open (true) or close the settings page.
    Settings(bool),
    /// Show the results drawer (true) or the modulation overview (false) in the overview slot.
    Results(bool),
    /// Open the dispute note (`Some`, with its text), edit it, or close it (`None`).
    Note(Option<String>),
}

/// Apply queued round intents in order. A refused one changes nothing; the last refusal is returned (the bar shows
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
            GameIntent::New(spec) => game.game_start(RoundSource::Settings(RandomSpec {
                seed: seed(),
                ..spec
            })),
            GameIntent::Code(code) => game.game_start(RoundSource::Code(code)),
            GameIntent::Listen(listen) => game.game_listen(listen),
            GameIntent::Submit => game.game_submit().map(drop),
            GameIntent::Rate(Some(rating)) => game.game_rate(rating),
            GameIntent::Rate(None) | GameIntent::View(_) => Ok(()),
            GameIntent::Hint => game.game_hint().map(drop),
            GameIntent::Reveal => game.game_reveal(),
            GameIntent::Vote(agreement) => game.game_vote(agreement),
            GameIntent::Dispute(note) => game.game_dispute(&note),
            GameIntent::End(choice) => game.game_end(choice),
        };
        if let Err(e) = result {
            refused = Some(e);
        }
    }
    refused
}

/// What the challenge bar shows, built from the round and the editor's state each frame ([`Recreate::view`]).
/// Plain values, so the header's view stays comparable.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "a view model: independent facts the bar shows"
)]
pub struct BarView {
    /// The shell plays rounds (it handed the editor a round view): without it the bar draws nothing.
    pub available: bool,
    /// Recreate mode: the bar replaces the header's Presets group (always while a round is live).
    pub mode: bool,
    /// A round is live (in any phase, Preparing and Broken included).
    pub live: bool,
    /// Where the round is.
    pub phase: Phase,
    /// The target is being prepared.
    pub preparing: bool,
    /// The target is ready (Target ▶ can play).
    pub ready: bool,
    /// The A/B switch.
    pub listen: Listen,
    /// Submit is possible now.
    pub can_submit: bool,
    /// Hint is possible now.
    pub can_hint: bool,
    /// Reveal is possible now.
    pub can_reveal: bool,
    /// Hint levels used, of [`HINT_LEVELS`].
    pub hints: u8,
    /// The level the dots show, 1–5: the round's difficulty while one is live, else the next round's settings'.
    pub level: u8,
    /// The next round's settings are not a level's (the dots read "custom").
    pub custom: bool,
    /// The blind rating is being asked (the score is held back).
    pub asking: bool,
    /// m:ss of active time ("Preparing…" while the target is prepared).
    pub time: String,
    /// The code field's text (before a round).
    pub code_text: String,
    /// The one-line readout for the status line: phase, scores, time, or why something was refused.
    pub readout: String,
    /// The challenge code (the readout's lead, and the timer's tooltip).
    pub code: String,
}

/// m:ss of active time.
pub fn clock(ms: u64) -> String {
    let s = ms / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

/// A score in hundredths of a percent, with one decimal.
pub fn percent(hundredths: u16) -> String {
    format!("{:.1} %", f64::from(hundredths) / 100.0)
}

/// The readout line: phase, scores and time, or what the round waits for.
fn readout(r: &RoundView, asking: bool, refused: Option<&str>) -> String {
    let time = clock(r.active_ms);
    let scores = r.last.map(|s| {
        format!(
            "Sound {} {} Structure {}",
            percent(s.score.sound),
            glyph::BULLET,
            percent(s.score.structure)
        )
    });
    if let Some(e) = refused {
        return e.to_owned();
    }
    if matches!(r.phase, Phase::Broken | Phase::Unknown) {
        let why = r.error.as_ref().map(GameError::hint).unwrap_or_default();
        return format!("This round cannot go on: End it. {why}");
    }
    if r.pending.preparing {
        return "Preparing the target…".to_owned();
    }
    if asking {
        return "How close is yours? Rate it before the score".to_owned();
    }
    if r.pending.scoring {
        return "Scoring…".to_owned();
    }
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
    } else {
        line
    }
}

/// The editor's Recreate state. See the [module docs](self).
#[derive(Debug, Default)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "editor view state: independent toggles"
)]
pub struct Recreate {
    /// The shell handed over a round view at least once.
    available: bool,
    round: Option<RoundView>,
    /// Recreate mode was entered (it stays on after a round ends, until the player leaves it).
    mode: bool,
    /// The next round's settings (the bar's level dots, the settings page).
    pub(super) settings: Settings,
    /// The code field's text.
    code_text: String,
    /// The settings page covers the body.
    pub(super) page: bool,
    /// The `dim_unused` assist (the settings page).
    pub(super) dim_unused: bool,
    /// The blind rating is being asked.
    asking: bool,
    /// The player chose the modulation overview over the results drawer.
    results_hidden: bool,
    /// The dispute note, while it is being written.
    pub(super) note: Option<String>,
    /// The last vote or dispute, as the drawer thanks for it ("Logged").
    pub(super) thanked: Option<String>,
    queued: Vec<GameIntent>,
    /// Results held back while the rating is asked.
    held: Vec<(JobId, JobResult)>,
    /// The last refusal of an intent (its hint).
    refused: Option<String>,
    /// The round's overlay, rebuilt with each round view.
    overlay: RoundOverlay,
    /// A view change at the frame's end asks for one more pass.
    changed: bool,
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
        if round.is_none() {
            self.note = None;
        }
        self.overlay = round
            .as_ref()
            .map(|r| RoundOverlay::of(r, self.dim_unused))
            .unwrap_or_default();
        self.round = round;
    }

    /// The round view of this frame, if a round is live.
    pub fn round(&self) -> Option<&RoundView> {
        self.round.as_ref()
    }

    /// Whether the shell plays rounds at all.
    pub fn available(&self) -> bool {
        self.available
    }

    /// Recreate mode: entered by the player, or a round is live.
    pub fn in_mode(&self) -> bool {
        self.available && (self.mode || self.round.is_some())
    }

    /// The next round's settings.
    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// The `dim_unused` assist is on.
    pub fn dim_unused(&self) -> bool {
        self.dim_unused
    }

    /// What the bar shows.
    pub fn view(&self) -> BarView {
        if !self.available {
            return BarView::default();
        }
        let refused = self.refused.as_deref();
        let next_level = self.settings.current_level();
        let Some(r) = self.round.as_ref() else {
            return BarView {
                available: true,
                mode: self.mode,
                level: next_level,
                custom: self.settings.preset().is_none(),
                code_text: self.code_text.clone(),
                readout: refused.unwrap_or_default().to_owned(),
                ..BarView::default()
            };
        };
        let open = r.phase.is_open() && r.ab.target_ready;
        BarView {
            available: true,
            mode: true,
            live: true,
            phase: r.phase,
            preparing: r.pending.preparing,
            ready: r.ab.target_ready,
            listen: r.ab.listen,
            can_submit: open && !r.pending.scoring && !self.asking,
            can_hint: open && !r.pending.hinting && r.hints < HINT_LEVELS,
            can_reveal: open,
            hints: r.hints,
            level: r.difficulty.map_or(next_level, Settings::level_of),
            custom: false,
            asking: self.asking,
            time: if r.pending.preparing {
                "Preparing…".to_owned()
            } else {
                clock(r.active_ms)
            },
            code_text: String::new(),
            readout: readout(r, self.asking, refused),
            code: r.code.clone(),
        }
    }

    /// Queue a control's intent at the end of the frame. View changes apply here; round intents wait for the shell.
    /// The first Submit of a round without a rating starts the rating question; a rating (or Skip) ends it, and so do
    /// starting and ending a round.
    pub fn push(&mut self, intent: GameIntent) {
        // Whatever it was, the bar or the drawer shows it in the next pass.
        self.changed = true;
        match &intent {
            GameIntent::View(view) => {
                self.view_change(view.clone());
                return;
            }
            GameIntent::Submit => {
                if self
                    .round
                    .as_ref()
                    .is_some_and(|r| r.first.is_none() && r.rating.is_none())
                {
                    self.asking = true;
                }
                self.results_hidden = false;
            }
            GameIntent::Rate(_) | GameIntent::Hint => self.results_hidden = false,
            GameIntent::Start { .. }
            | GameIntent::New(_)
            | GameIntent::Code(_)
            | GameIntent::End(_) => {
                self.asking = false;
                self.results_hidden = false;
                self.note = None;
                self.thanked = None;
                self.page = false;
                self.mode = true;
            }
            GameIntent::Vote(_) => {
                self.thanked = Some("Thanks: your vote is in the round log".to_owned());
            }
            GameIntent::Dispute(_) => {
                self.note = None;
                self.thanked = Some("Thanks: your disagreement is in the round log".to_owned());
            }
            GameIntent::Listen(_) | GameIntent::Reveal => {}
        }
        if matches!(intent, GameIntent::Rate(_)) {
            self.asking = false;
        }
        self.queued.push(intent);
    }

    fn view_change(&mut self, view: ViewIntent) {
        self.changed = true;
        match view {
            ViewIntent::Mode(on) => {
                self.mode = on;
                if !on {
                    self.page = false;
                    self.refused = None;
                }
            }
            ViewIntent::Level(level) => self.settings = Settings::level(level),
            ViewIntent::CodeText(text) => {
                self.code_text = text;
                self.refused = None;
            }
            ViewIntent::Start => {
                let text = self.code_text.trim();
                if text.is_empty() {
                    self.push(GameIntent::New(self.settings.spec(0)));
                } else {
                    match ChallengeCode::decode(text) {
                        Ok(code) => {
                            self.code_text.clear();
                            self.push(GameIntent::Code(code));
                        }
                        Err(e) => {
                            self.refused = Some(format!("That code does not work: {}", e.hint()));
                        }
                    }
                }
            }
            ViewIntent::Settings(open) => self.page = open,
            ViewIntent::Results(show) => self.results_hidden = !show,
            ViewIntent::Note(note) => self.note = note,
        }
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

    /// The results drawer has something to show: the rating question, a score, a hint, a reveal.
    pub fn has_results(&self) -> bool {
        let Some(r) = &self.round else {
            return false;
        };
        self.asking
            || r.submits > 0
            || r.pending.scoring
            || r.revealed
            || r.hints > 0
            || r.pending.hinting
    }

    /// The results drawer takes the overview slot (it has something to show and the player did not hide it).
    pub fn drawer_open(&self) -> bool {
        self.has_results() && (!self.results_hidden || self.asking)
    }

    /// The settings page covers the body.
    pub fn covers_body(&self) -> bool {
        self.page && self.in_mode()
    }

    /// The overlay to lay over the cards while a round is live.
    pub fn overlay(&self) -> Option<&RoundOverlay> {
        self.round.as_ref().map(|_| &self.overlay)
    }

    /// The queued intents (tests).
    pub fn queued(&self) -> &[GameIntent] {
        &self.queued
    }

    /// A view change happened at this frame's end: the editor asks egui for one more pass so it shows.
    pub(crate) fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
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
    fn no_shell_no_bar_and_a_shell_without_a_round_offers_recreate() {
        let mut recreate = Recreate::default();
        assert_eq!(recreate.view(), BarView::default());
        recreate.set_round(None);
        let v = recreate.view();
        assert!(v.available && !v.live && !v.mode);
        recreate.push(GameIntent::View(ViewIntent::Mode(true)));
        assert!(recreate.view().mode);
        assert!(
            recreate.queued().is_empty(),
            "view changes never reach the shell"
        );
    }

    #[test]
    fn the_first_submit_asks_the_blind_rating_and_holds_the_score_back() {
        let mut recreate = Recreate::default();
        recreate.set_round(Some(round(Phase::Playing)));
        recreate.push(GameIntent::Submit);
        assert!(recreate.asking());
        assert!(!recreate.view().can_submit);
        assert!(recreate.drawer_open(), "the drawer asks");
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
        assert_eq!(
            readout(&r, false, None),
            "Not yet • Sound 72.5 % • Structure 65.0 % • 1:23"
        );
        r.pending.preparing = true;
        assert_eq!(readout(&r, false, None), "Preparing the target…");
        assert_eq!(readout(&r, false, Some("not ready")), "not ready");
    }

    #[test]
    fn start_decodes_the_code_field_or_uses_the_settings() {
        let mut recreate = Recreate::default();
        recreate.set_round(None);
        recreate.push(GameIntent::View(ViewIntent::Level(3)));
        recreate.push(GameIntent::View(ViewIntent::Start));
        assert_eq!(
            recreate.queued(),
            [GameIntent::New(Settings::level(3).spec(0))]
        );
        let mut recreate = Recreate::default();
        recreate.set_round(None);
        recreate.push(GameIntent::View(ViewIntent::CodeText("not a code".into())));
        recreate.push(GameIntent::View(ViewIntent::Start));
        assert_eq!(recreate.queued(), []);
        assert!(
            recreate
                .view()
                .readout
                .starts_with("That code does not work")
        );
        let code = ChallengeCode::short(Settings::level(2).spec(99)).unwrap();
        recreate.push(GameIntent::View(ViewIntent::CodeText(code.to_string())));
        recreate.push(GameIntent::View(ViewIntent::Start));
        assert_eq!(recreate.queued(), [GameIntent::Code(code)]);
        assert_eq!(
            recreate.view().code_text,
            "",
            "the field empties once the code is taken"
        );
    }

    #[test]
    fn the_drawer_opens_with_a_score_a_hint_or_a_reveal_and_can_be_hidden() {
        let mut recreate = Recreate::default();
        let mut r = round(Phase::Playing);
        recreate.set_round(Some(r.clone()));
        assert!(!recreate.drawer_open(), "nothing to show before a submit");
        r.hints = 1;
        recreate.set_round(Some(r.clone()));
        assert!(recreate.drawer_open());
        recreate.push(GameIntent::View(ViewIntent::Results(false)));
        assert!(!recreate.drawer_open(), "the overview comes back");
        recreate.push(GameIntent::Hint);
        assert!(recreate.drawer_open(), "a new hint shows the drawer again");
    }
}
