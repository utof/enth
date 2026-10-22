//! **Owner: lane DUI** (the duel card, feature `duel`). Pre-declared by lane S0e (R4/R5); **lane SKD** wrote the first
//! seam for freeze point F2d, which lane DUI grows into the card's state.
//!
//! The editor's duel state and the seam to the shell (docs/duels.md, "The flow"): [`DuelIntent`] (what the card asks
//! for), `Editor::set_duel` (the session's duel read model and the shell's codes, handed over before each frame) and
//! `Editor::apply_duel` (the intents applied to the session's [`DuelEdit`] after the frame). It mirrors
//! `create/state.rs` and `learn/state.rs`.
//!
//! Who does what:
//!
//! - **The session** (`enth-session`'s `duel`) holds the duel: the player's result, their scorecard as reported, their
//!   answer sealed until the player has played, the verification the shell's worker measured, the load as one undo
//!   step. [`Duel::apply`] makes those calls.
//! - **The shell** makes the codes, because only it has OS randomness (the salt) and the game worker: the card's
//!   **Share** is a request the shell takes after the frame ([`Duel::take_share`]), and the codes come back in
//!   [`ShellDuel`] with the next `set_duel`.
//! - **The clipboard** is written by the card itself, with egui's `copy_text`, only inside a click on **Copy**; it is
//!   never read by the editor or the shell. Pasting is the platform's own Paste (Ctrl+V, ⌘V) into the paste box, which
//!   egui-baseview turns into text, and **Read** hands that text to paste-anything ([`super::paste`]).

use enth_challenge::ChallengeCode;
use enth_session::duel::{DuelEdit, DuelError, DuelView};

/// What the card asks for. The editor queues them while drawing and applies them after the frame ([`Duel::apply`]);
/// the card's own (open, close, typing in the paste box, a copy note) take effect at once, so the next pass draws them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DuelIntent {
    /// Show or hide the card (the overview's **Duel**, the card's ×).
    Toggle,
    /// Make the share message and the answer code from the player's result (the shell, after the frame).
    Share,
    /// The paste box's text (typed, or pasted with the platform's Paste).
    PasteText(String),
    /// Read the paste box: every code in it that decodes (paste-anything), imported into the duel.
    Read,
    /// Check their answer on this computer (the session's verify job, run by the shell's worker).
    Verify,
    /// Play their checked patch once.
    Audition,
    /// Stop it.
    StopAudition,
    /// Load their checked patch as one undo step.
    Use,
    /// Close the duel (a running check is cancelled).
    Close,
    /// The card wrote this to the clipboard on a click: "message" or "answer" (a note on the card; nothing else).
    Copied(&'static str),
}

/// The codes the shell made for the open duel ([`ShellDuel::codes`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShareCodes {
    /// The challenge code the codes are for (canonical text).
    pub code: String,
    /// The share message: the two human lines ("enth duel · Level 2 · …", "93.4 % in 2:41 · …") and the scorecard,
    /// one per line. Sent when you finish: it spoils nothing.
    pub message: String,
    /// The scorecard alone.
    pub scorecard: String,
    /// The answer code: it gives your patch away, so it is sent after your friend has played.
    pub answer: String,
    /// Symbols in the scorecard (code v1's check symbol included, hyphens not).
    pub scorecard_symbols: usize,
    /// Symbols in the answer code.
    pub answer_symbols: usize,
}

/// What the shell adds to the session's read model before each frame (`Editor::set_duel`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShellDuel {
    /// The codes of the open duel, once made.
    pub codes: Option<ShareCodes>,
    /// The worker is making them.
    pub making: bool,
    /// What the shell last said (why it could not share, why an audition did not play), for the card.
    pub said: Option<String>,
}

/// The editor's duel state (see the [module docs](self)).
#[derive(Debug, Default)]
pub struct Duel {
    /// The shell hands the editor a duel read model (so the card can open).
    available: bool,
    open: bool,
    view: Option<DuelView>,
    shell: ShellDuel,
    paste: String,
    /// What the last Read found and imported, one line each.
    read: Vec<String>,
    /// The last refusal, for the card.
    said: Option<String>,
    /// The last copy note.
    copied: Option<&'static str>,
    queued: Vec<DuelIntent>,
    /// Share was clicked: the shell takes it after the frame.
    share: bool,
}

impl Duel {
    /// The session's duel and the shell's codes, before each frame. A shell that calls it says duels are available.
    pub fn set_duel(&mut self, view: Option<DuelView>, shell: ShellDuel) {
        self.available = true;
        // A duel that opens (a pasted scorecard, a share) shows the card.
        if view.is_some() && self.view.as_ref().map(|v| v.id) != view.as_ref().map(|v| v.id) {
            self.open = true;
        }
        self.view = view;
        self.shell = shell;
    }

    /// Whether the shell says duels are available.
    pub fn available(&self) -> bool {
        self.available
    }

    /// Whether the card shows.
    pub fn card_open(&self) -> bool {
        self.available && self.open
    }

    /// The session's duel, if one is open.
    pub fn view(&self) -> Option<&DuelView> {
        self.view.as_ref()
    }

    /// What the shell said and made.
    pub fn shell(&self) -> &ShellDuel {
        &self.shell
    }

    /// The paste box's text.
    pub fn paste_text(&self) -> &str {
        &self.paste
    }

    /// What the last Read found, one line each.
    pub fn read_lines(&self) -> &[String] {
        &self.read
    }

    /// The last refusal (the session's, or the paste's).
    pub fn said(&self) -> Option<&str> {
        self.said.as_deref()
    }

    /// The last copy note ("message", "answer").
    pub fn copied(&self) -> Option<&'static str> {
        self.copied
    }

    /// The challenge to play, when a duel is open and the player has not played it: what **Play** starts.
    pub fn to_play(&self) -> Option<ChallengeCode> {
        let view = self.view.as_ref()?;
        if view.own.is_some() {
            return None;
        }
        ChallengeCode::decode(&view.code).ok()
    }

    /// Queue an intent for [`Duel::apply`]; the card's own take effect at once, so the next pass draws them.
    pub fn push(&mut self, intent: DuelIntent) {
        match &intent {
            DuelIntent::Toggle => {
                self.open = !self.open;
                return;
            }
            DuelIntent::PasteText(text) => {
                text.clone_into(&mut self.paste);
                return;
            }
            DuelIntent::Copied(what) => {
                self.copied = Some(what);
                return;
            }
            DuelIntent::Share => {
                self.share = true;
                self.said = None;
                self.copied = None;
                return;
            }
            _ => {}
        }
        self.queued.push(intent);
    }

    /// Whether intents wait for the session or the shell.
    pub fn has_queued(&self) -> bool {
        !self.queued.is_empty() || self.share
    }

    /// After the frame (the shell): whether Share was clicked. The shell reads the player's result, draws the duel's
    /// salt and makes the codes on its worker; they come back through `set_duel`.
    pub fn take_share(&mut self) -> bool {
        std::mem::take(&mut self.share)
    }

    /// After the frame: the queued intents, applied to the session's duel edit trait in order. A refusal is shown on
    /// the card.
    pub fn apply(&mut self, session: &mut dyn DuelEdit) {
        for intent in std::mem::take(&mut self.queued) {
            let result: Result<(), DuelError> = match intent {
                DuelIntent::Toggle
                | DuelIntent::Share
                | DuelIntent::PasteText(_)
                | DuelIntent::Copied(_) => continue,
                DuelIntent::Read => {
                    let read = super::paste::read(&self.paste, session);
                    self.said = read.refused;
                    self.read = read.lines;
                    if self.said.is_none() {
                        self.paste.clear();
                    }
                    continue;
                }
                DuelIntent::Verify => session.duel_verify().map(|_| ()),
                DuelIntent::Audition => session.duel_audition(),
                DuelIntent::StopAudition => {
                    session.duel_stop_audition();
                    Ok(())
                }
                DuelIntent::Use => session.duel_use_answer(""),
                DuelIntent::Close => {
                    session.duel_close();
                    self.read.clear();
                    Ok(())
                }
            };
            self.said = result.err().map(|e| format!("{e}: {}", e.hint()));
        }
        self.view = session.duel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use enth_patch::Patch;
    use enth_session::Session;

    #[test]
    fn the_cards_own_intents_take_effect_at_once_and_share_goes_to_the_shell() {
        let mut d = Duel::default();
        d.set_duel(None, ShellDuel::default());
        assert!(!d.card_open());
        d.push(DuelIntent::Toggle);
        assert!(d.card_open());
        d.push(DuelIntent::PasteText("hello".to_owned()));
        assert_eq!(d.paste_text(), "hello");
        d.push(DuelIntent::Share);
        assert!(d.has_queued());
        assert!(d.take_share());
        assert!(!d.take_share(), "taken once");
        assert!(!d.has_queued());
    }

    #[test]
    fn verify_without_a_duel_is_refused_on_the_card() {
        let mut s = Session::new(Patch::init());
        let mut d = Duel::default();
        d.set_duel(None, ShellDuel::default());
        d.push(DuelIntent::Verify);
        d.apply(&mut s);
        assert!(
            d.said().is_some_and(|t| t.contains("no duel")),
            "{:?}",
            d.said()
        );
    }
}
