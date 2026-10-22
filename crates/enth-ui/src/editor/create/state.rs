//! **Owner: lane PUI** (the Create view, feature `create`). Pre-declared by lane S0s (R3); **lane SK3** wrote the
//! minimal seam for freeze point F2s, which lane PUI grew into the view's state.
//!
//! The editor's Create state and the seam to the shell (docs/r3.md, "The Create view and the session"; docs/editor.md,
//! "The Create view"): [`CreateIntent`] (what the view asks for), `Editor::set_create` (the session's
//! `CreateEdit::create()` read model, handed over before each frame, as `set_round` hands over a round) and
//! `Editor::apply_create` (the intents applied to the session's `CreateEdit` after the frame). It mirrors
//! `recreate/state.rs` and `learn/state.rs`.
//!
//! **The view never reads a file or a folder** (the editor does no I/O): a drop gives a path, and the shell's
//! `enth-create` thread decodes it; the folder browser asks for a listing ([`Create::take_browse`], read by the shell
//! after the frame) and the shell hands it back as data ([`Create::set_folder`]).
//!
//! What the state keeps between frames, besides the session's read model: the chosen file (a drop or the browser
//! **starts a job at once** when none runs), the selected branch (the compare area's; the best one until a tile is
//! clicked), whether the folder browser shows and its last listing, the typed path, the settings and the last
//! refusal.

use std::path::{Path, PathBuf};

use enth_session::create::{
    CreateEdit, CreateError, CreatePhase, CreateSettings, CreateSource, CreateView, Which,
};

use super::HeaderCreate;
use super::browser::{BrowseAt, Folder};

/// What the Create view asks for. The editor queues them while drawing and applies them after the frame
/// ([`Create::apply`]); the view's own (open, close, choose, select, browse) take effect at once, so the next pass
/// draws them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CreateIntent {
    /// Show or hide the Create panel (the header's **Create**, the panel's ×).
    Toggle,
    /// Choose this file (a drop on the editor, the browser, or the typed path); it opens the panel and starts a job
    /// when none runs.
    Choose(PathBuf),
    /// Start a job on the chosen file.
    Start,
    /// Cancel the running job.
    Cancel,
    /// Play a candidate's patch once.
    Audition(usize, Which),
    /// Play the upload once, level-matched to the candidate played last (lane PUI).
    AuditionUpload,
    /// The A/B against the upload (lane PUI): the selected candidate if the upload was played last (or nothing was),
    /// else the upload.
    Flip(usize),
    /// Stop an audition.
    StopAudition,
    /// "Use this": load the candidate's patch (one undo step).
    Use(usize, Which),
    /// Close the job: a running one is cancelled.
    Clear,
    /// Select a branch for the compare area (lane PUI).
    Select(usize),
    /// Show the folder browser at a folder (lane PUI); the shell lists it.
    Browse(BrowseAt),
    /// Hide the folder browser (lane PUI).
    CloseBrowser,
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
    /// The typed path (the folder browser's field).
    pub(super) path_text: String,
    settings: CreateSettings,
    queued: Vec<CreateIntent>,
    /// The last refusal, for the panel.
    said: Option<String>,
    /// The branch the compare area shows (`None`: the best).
    selected: Option<usize>,
    /// The folder browser shows.
    browsing: bool,
    /// The shell's last listing.
    folder: Option<Folder>,
    /// A listing the shell is asked for (after the frame).
    browse: Option<BrowseAt>,
    /// Jobs started from this editor (each Start after the first takes the next seed, so a run again on the same
    /// file gives new candidates).
    runs: u64,
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

    /// Whether a job runs in the shell.
    pub fn running(&self) -> bool {
        self.view.as_ref().is_some_and(|v| v.phase.running())
    }

    /// The branch the compare area shows: the one selected while the job has it, else the best.
    pub fn selected(&self) -> Option<usize> {
        let view = self.view.as_ref()?;
        self.selected
            .filter(|b| view.candidates.iter().any(|c| c.branch == *b))
            .or_else(|| view.best())
    }

    /// Whether the folder browser shows.
    pub fn browsing(&self) -> bool {
        self.browsing
    }

    /// The shell's last listing for the folder browser.
    pub fn folder(&self) -> Option<&Folder> {
        self.folder.as_ref()
    }

    /// After the frame (the shell): the folder to list, if the browser asked for one. The shell lists it (the plugin:
    /// one directory read on the GUI thread) and hands it back with [`Create::set_folder`].
    pub fn take_browse(&mut self) -> Option<BrowseAt> {
        self.browse.take()
    }

    /// The shell's listing of the folder the browser asked for.
    pub fn set_folder(&mut self, folder: Folder) {
        self.folder = Some(folder);
    }

    /// Queue an intent for [`Create::apply`]; the panel's own (open, close, choose, select, browse) take effect at
    /// once, so the next pass draws them.
    pub fn push(&mut self, intent: CreateIntent) {
        match &intent {
            CreateIntent::Toggle => self.open = !self.open,
            CreateIntent::Choose(path) => {
                self.file = Some(path.clone());
                self.path_text = path.to_string_lossy().into_owned();
                self.open = true;
                self.said = None;
                self.browsing = false;
                // A drop or a pick starts at once when nothing runs (the job reads the file; its intake is what
                // the drop zone shows).
                if !self.running() {
                    self.queued.push(intent);
                    self.queued.push(CreateIntent::Start);
                    return;
                }
            }
            CreateIntent::Select(branch) => self.selected = Some(*branch),
            CreateIntent::Browse(at) => {
                self.browsing = true;
                self.open = true;
                self.browse = Some(at.clone());
            }
            CreateIntent::CloseBrowser => self.browsing = false,
            _ => {}
        }
        self.queued.push(intent);
    }

    /// Whether intents wait for the session.
    pub fn has_queued(&self) -> bool {
        !self.queued.is_empty()
    }

    /// The folder the browser opens at: the listing's, else the chosen file's.
    pub(super) fn browse_from(&self) -> BrowseAt {
        self.folder
            .as_ref()
            .map(|f| f.path.clone())
            .or_else(|| {
                self.file
                    .as_deref()
                    .and_then(Path::parent)
                    .map(Path::to_path_buf)
            })
            .map_or(BrowseAt::Start, BrowseAt::Path)
    }

    /// After the frame: the queued intents, applied to the session's create edit trait in order. A refusal is shown
    /// in the panel.
    pub fn apply(&mut self, session: &mut dyn CreateEdit) {
        for intent in std::mem::take(&mut self.queued) {
            let result = match intent {
                CreateIntent::Toggle
                | CreateIntent::Choose(_)
                | CreateIntent::Select(_)
                | CreateIntent::Browse(_)
                | CreateIntent::CloseBrowser => continue,
                CreateIntent::Start => match &self.file {
                    Some(file) => {
                        let settings = CreateSettings {
                            seed: self.settings.seed + self.runs,
                            ..self.settings.clone()
                        };
                        let started = session
                            .create_start(CreateSource::file(file.clone()), settings)
                            .map(|_| ());
                        if started.is_ok() {
                            self.runs += 1;
                            self.selected = None;
                        }
                        started
                    }
                    None => Err(CreateError::NoFile),
                },
                CreateIntent::Cancel => session.create_cancel(),
                CreateIntent::Audition(branch, which) => session.create_audition(branch, which),
                CreateIntent::AuditionUpload => session.create_audition_upload(),
                CreateIntent::Flip(branch) => {
                    let view = session.create();
                    let upload_next = view.as_ref().is_some_and(|v| {
                        !v.upload && v.auditioning.is_some_and(|(b, _)| b == branch)
                    });
                    if upload_next {
                        session.create_audition_upload()
                    } else {
                        let which = view
                            .and_then(|v| v.auditioning)
                            .filter(|(b, _)| *b == branch)
                            .map_or(Which::Refined, |(_, w)| w);
                        session.create_audition(branch, which)
                    }
                }
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

    /// Whether the job is past its intake (the upload can be played).
    pub(super) fn has_intake(&self) -> bool {
        self.view.as_ref().is_some_and(|v| v.intake.is_some())
    }

    /// Whether the job is cancelling.
    pub(super) fn cancelling(&self) -> bool {
        self.view
            .as_ref()
            .is_some_and(|v| v.phase == CreatePhase::Cancelling)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use enth_patch::Patch;
    use enth_session::Session;
    use enth_session::create::CreateRequest;

    #[test]
    fn a_choice_starts_a_job_at_once_and_a_second_start_takes_the_next_seed() {
        let mut s = Session::new(Patch::init());
        let mut c = Create::default();
        c.set_create(None);
        c.push(CreateIntent::Choose("/sounds/kick.wav".into()));
        assert!(c.panel_open());
        c.apply(&mut s);
        let seeds: Vec<u64> = s
            .drain_create_requests()
            .filter_map(|r| match r {
                CreateRequest::Run(run) => Some(run.settings.seed),
                _ => None,
            })
            .collect();
        assert_eq!(seeds, [1]);
        assert!(c.running());
        // A choice while it runs only chooses.
        c.push(CreateIntent::Choose("/sounds/pad.wav".into()));
        c.apply(&mut s);
        assert_eq!(s.drain_create_requests().count(), 0);
        c.push(CreateIntent::Clear);
        c.apply(&mut s);
        c.push(CreateIntent::Start);
        c.apply(&mut s);
        let runs: Vec<(u64, String)> = s
            .drain_create_requests()
            .filter_map(|r| match r {
                CreateRequest::Run(run) => Some((run.settings.seed, run.source.name())),
                _ => None,
            })
            .collect();
        assert_eq!(runs, [(2, "pad.wav".to_owned())]);
    }

    #[test]
    fn browsing_asks_the_shell_once_and_a_choice_closes_the_browser() {
        let mut c = Create::default();
        c.set_create(None);
        c.push(CreateIntent::Browse(BrowseAt::Start));
        assert!(c.browsing());
        assert_eq!(c.take_browse(), Some(BrowseAt::Start));
        assert_eq!(c.take_browse(), None, "asked once");
        c.push(CreateIntent::Choose("/sounds/kick.wav".into()));
        assert!(!c.browsing());
        assert_eq!(
            c.browse_from(),
            BrowseAt::Path("/sounds".into()),
            "the next browse opens at the chosen file's folder"
        );
    }
}
