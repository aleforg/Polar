use std::{
    ops::Range,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use gpui_kit::{Context, FocusHandle, Window};
use polar_note_revision::{Action, Error, Notice, Response};

use super::{Notebook, Panel};
use crate::editor::{RevisionSnapshot, revision as markdown};

pub(super) struct Target {
    note: u64,
    snapshot: RevisionSnapshot,
    range: Range<usize>,
    action: Action,
    editing: bool,
    whole_note: bool,
}

struct Request {
    target: Target,
    passages: Vec<markdown::Selection>,
}

enum ProposalError {
    Transport(Error),
    Markdown,
    Notice,
}

pub(super) enum Pane {
    Hidden,
    Running(Target),
    Preview {
        target: Target,
        replacement: String,
        notices: Vec<Notice>,
        formatting: bool,
    },
    Message(&'static str),
}

pub(super) struct Revision {
    pub(super) pane: Pane,
    pub(super) show_original: bool,
    pending: Option<Arc<AtomicBool>>,
    pub(super) dismiss_focus: FocusHandle,
}

impl Revision {
    pub(super) fn new(cx: &mut Context<Notebook>) -> Self {
        Self {
            pane: Pane::Hidden,
            show_original: false,
            pending: None,
            dismiss_focus: cx.focus_handle(),
        }
    }

    pub(super) fn busy(&self) -> bool {
        self.pending.is_some()
    }

    pub(super) fn cancel(&mut self) -> bool {
        if let Some(cancelled) = &self.pending {
            cancelled.store(true, Ordering::Relaxed);
        }
        !matches!(
            std::mem::replace(&mut self.pane, Pane::Hidden),
            Pane::Hidden
        )
    }
}

impl Drop for Revision {
    fn drop(&mut self) {
        self.cancel();
    }
}

impl Target {
    pub(super) fn original(&self) -> &str {
        &self.snapshot.source[self.range.clone()]
    }

    pub(super) fn title(&self) -> &'static str {
        match self.action {
            Action::Correct => "Correggi selezione",
            Action::Organize => "Organizza appunti",
        }
    }

    pub(super) fn scope(&self) -> &'static str {
        if self.whole_note {
            "Corpo della nota"
        } else {
            "Testo selezionato"
        }
    }
}

fn propose(
    helper: &Path,
    action: Action,
    source: &str,
    scope: Range<usize>,
    passages: Vec<markdown::Selection>,
    cancelled: &AtomicBool,
) -> Result<Response, ProposalError> {
    let mut revised = source.to_owned();
    let mut all_notices = Vec::new();
    // Start at the end so the source offsets of the remaining passages stay valid.
    for passage in passages.into_iter().rev() {
        let original = &source[passage.range.clone()];
        let units = matches!(action, Action::Organize).then_some(passage.units.as_slice());
        let response = polar_note_revision::revise(helper, action, original, units, cancelled)
            .map_err(ProposalError::Transport)?;
        let Response::Ok {
            markdown: replacement,
            notices,
        } = response
        else {
            return Ok(response);
        };
        if replacement == original {
            continue;
        }
        if !markdown::validate(&revised, &passage.range, &replacement, action) {
            return Err(ProposalError::Markdown);
        }
        if !notices.iter().all(|notice| {
            !notice.source.is_empty()
                && original.match_indices(&notice.source).count() == 1
                && replacement.contains(&notice.source)
        }) {
            return Err(ProposalError::Notice);
        }
        revised.replace_range(passage.range, &replacement);
        all_notices.extend(notices.into_iter().rev());
    }
    all_notices.reverse();
    let end = revised.len() - (source.len() - scope.end);
    Ok(Response::Ok {
        markdown: revised[scope.start..end].to_owned(),
        notices: all_notices,
    })
}

fn helper_path() -> Result<PathBuf, &'static str> {
    let os = objc2_foundation::NSProcessInfo::processInfo().operatingSystemVersion();
    if !cfg!(target_arch = "aarch64") || (os.majorVersion, os.minorVersion) < (26, 4) {
        return Err("La revisione richiede un Mac con Apple silicon e macOS 26.4 o successivo.");
    }
    std::env::current_exe().ok()
        .and_then(|exe| exe.parent()?.parent().map(|contents| contents.join("Helpers/polar-note-revision")))
        .filter(|path| path.is_file())
        .ok_or("Il componente per la revisione manca. Apri il bundle Polar compilato con l’SDK macOS 27.")
}

impl Notebook {
    pub(super) fn start_revision(
        &mut self,
        action: Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.revision.busy() {
            return;
        }
        self.panel = Panel::None;
        let request = self
            .revision_target(action)
            .and_then(|request| Ok((helper_path()?, request)));
        let (helper, Request { target, passages }) = match request {
            Ok(request) => request,
            Err(message) => {
                self.revision.pane = Pane::Message(message);
                cx.notify();
                return;
            }
        };
        let source = target.snapshot.source.clone();
        let scope = target.range.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        self.revision.pending = Some(cancelled.clone());
        self.revision.show_original = false;
        self.revision.pane = Pane::Running(target);
        cx.notify();

        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(
                    async move { propose(&helper, action, &source, scope, passages, &cancelled) },
                )
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.finish_revision(result, window, cx)
            });
        })
        .detach();
    }

    fn revision_target(&self, action: Action) -> Result<Request, &'static str> {
        let note = self
            .selected()
            .filter(|note| self.loaded_note == Some(note.id) && note.can_edit())
            .ok_or("Apri una nota modificabile.")?;
        if matches!(action, Action::Correct) && !self.editing {
            return Err("Passa alla modalità di modifica e seleziona il testo da correggere.");
        }
        let snapshot = self
            .source
            .revision_snapshot()
            .ok_or("Seleziona un unico passaggio dopo aver terminato di scrivere.")?;
        let whole_note = matches!(action, Action::Organize) && snapshot.selection.is_empty();
        let passages = if whole_note {
            markdown::organization_passages(&snapshot.source)?
        } else {
            vec![markdown::selection(
                &snapshot.source,
                &snapshot.selection,
                action,
            )?]
        };
        let range = passages.first().unwrap().range.start..passages.last().unwrap().range.end;
        Ok(Request {
            target: Target {
                note: note.id,
                snapshot,
                range,
                action,
                editing: self.editing,
                whole_note,
            },
            passages,
        })
    }

    fn revision_current(&self, target: &Target) -> bool {
        self.editing == target.editing
            && self.loaded_note == Some(target.note)
            && self
                .selected()
                .is_some_and(|note| note.id == target.note && note.can_edit())
            && self.source.revision_is_current(&target.snapshot)
    }

    pub(super) fn invalidate_revision(&mut self) -> bool {
        let (Pane::Running(target) | Pane::Preview { target, .. }) = &self.revision.pane else {
            return false;
        };
        !self.revision_current(target) && self.revision.cancel()
    }

    pub(super) fn dismiss_revision(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.revision.cancel() {
            self.workspace_focus.focus(window, cx);
            self.source.focus();
            cx.notify();
            return true;
        }
        false
    }

    fn finish_revision(
        &mut self,
        result: Result<Response, ProposalError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.revision.pending = None;
        if let Pane::Running(target) = std::mem::replace(&mut self.revision.pane, Pane::Hidden)
            && self.revision_current(&target)
        {
            self.revision.pane = match result {
                Ok(Response::Ok {
                    markdown: replacement,
                    ..
                }) if replacement == target.original() => {
                    Pane::Message("La revisione non propone modifiche.")
                }
                Ok(Response::Ok {
                    markdown: replacement,
                    notices,
                }) => {
                    let formatting = matches!(target.action, Action::Correct)
                        && replacement.matches("**").count()
                            > target.original().matches("**").count();
                    self.source.blur();
                    self.revision.dismiss_focus.focus(window, cx);
                    Pane::Preview {
                        target,
                        replacement,
                        notices,
                        formatting,
                    }
                }
                Ok(Response::InvalidResult {}) => Pane::Message(
                    "Il modello non ha prodotto una proposta utilizzabile. La nota è rimasta intatta.",
                ),
                Err(ProposalError::Markdown) => Pane::Message(
                    "La proposta non conserva il testo o la struttura Markdown della nota. La nota è rimasta intatta.",
                ),
                Err(ProposalError::Notice) => Pane::Message(
                    "La proposta non conserva un passaggio segnalato. La nota è rimasta intatta.",
                ),
                Ok(Response::IntelligenceDisabled {}) => {
                    Pane::Message("Attiva Apple Intelligence in Impostazioni di Sistema e riprova.")
                }
                Ok(Response::ModelNotReady {}) => Pane::Message(
                    "Il modello non è pronto. Controlla Apple Intelligence in Impostazioni di Sistema e riprova.",
                ),
                Ok(Response::DeviceIneligible {} | Response::ModelUnavailable {}) => Pane::Message(
                    "Il modello locale di Apple Intelligence non è disponibile su questo Mac.",
                ),
                Ok(Response::InputTooLong {})
                | Err(ProposalError::Transport(Error::RequestTooLarge)) => Pane::Message(
                    "Il testo è troppo lungo. Seleziona un passaggio più breve e riprova.",
                ),
                Ok(Response::Refused {}) => {
                    Pane::Message("Il modello non ha potuto elaborare il passaggio.")
                }
                Err(ProposalError::Transport(Error::Timeout)) => {
                    Pane::Message("La revisione ha superato il tempo massimo. Riprova.")
                }
                Err(ProposalError::Transport(Error::Cancelled)) => Pane::Hidden,
                _ => Pane::Message("Impossibile completare la revisione locale. Riprova."),
            };
        }
        cx.notify();
    }

    pub(super) fn apply_revision(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Pane::Preview {
            target,
            replacement,
            ..
        } = std::mem::replace(&mut self.revision.pane, Pane::Hidden)
        {
            self.workspace_focus.focus(window, cx);
            if self.revision_current(&target) {
                if !self.editing {
                    self.start_editing(window, cx);
                }
                if self
                    .source
                    .apply_revision(&target.snapshot, target.range, &replacement)
                {
                    self.sync_editor(cx);
                    cx.notify();
                    return;
                }
            }
            self.revision.pane = Pane::Message("La nota è cambiata. Ripeti la revisione.");
        }
        cx.notify();
    }
}

#[cfg(debug_assertions)]
impl Notebook {
    pub(super) fn smoke_revision_fixture(
        &mut self,
        markdown: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.new_note(window, cx);
        self.source.load(markdown);
        self.sync_editor(cx);
        let body = markdown.find("\n\n").expect("fixture title") + 2;
        let start = markdown[..body].encode_utf16().count();
        self.source
            .select_revision_fixture(start, markdown[body..].encode_utf16().count());
    }

    pub(super) fn smoke_revision_state(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let source = "Titolo 🐻\n\nRiunone";
        self.smoke_revision_fixture(source, window, cx);
        let Request { target, .. } = self.revision_target(Action::Correct).unwrap();
        self.revision.pane = Pane::Running(target);
        self.source.load(source);
        self.finish_revision(
            Ok(Response::Ok {
                markdown: "Riunione".into(),
                notices: vec![],
            }),
            window,
            cx,
        );
        assert!(matches!(self.revision.pane, Pane::Hidden));
        assert_eq!(self.source.value(), source);

        self.source
            .select_revision_fixture("Titolo 🐻\n\n".encode_utf16().count(), 7);
        let Request { target, .. } = self.revision_target(Action::Correct).unwrap();
        self.revision.pane = Pane::Running(target);
        assert!(self.dismiss_revision(window, cx));
        self.finish_revision(
            Ok(Response::Ok {
                markdown: "Riunione".into(),
                notices: vec![],
            }),
            window,
            cx,
        );
        assert!(matches!(self.revision.pane, Pane::Hidden));
        assert_eq!(self.source.value(), source);

        for (result, too_long) in [
            (Ok(Response::InvalidResult {}), false),
            (Err(ProposalError::Markdown), false),
            (Err(ProposalError::Notice), false),
            (Err(ProposalError::Transport(Error::Timeout)), false),
            (Ok(Response::InputTooLong {}), true),
            (Err(ProposalError::Transport(Error::RequestTooLarge)), true),
        ] {
            let Request { target, .. } = self.revision_target(Action::Correct).unwrap();
            self.revision.pane = Pane::Running(target);
            self.finish_revision(result, window, cx);
            let Pane::Message(message) = self.revision.pane else {
                panic!("revision failure should explain what happened");
            };
            assert_eq!(message.contains("passaggio più breve"), too_long);
            assert_eq!(self.source.value(), source);
        }
        assert!(self.dismiss_revision(window, cx));
    }
}
