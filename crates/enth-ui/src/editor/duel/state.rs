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
//! - **The clipboard** is written by the card itself, with egui's `copy_text`, only inside a click on **Copy**. It is
//!   read only on an explicit Paste: the platform's own (Ctrl+V, ⌘V) into the focused paste box, or the card's
//!   **Paste** button, whose click (and nothing else) asks the integration for the clipboard with egui's
//!   `ViewportCommand::RequestPaste`; the `Event::Paste` that answers it fills the box and is read at once. **Read**
//!   hands the box's text to paste-anything ([`super::paste`]).
//! - **The duel history** is the shell's file (lane DUI, `enth-plugin`'s `duel/history.rs`): the shell hands its rows
//!   over in [`ShellDuel::history`], and takes what the card asks of it after the frame ([`Duel::take_asks`]):
//!   **Share**, a history entry to reopen, the codes a Read imported (so it keeps them), the friend's name typed.

use enth_challenge::ChallengeCode;
use enth_session::duel::{DuelEdit, DuelError, DuelId, DuelView};

/// The most characters a friend's name keeps.
pub const NAME_CHARS: usize = 40;

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
    /// **Paste** was clicked: the card asked the integration for the clipboard (`ViewportCommand::RequestPaste`, in
    /// the click and nowhere else) and waits for the `Event::Paste` that answers it.
    Paste,
    /// The clipboard's text arrived for a Paste click: it replaces the paste box's text, which is read at once.
    Pasted(String),
    /// Reopen a duel kept in the history (the shell restores it: your result, its salt and codes, their codes).
    OpenHistory(u64),
    /// The friend's name, typed for the open duel (optional, kept only in the history on this computer; never in a
    /// code).
    Name(String),
}

/// What kind of code a Read imported into the duel (the shell keeps the friend's codes in the history).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportKind {
    /// A challenge code (a duel opened on it).
    Challenge,
    /// Their scorecard.
    Scorecard,
    /// Their answer code.
    Answer,
}

/// A code a Read imported, in its canonical text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Imported {
    /// What it was.
    pub kind: ImportKind,
    /// Its canonical text.
    pub text: String,
}

/// What the card asks of the shell after a frame ([`Duel::take_asks`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShellAsks {
    /// **Share result** was clicked.
    pub share: bool,
    /// A history entry to reopen.
    pub open: Option<u64>,
    /// The codes the last Read imported into the open duel, to keep in the history.
    pub imported: Vec<Imported>,
    /// The friend's name, typed for the open duel.
    pub name: Option<String>,
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
    /// The duel history, newest first (lane DUI: the shell's per-user file; the card shows the first rows).
    pub history: Vec<super::history::HistoryRow>,
    /// The history entry the open duel is kept in, if any.
    pub entry: Option<u64>,
    /// The friend's name kept for the open duel (empty: none).
    pub name: String,
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
    /// What the shell takes after the frame: Share, a history entry to reopen, the imported codes, a typed name.
    asks: ShellAsks,
    /// **Paste** was clicked and the clipboard's text has not arrived yet.
    paste_pending: bool,
    /// The name field's text, and the history entry it was filled from.
    name: String,
    name_for: Option<(Option<DuelId>, Option<u64>)>,
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
        // The name field follows the open duel and its history entry.
        let entry = (self.view.as_ref().map(|v| v.id), shell.entry);
        if self.name_for != Some(entry) {
            self.name.clone_from(&shell.name);
            self.name_for = Some(entry);
        }
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

    /// Whether a Paste click waits for the clipboard's text.
    pub fn paste_pending(&self) -> bool {
        self.paste_pending
    }

    /// The name field's text.
    pub fn name_text(&self) -> &str {
        &self.name
    }

    /// The name field's text, for the field to edit (the card's own state: the shell hears of it with
    /// [`DuelIntent::Name`] when the field is left).
    pub(crate) fn name_buffer(&mut self) -> &mut String {
        &mut self.name
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
                self.asks.share = true;
                self.said = None;
                self.copied = None;
                return;
            }
            DuelIntent::Paste => {
                self.paste_pending = true;
                self.said = None;
                return;
            }
            DuelIntent::Pasted(text) => {
                self.paste_pending = false;
                text.clone_into(&mut self.paste);
                self.queued.push(DuelIntent::Read);
                return;
            }
            DuelIntent::OpenHistory(id) => {
                self.asks.open = Some(*id);
                self.said = None;
                self.read.clear();
                return;
            }
            DuelIntent::Name(name) => {
                let name: String = name.trim().chars().take(NAME_CHARS).collect();
                self.name.clone_from(&name);
                self.asks.name = Some(name);
                return;
            }
            _ => {}
        }
        self.queued.push(intent);
    }

    /// Whether intents wait for the session or the shell.
    pub fn has_queued(&self) -> bool {
        !self.queued.is_empty() || self.asks != ShellAsks::default()
    }

    /// After the frame (the shell): whether Share was clicked. The shell reads the player's result, draws the duel's
    /// salt and makes the codes on its worker; they come back through `set_duel`. ([`Duel::take_asks`] takes it with
    /// the rest of what the card asks.)
    pub fn take_share(&mut self) -> bool {
        std::mem::take(&mut self.asks.share)
    }

    /// After the frame (the shell): everything the card asks of it ([`ShellAsks`]), taken once.
    pub fn take_asks(&mut self) -> ShellAsks {
        std::mem::take(&mut self.asks)
    }

    /// After the frame: the queued intents, applied to the session's duel edit trait in order. A refusal is shown on
    /// the card.
    pub fn apply(&mut self, session: &mut dyn DuelEdit) {
        for intent in std::mem::take(&mut self.queued) {
            let result: Result<(), DuelError> = match intent {
                DuelIntent::Toggle
                | DuelIntent::Share
                | DuelIntent::PasteText(_)
                | DuelIntent::Copied(_)
                | DuelIntent::Paste
                | DuelIntent::Pasted(_)
                | DuelIntent::OpenHistory(_)
                | DuelIntent::Name(_) => continue,
                DuelIntent::Read => {
                    let read = super::paste::read(&self.paste, session);
                    self.said = read.refused;
                    self.read = read.lines;
                    self.asks.imported.extend(read.imported);
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
