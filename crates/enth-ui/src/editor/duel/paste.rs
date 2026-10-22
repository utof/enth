//! **Owner: lane DUI** (the duel card, feature `duel`). Pre-declared by lane S0e (R4/R5); **lane SKD** wrote the
//! first Read for freeze point F2d.
//!
//! The paste box (docs/duels.md, "Sharing and paste"): it takes a whole chat message and shows every token that decodes
//! (a challenge, a scorecard, an answer), through `enth-challenge`'s paste-anything. **The clipboard is read only on an
//! explicit Paste**, never on its own: the editor never reads it at all. The platform's Paste (Ctrl+V, ⌘V) into the
//! focused box is turned into text by egui-baseview, and **Read** ([`read`]) hands that text to [`paste::paste`].
//!
//! Decoding an answer code regenerates the challenge's target in parameter space (no render), so a Read stays well
//! inside one frame.

use enth_challenge::paste::{self, Found};
use enth_session::duel::{DuelEdit, FriendAnswer, FriendScorecard};

/// What a Read found and did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Read {
    /// One line per code found, saying what it was and what happened to it.
    pub lines: Vec<String>,
    /// Why something was refused (a typo, "update enth", a code for another duel), with its hint; `None` when all went.
    pub refused: Option<String>,
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
                        Ok(_) => format!("Challenge {canonical}: a duel on it is open, play it"),
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
                match session.duel_import_scorecard(FriendScorecard::new(card, text)) {
                    Ok(_) => format!("Their scorecard, as reported by their app: {claim}"),
                    Err(e) => {
                        refusals.push(format!("{e}: {}", e.hint()));
                        continue;
                    }
                }
            }
            Found::Answer(answer) => {
                let code = answer.challenge().to_string();
                match session.duel_import_answer(FriendAnswer::new(code, answer.encode())) {
                    Ok(_) => "Their answer code: held until you have played".to_owned(),
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
