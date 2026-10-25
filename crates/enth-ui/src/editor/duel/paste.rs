//! **Owner: lane DUI** (the duel card, feature `duel`). Pre-declared by lane S0e (R4/R5); **lane SKD** wrote the
//! first Read for freeze point F2d.
//!
//! The paste box (docs/duels.md, "Sharing and paste"): it takes a whole chat message and shows every token that decodes
//! (a challenge, a scorecard, an answer), through `enth-challenge`'s paste-anything. **The clipboard is read only on an
//! explicit Paste**, never on its own:
//!
//! - the platform's Paste (Ctrl+V, ⌘V) into the focused box, which the integration (egui-baseview in the plugin)
//!   turns into text;
//! - the card's **Paste** button ([`button`]): its click, and nothing else in the editor, sends egui's
//!   `ViewportCommand::RequestPaste`, so the integration reads the clipboard and answers with an `Event::Paste`
//!   ([`arrived`] takes it while the click waits, unless the box has the keyboard focus and takes it itself); the text
//!   replaces the box's and is read at once. egui-winit (the dev window) answers it; egui-baseview answers it from
//!   fork patch 13 (lane FK4) on, and until then the click changes nothing but the note under the box, which says to
//!   press Ctrl+V in the box instead.
//!
//! **Read** ([`read`]) hands the box's text to [`paste::paste`]. Decoding an answer code regenerates the challenge's
//! target in parameter space (no render), so a Read stays well inside one frame.

use egui::{Event, Id, Ui, ViewportCommand};
use enth_challenge::paste::{self, Found};
use enth_session::duel::{DuelEdit, FriendAnswer, FriendScorecard};

use super::card::button as card_button;
use super::state::{Duel, DuelIntent, ImportKind, Imported};
use crate::Theme;
use crate::editor::cards::{Intent, Intents};

/// The paste box's id (the Paste button checks whether it has the keyboard focus).
pub fn box_id() -> Id {
    Id::unique("enth.duel.paste")
}

/// **Paste**: the one control that reads the clipboard. Its click asks the integration for it
/// (`ViewportCommand::RequestPaste`) and marks the card as waiting for the answer.
pub fn button(ui: &mut Ui, t: &Theme, out: &mut Intents) {
    let response = card_button(
        ui,
        t,
        "Paste",
        "Paste from clipboard",
        true,
        DuelIntent::Paste,
        out,
    )
    .on_hover_text("Put your friend's message from the clipboard in the box, and read it");
    if response.clicked() {
        ui.ctx().send_viewport_cmd(ViewportCommand::RequestPaste);
    }
}

/// While a Paste click waits: the text of the `Event::Paste` that answers it, pushed as [`DuelIntent::Pasted`]. A box
/// with the keyboard focus takes a paste itself (the platform's Paste), so then nothing is taken here.
pub fn arrived(duel: &Duel, ui: &Ui, out: &mut Intents) {
    if !duel.paste_pending() || ui.ctx().memory(|m| m.has_focus(box_id())) {
        return;
    }
    let pasted = ui.input(|i| {
        i.events.iter().find_map(|e| match e {
            Event::Paste(text) => Some(text.clone()),
            _ => None,
        })
    });
    if let Some(text) = pasted {
        out.push(Intent::Duel(DuelIntent::Pasted(text)));
    }
}

/// What a Read found and did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Read {
    /// One line per code found, saying what it was and what happened to it.
    pub lines: Vec<String>,
    /// Why something was refused (a typo, "update enth", a code for another duel), with its hint; `None` when all went.
    pub refused: Option<String>,
    /// The codes imported into the duel, in their canonical text (the shell keeps the friend's in the history).
    pub imported: Vec<Imported>,
}

/// Read `text`: every code that decodes, in order, imported into the duel. A challenge opens a duel on it (when none
/// is open on it); a scorecard is held as reported; an answer is held sealed until the player has played; an ear
/// challenge or ear result is named, for the Ear view. Code-looking tokens that do not decode say why.
pub fn read(text: &str, session: &mut dyn DuelEdit) -> Read {
    let pasted = paste::paste(text);
    let mut out = Read::default();
    let mut refusals = Vec::new();
    for found in pasted.found {
        let line = match found {
            Found::Challenge(code) => {
                let canonical = code.to_string();
                let open = session.duel().is_some_and(|v| v.code == canonical);
                if open {
                    format!("Challenge {canonical}: the duel is already on it")
                } else {
                    match session.duel_open(&canonical) {
                        Ok(_) => {
                            out.imported.push(Imported {
                                kind: ImportKind::Challenge,
                                text: canonical.clone(),
                            });
                            format!("Challenge {canonical}: a duel on it is open, play it")
                        }
                        Err(e) => {
                            refusals.push(format!("{e}: {}", e.hint()));
                            continue;
                        }
                    }
                }
            }
            Found::Scorecard(card) => {
                let text = card.encode();
                let [_, claim] = card.share_lines();
                match session.duel_import_scorecard(FriendScorecard::new(card, text.clone())) {
                    Ok(_) => {
                        out.imported.push(Imported {
                            kind: ImportKind::Scorecard,
                            text,
                        });
                        format!("Their scorecard, as reported by their app: {claim}")
                    }
                    Err(e) => {
                        refusals.push(format!("{e}: {}", e.hint()));
                        continue;
                    }
                }
            }
            Found::Answer(answer) => {
                let code = answer.challenge().to_string();
                let text = answer.encode();
                match session.duel_import_answer(FriendAnswer::new(code, text.clone())) {
                    Ok(_) => {
                        out.imported.push(Imported {
                            kind: ImportKind::Answer,
                            text,
                        });
                        "Their answer code: held until you have played".to_owned()
                    }
                    Err(e) => {
                        refusals.push(format!("{e}: {}", e.hint()));
                        continue;
                    }
                }
            }
            Found::EarChallenge(code) => {
                format!("An ear challenge ({code}): it opens in the Ear view")
            }
            Found::EarResult(_) => "An ear result: it is checked in the Ear view".to_owned(),
            _ => "A kind of code this card does not take".to_owned(),
        };
        out.lines.push(line);
    }
    for r in pasted.refused {
        refusals.push(format!("{}: {}", r.text, r.error.hint()));
    }
    if out.lines.is_empty() && refusals.is_empty() {
        refusals.push("no code in the text: paste your friend's whole message".to_owned());
    }
    out.refused = (!refusals.is_empty()).then(|| refusals.join("; "));
    out
}
