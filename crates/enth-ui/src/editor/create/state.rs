//! **Owner: lane PUI** (the Create view, feature `create`). Pre-declared by lane S0s (R3); **lane SK3** wrote the
//! minimal seam for freeze point F2s, which PUI replaces or grows.
//!
//! The editor's Create state and the seam to the shell (docs/r3.md, "The Create view and the session"):
//! [`CreateIntent`] (what the view asks for: open the panel, choose or drop a file, start, cancel, audition a
//! candidate, stop, use this, close), `Editor::set_create` (the session's `CreateEdit::create()` read model, handed
//! over before each frame, as `set_round` hands over a round) and `Editor::apply_create` (the intents applied to the
//! session's `CreateEdit` after the frame). It mirrors `recreate/state.rs` and `learn/state.rs`. The view never reads
//! a file: a drop gives a path, and the shell's `enth-create` thread decodes it.

use std::path::PathBuf;

use enth_session::create::{
    CreateEdit, CreateSettings, CreateSource, CreateView, Which,
};

use super::HeaderCreate;

/// What the Create view asks for. The editor queues them while drawing and applies them after the frame
/// ([`Create::apply`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CreateIntent {
    /// Show or hide the Create panel (the header's **Create**).
    Toggle,
    /// Choose this file (a drop on the editor, or the typed path); it opens the panel.
    Choose(PathBuf),
    /// Start a job on the chosen file.
    Start,
    /// Cancel the running job.
    Cancel,
    /// Play a candidate's patch once.
    Audition(usize, Which),
    /// Stop an audition.
    StopAudition,
    /// "Use this": load the candidate's patch (one undo step).
    Use(usize, Which),
    /// Close the job: a running one is cancelled.
    Clear,
}

/// The editor's Create state (see the [module docs](self)).
#[derive(Debug, Default)]
pub struct Create {
    /// The shell hands the editor a create read model (so the header shows **Create**).
    available: bool,
    open: bool,
    view: Option<CreateView>,
    /// The file chosen (not started yet, or the last job's).
    file: Option<PathBuf>,
    /// The typed path.
    pub(super) path_text: String,
    settings: CreateSettings,
    queued: Vec<CreateIntent>,
    /// The last refusal, for the panel.
    said: Option<String>,
}

impl Create {
    /// The session's create job, before each frame. A shell that calls it says create is available; a job that
    /// appears opens the panel.
    pub fn set_create(&mut self, view: Option<CreateView>) {
        self.available = true;
        if view.is_some() && self.view.is_none() {
            self.open = true;
        }
        self.view = view;
    }

    /// What the header's Create hook shows.
    pub fn header(&self) -> HeaderCreate {
        HeaderCreate {
            available: self.available,
        }
    }

    /// Whether the shell says create is available (a dropped file is taken then).
    pub fn available(&self) -> bool {
        self.available
    }

    /// Whether the panel is open.
    pub fn panel_open(&self) -> bool {
        self.available && self.open
    }

    /// The job's read model, if there is one.
    pub fn view(&self) -> Option<&CreateView> {
        self.view.as_ref()
    }

    /// The chosen file.
    pub fn file(&self) -> Option<&PathBuf> {
        self.file.as_ref()
    }

    /// The settings a Start uses.
    pub fn settings(&self) -> &CreateSettings {
        &self.settings
    }

    /// The last refusal.
    pub fn said(&self) -> Option<&str> {
        self.said.as_deref()
    }

    /// Queue an intent for [`Create::apply`]; the panel's own (open, close, choose) take effect at once, so the next
    /// pass draws them.
    pub fn push(&mut self, intent: CreateIntent) {
        match &intent {
            CreateIntent::Toggle => self.open = !self.open,
            CreateIntent::Choose(path) => {
                self.file = Some(path.clone());
                self.path_text = path.to_string_lossy().into_owned();
                self.open = true;
                self.said = None;
            }
            _ => {}
        }
        self.queued.push(intent);
    }

    /// Whether intents wait for the session.
    pub fn has_queued(&self) -> bool {
        !self.queued.is_empty()
    }

    /// After the frame: the queued intents, applied to the session's create edit trait in order. A refusal is shown
    /// in the panel.
    pub fn apply(&mut self, session: &mut dyn CreateEdit) {
        for intent in std::mem::take(&mut self.queued) {
            let result = match intent {
                CreateIntent::Toggle | CreateIntent::Choose(_) => Ok(()),
                CreateIntent::Start => match &self.file {
                    Some(file) => session
                        .create_start(CreateSource::file(file.clone()), self.settings.clone())
                        .map(|_| ()),
                    None => Err(enth_session::create::CreateError::NoFile),
                },
                CreateIntent::Cancel => session.create_cancel(),
                CreateIntent::Audition(branch, which) => session.create_audition(branch, which),
                CreateIntent::StopAudition => {
                    session.create_stop_audition();
                    Ok(())
                }
                CreateIntent::Use(branch, which) => session.create_use(branch, which),
                CreateIntent::Clear => {
                    session.create_clear();
                    Ok(())
                }
            };
            self.said = result.err().map(|e| format!("{e}: {}", e.hint()));
        }
        self.view = session.create();
    }
}
