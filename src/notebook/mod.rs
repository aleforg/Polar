pub(crate) mod actions;
mod document;
mod format_bar;
mod info;
mod insert;
mod navigation;
mod revision;
mod tag_picker;
mod widgets;

use std::{collections::BTreeSet, path::PathBuf, rc::Rc, sync::Arc, time::Duration};

use gpui_kit::{
    AppContext, Context, Entity, FocusHandle, InteractiveElement, IntoElement, ParentElement,
    Render, Styled, Subscription, Task, UniformListScrollHandle, Window,
    component::input::{InputEvent, InputState},
    div,
    prelude::FluentBuilder as _,
    rgb,
};

use self::{actions::*, format_bar::FormatGroup, info::InfoTab, tag_picker::TagPicker};
use crate::{
    editor::{EditorEvent, EditorMode, RichEditor, markdown::DocumentLink},
    notes::{self, Filter, Note, tag_icons::TagIcons},
    palette::{EDITOR, TEXT},
    store::Store,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Panel {
    None,
    Format,
    FormatOptions(FormatGroup),
    Info,
    Actions,
}

impl Panel {
    fn is_format(self) -> bool {
        matches!(self, Self::Format | Self::FormatOptions(_))
    }
}

pub(crate) struct Notebook {
    store: Option<Store>,
    notes: Vec<Note>,
    tag_icons: TagIcons,
    tag_picker: Option<TagPicker>,
    tag_icon_search: Entity<InputState>,
    selected: Option<u64>,
    filter: Filter,
    query: String,
    search_open: bool,
    search: Entity<InputState>,
    source: Rc<RichEditor>,
    editing: bool,
    panes: u8,
    panel: Panel,
    info_tab: InfoTab,
    loaded_note: Option<u64>,
    expanded_tags: BTreeSet<String>,
    indexing_tags: bool,
    expanded_notes: bool,
    workspace_focus: FocusHandle,
    note_list_scroll: UniformListScrollHandle,
    dirty: bool,
    saving: Option<Task<()>>,
    startup_error: Option<String>,
    status: Option<String>,
    revision: revision::Revision,
    _subscriptions: [Subscription; 2],
}

impl Notebook {
    pub(crate) fn new(directory: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (store, notes, tag_icons, startup_error) = match Store::open(&directory) {
            Ok((store, archive)) => (Some(store), archive.notes, archive.tag_icons, None),
            Err(error) => (
                None,
                vec![],
                TagIcons::new(),
                Some(format!("Impossibile aprire le note. {error}")),
            ),
        };
        let selected = notes::visible_indices(&notes, &Filter::Notes, "")
            .first()
            .map(|&index| notes[index].id);
        let indexing_tags = Self::index_tags(&notes, cx);
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Ricerca"));
        let tag_icon_search = cx.new(|cx| InputState::new(window, cx).placeholder("Ricerca"));
        let weak_notebook = cx.weak_entity();
        let async_app = cx.to_async();
        let handle = Window::window_handle(window);
        let source = RichEditor::new(directory.join("attachments"), move |event| {
            let weak = weak_notebook.clone();
            let app = async_app.clone();
            async_app
                .foreground_executor()
                .spawn(async move {
                    app.update(|cx| {
                        let _ = handle.update(cx, |_, window, cx| {
                            let _ = weak.update(cx, |this, cx| match event {
                                EditorEvent::Changed => this.sync_editor(cx),
                                EditorEvent::WikiCompletion => {
                                    this.source.show_wiki_suggestions(&this.notes)
                                }
                                EditorEvent::Follow(DocumentLink::Web(url)) => cx.open_url(&url),
                                EditorEvent::Follow(DocumentLink::Wiki(title)) => {
                                    if let Some(id) = notes::links::target(&this.notes, &title) {
                                        this.follow_note(id, window, cx);
                                    }
                                }
                                EditorEvent::Follow(DocumentLink::Tag(tag)) => {
                                    this.panes = this.panes.max(2);
                                    this.set_filter(Filter::Tag(tag), window, cx);
                                }
                            });
                        });
                    });
                })
                .detach();
        });
        source.set_tag_icons(&tag_icons);
        let workspace_focus = cx.focus_handle();
        workspace_focus.focus(window, cx);
        let subscriptions = [
            cx.subscribe(&tag_icon_search, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.subscribe_in(&search, window, |this, input, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    let query = input.read(cx).value().trim().to_owned();
                    if query != this.query {
                        this.sync_editor(cx);
                        this.query = query;
                        this.reselect();
                        cx.notify();
                    }
                }
            }),
        ];
        let weak = cx.weak_entity();
        window.on_window_should_close(cx, move |_, cx| {
            weak.update(cx, |this, cx| {
                if this.flush(cx) {
                    this.revision.cancel();
                    true
                } else {
                    false
                }
            })
            .unwrap_or(true)
        });
        let editing = notes
            .iter()
            .any(|note| Some(note.id) == selected && note.can_edit());
        Self {
            store,
            notes,
            tag_icons,
            tag_picker: None,
            tag_icon_search,
            selected,
            filter: Filter::Notes,
            query: String::new(),
            search_open: false,
            search,
            source,
            editing,
            panes: 3,
            panel: Panel::None,
            info_tab: InfoTab::Stats,
            loaded_note: None,
            expanded_tags: BTreeSet::new(),
            indexing_tags,
            expanded_notes: true,
            workspace_focus,
            note_list_scroll: UniformListScrollHandle::new(),
            dirty: false,
            saving: None,
            startup_error,
            status: None,
            revision: revision::Revision::new(cx),
            _subscriptions: subscriptions,
        }
    }

    fn index_tags(notes: &[Note], cx: &mut Context<Self>) -> bool {
        let candidates: Vec<_> = notes
            .iter()
            .enumerate()
            .filter(|(_, note)| {
                !note.trashed && !note.archived && notes::tags::may_have_tags(note.text())
            })
            .collect();
        let bytes = candidates
            .iter()
            .map(|(_, note)| note.text().len())
            .sum::<usize>();
        if candidates.len() < 64 && bytes < 65536 {
            return false;
        }
        // Bound simultaneous parses and keep large individual documents on one worker.
        let workers = if candidates.len() >= 256
            && bytes >= 262_144
            && candidates
                .iter()
                .all(|(_, note)| note.text().len() <= 262_144)
        {
            std::thread::available_parallelism().map_or(1, |count| count.get().min(4))
        } else {
            1
        };
        let mut indexed: Vec<_> = candidates
            .into_iter()
            .map(|(index, note)| (index, note.text_snapshot(), BTreeSet::new()))
            .collect();
        let executor = cx.background_executor().clone();
        let worker = executor.clone().spawn(async move {
            let _timing = crate::performance::Sample::start("tag_index");
            let chunk_size = indexed.len().div_ceil(workers);
            executor
                .scoped(|scope| {
                    for chunk in indexed.chunks_mut(chunk_size) {
                        scope.spawn(async move {
                            for (_, source, tags) in chunk {
                                *tags = notes::tags(source);
                            }
                        });
                    }
                })
                .await;
            indexed
        });
        cx.spawn(async move |this, cx| {
            let indexed = worker.await;
            let _ = this.update(cx, |this, cx| {
                for (index, source, tags) in indexed {
                    if let Some(note) = this.notes.get(index)
                        && Arc::ptr_eq(&note.text_snapshot(), &source)
                        && !note.tags_cached()
                    {
                        note.prime_tags(tags);
                    }
                }
                this.indexing_tags = false;
                cx.notify();
            });
        })
        .detach();
        true
    }

    fn selected(&self) -> Option<&Note> {
        self.notes
            .iter()
            .find(|note| Some(note.id) == self.selected)
    }

    fn selected_mut(&mut self) -> Option<&mut Note> {
        self.notes
            .iter_mut()
            .find(|note| Some(note.id) == self.selected)
    }

    pub(crate) fn flush(&mut self, cx: &mut Context<Self>) -> bool {
        self.sync_editor(cx);
        if !self.dirty {
            return true;
        }
        let Some(store) = &self.store else {
            return false;
        };
        match store.save(&self.notes, &self.tag_icons) {
            Ok(()) => {
                self.dirty = false;
                self.status = None;
            }
            Err(error) => {
                self.status = Some(format!("Salvataggio non riuscito. {error}"));
            }
        }
        cx.notify();
        !self.dirty
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        self.dirty = true;
        let timer = cx.background_executor().timer(Duration::from_millis(250));
        self.saving = Some(cx.spawn(async move |this, cx| {
            timer.await;
            let _ = this.update(cx, |this, cx| this.flush(cx));
        }));
        cx.notify();
    }

    fn update_text(&mut self, text: String, cx: &mut Context<Self>) {
        if let Some(note) = self
            .notes
            .iter_mut()
            .find(|note| Some(note.id) == self.selected)
            .filter(|note| note.can_edit() && note.text() != text)
        {
            let tags = self.source.cached_tags(&text);
            note.created.get_or_insert(note.modified);
            note.set_text(text);
            if let Some(tags) = tags {
                note.prime_tags(tags);
            }
            note.modified = notes::now();
            self.changed(cx);
        }
    }

    fn reselect(&mut self) {
        let results = notes::visible_indices(&self.notes, &self.filter, &self.query);
        if !results
            .iter()
            .any(|&index| Some(self.notes[index].id) == self.selected)
        {
            self.select_note(results.first().map(|&index| self.notes[index].id));
        }
        self.panel = Panel::None;
    }

    fn select_note(&mut self, selected: Option<u64>) {
        if self.selected != selected {
            self.revision.cancel();
            self.source.blur();
            self.selected = selected;
            self.editing = self.selected().is_some_and(Note::can_edit);
        }
    }

    fn follow_note(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_editor(cx);
        let Some(note) = self
            .notes
            .iter()
            .find(|note| note.id == id && !note.trashed)
        else {
            return;
        };
        self.filter = if note.archived {
            Filter::Archive
        } else {
            Filter::Notes
        };
        self.clear_search(window, cx);
        self.select_note(Some(id));
        self.panel = Panel::None;
        self.workspace_focus.focus(window, cx);
        cx.notify();
    }

    fn set_filter(&mut self, filter: Filter, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_editor(cx);
        self.filter = filter;
        self.clear_search(window, cx);
        self.reselect();
        cx.notify();
    }

    fn clear_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.query.clear();
        self.search
            .update(cx, |input, cx| input.set_value("", window, cx));
    }

    fn new_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.add_note(String::new(), window, cx);
    }

    fn duplicate_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_editor(cx);
        if let Some(note) = self.selected().filter(|note| !note.trashed) {
            self.add_note(note.text().to_owned(), window, cx);
        }
    }

    fn add_note(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_editor(cx);
        if self.store.is_none() {
            return;
        }
        self.clear_search(window, cx);
        self.filter = Filter::Notes;
        let last_id = self.notes.iter().map(|note| note.id).max().unwrap_or(0);
        let Some(id) = last_id.checked_add(1).filter(|id| *id < u64::MAX) else {
            self.status = Some("Impossibile assegnare un identificativo alla nota".into());
            cx.notify();
            return;
        };
        let now = notes::now();
        let mut note = Note::new(id, text, now);
        note.created = Some(now);
        self.notes.push(note);
        self.select_note(Some(id));
        self.panel = Panel::None;
        self.start_editing(window, cx);
        self.changed(cx);
    }

    fn sync_editor(&mut self, cx: &mut Context<Self>) {
        if self.invalidate_revision() {
            cx.notify();
        }
        if self.loaded_note.is_some() && self.loaded_note == self.selected {
            self.update_text(self.source.value(), cx);
        }
    }

    fn prepare_source(&mut self) {
        if self.loaded_note != self.selected {
            self.loaded_note = self.selected;
            if let Some(note) = self.selected() {
                self.source.load(note.text());
            }
        }
        if let Some(note) = self.selected() {
            self.source.set_mode(if !note.can_edit() {
                EditorMode::Protected
            } else if self.editing {
                EditorMode::Editing
            } else {
                EditorMode::Reading
            });
        } else {
            self.source.hide();
        }
        self.invalidate_revision();
    }

    fn start_editing(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.selected().is_some_and(Note::can_edit) {
            self.editing = true;
            self.prepare_source();
            self.source.focus();
            cx.notify();
        }
    }

    fn stop_editing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_editor(cx);
        self.editing = false;
        self.prepare_source();
        self.workspace_focus.focus(window, cx);
        self.reselect();
        self.flush(cx);
        cx.notify();
    }

    fn find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.source.blur();
        self.search_open = true;
        self.panes = self.panes.max(2);
        self.search.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn close_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_editor(cx);
        self.search_open = false;
        self.clear_search(window, cx);
        self.reselect();
        self.workspace_focus.focus(window, cx);
        cx.notify();
    }

    fn toggle_read_only(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_editor(cx);
        if let Some(note) = self
            .selected_mut()
            .filter(|note| !note.trashed && !note.archived)
        {
            note.read_only = !note.read_only;
            self.stop_editing(window, cx);
            self.changed(cx);
        }
    }

    fn pin(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_editor(cx);
        if let Some(note) = self.selected_mut().filter(|note| !note.trashed) {
            note.pinned = !note.pinned;
            let was_editing = self.editing;
            self.reselect();
            if was_editing && !self.editing {
                self.workspace_focus.focus(window, cx);
            }
            self.changed(cx);
        }
    }

    fn toggle_archive(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_editor(cx);
        if let Some(note) = self.selected_mut().filter(|note| !note.trashed) {
            note.archived = !note.archived;
            self.reselect();
            if !self.editing {
                self.workspace_focus.focus(window, cx);
            }
            self.changed(cx);
        }
    }

    fn trash_or_restore(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_editor(cx);
        if let Some(note) = self.selected_mut() {
            note.trashed = !note.trashed;
            let was_editing = self.editing;
            self.reselect();
            if was_editing && !self.editing {
                self.workspace_focus.focus(window, cx);
            }
            self.changed(cx);
        }
    }

    fn cycle_panes(&mut self, cx: &mut Context<Self>) {
        self.panes = match self.panes {
            3 => 2,
            2 => 1,
            _ => 3,
        };
        self.panel = Panel::None;
        cx.notify();
    }

    fn toggle_panel(&mut self, panel: Panel, cx: &mut Context<Self>) {
        self.panel = if self.panel == panel || (panel == Panel::Format && self.panel.is_format()) {
            Panel::None
        } else {
            panel
        };
        cx.notify();
    }

    fn format(
        &mut self,
        prefix: &'static str,
        suffix: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_editing(window, cx);
        if !self.editing {
            return;
        }
        self.finish_format_action();
        if suffix.is_empty() {
            self.source.prefix_line(prefix);
        } else {
            self.source.format(prefix, suffix);
        }
        self.sync_editor(cx);
        cx.notify();
    }

    fn finish_format_action(&mut self) {
        self.panel = if self.panel.is_format() {
            Panel::Format
        } else {
            Panel::None
        };
    }

    fn export(&mut self, cx: &mut Context<Self>) {
        let Some(note) = self.selected() else { return };
        let name = note.title().replace(['/', ':'], "-") + ".md";
        let text = note.text_snapshot();
        let attachments = self.store.as_ref().map(Store::attachments);
        let folder = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let path = cx.prompt_for_new_path(&folder, Some(&name));
        self.panel = Panel::None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = match path.await {
                Ok(Ok(Some(path))) => {
                    cx.background_executor()
                        .spawn(async move {
                            if let Some(directory) = attachments {
                                crate::attachments::export_references(&directory, &path, &text)?;
                            }
                            std::fs::write(path, text.as_bytes())
                        })
                        .await
                }
                Ok(Ok(None)) => return,
                Ok(Err(error)) => Err(std::io::Error::other(error)),
                Err(error) => Err(std::io::Error::other(error)),
            };
            let _ = this.update(cx, |this, cx| {
                this.status = Some(match result {
                    Ok(()) => "Nota esportata".into(),
                    Err(error) => format!("Esportazione non riuscita. {error}"),
                });
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for Notebook {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _timing = crate::performance::Sample::start("workspace");
        self.prepare_source();
        div()
            .id("workspace")
            .track_focus(&self.workspace_focus)
            .key_context("Notebook")
            .size_full()
            .flex()
            .bg(rgb(EDITOR))
            .text_color(rgb(TEXT))
            .font_family(".SystemUIFont")
            .on_action(cx.listener(|this, _: &NewNote, window, cx| this.new_note(window, cx)))
            .on_action(cx.listener(|this, _: &Find, window, cx| this.find(window, cx)))
            .on_action(cx.listener(|this, _: &PreviousNote, window, cx| {
                this.step_note(-1, window, cx);
            }))
            .on_action(cx.listener(|this, _: &NextNote, window, cx| {
                this.step_note(1, window, cx);
            }))
            .on_action(cx.listener(|this, _: &EditNote, window, cx| {
                if this.editing {
                    this.stop_editing(window, cx);
                } else {
                    this.start_editing(window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &Escape, window, cx| {
                if this.dismiss_revision(window, cx) {
                    return;
                }
                if this.tag_picker.take().is_some() {
                    this.workspace_focus.focus(window, cx);
                    cx.notify();
                    return;
                }
                if this.source.dismiss_wiki_completion() {
                    return;
                }
                if this.panel != Panel::None {
                    this.panel = Panel::None;
                } else if this.search_open {
                    this.close_search(window, cx);
                } else if this.editing {
                    this.stop_editing(window, cx);
                }
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &CorrectSelection, window, cx| {
                this.start_revision(polar_note_revision::Action::Correct, window, cx);
            }))
            .on_action(cx.listener(|this, _: &OrganizeNote, window, cx| {
                this.start_revision(polar_note_revision::Action::Organize, window, cx);
            }))
            .on_action(cx.listener(|this, _: &UndoNote, _, _| this.source.undo_or_redo(false)))
            .on_action(cx.listener(|this, _: &RedoNote, _, _| this.source.undo_or_redo(true)))
            .on_action(
                cx.listener(|this, _: &Bold, window, cx| this.format("**", "**", window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &Italic, window, cx| this.format("*", "*", window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &Underline, window, cx| {
                    this.format("<u>", "</u>", window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &Pin, window, cx| this.pin(window, cx)))
            .on_action(cx.listener(|this, _: &Trash, window, cx| this.trash_or_restore(window, cx)))
            .on_action(cx.listener(|this, _: &CyclePanes, _, cx| this.cycle_panes(cx)))
            .on_action(cx.listener(|this, _: &Export, _, cx| this.export(cx)))
            .on_action(cx.listener(|this, _: &Close, window, cx| {
                if this.flush(cx) {
                    this.revision.cancel();
                    window.remove_window();
                }
            }))
            .when(self.panes == 3, |view| view.child(self.sidebar(cx)))
            .when(self.panes >= 2, |view| view.child(self.note_list(cx)))
            .child(self.editor(window, cx))
            .children(self.tag_picker(cx))
    }
}

#[cfg(debug_assertions)]
pub(crate) fn run_smoke_test(
    window_handle: gpui_kit::AnyWindowHandle,
    notebook: Entity<Notebook>,
    cx: &mut gpui_kit::App,
) {
    window_handle
        .update(cx, |_, window, cx| {
            notebook.update(cx, |this, cx| this.new_note(window, cx));
            cx.spawn(async move |cx| {
                let draw = |cx: &mut gpui_kit::App| {
                    window_handle
                        .update(cx, |_, window, cx| {
                            window.draw(cx).clear(cx);
                        })
                        .expect("smoke-test frame");
                };
                cx.background_executor()
                    .timer(Duration::from_millis(450))
                    .await;
                cx.update(draw);
                let note_scroll = cx.update(|cx| {
                    let this = notebook.read(cx);
                    (this.notes.len() > 30).then(|| this.note_list_scroll.clone())
                });
                if let Some(scroll) = note_scroll {
                    assert!(scroll.is_scrollable(), "Large note lists must support scrolling");
                    scroll.scroll_to_bottom();
                    cx.update(draw);
                    assert_eq!(scroll.is_scrolled_to_end(), Some(true), "Scroll to the last virtual row");
                    scroll.scroll_to_item_strict(0, gpui_kit::ScrollStrategy::Top);
                    cx.update(draw);
                }
                let editor_frame = cx.update(|cx| {
                    notebook.update(cx, |this, cx| {
                        let frame = this.source.viewport_frame();
                        this.toggle_panel(Panel::Format, cx);
                        frame
                    })
                });
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
                cx.update(draw);
                cx.update(|cx| {
                    notebook.update(cx, |this, cx| {
                        assert!(
                            this.editing && this.source.is_visible(),
                            "The format menu must keep the native editor visible"
                        );
                        assert_eq!(this.source.viewport_frame(), editor_frame);
                        assert_eq!(this.panel, Panel::Format);
                        assert!(
                            this.source.has_format_overlay(),
                            "Format mask at {editor_frame:?}"
                        );
                        this.source.assert_format_overlay(1);
                        this.toggle_panel(Panel::FormatOptions(FormatGroup::Heading), cx);
                    });
                });
                cx.update(draw);
                cx.update(|cx| {
                    window_handle
                        .update(cx, |_, window, cx| {
                            notebook.update(cx, |this, cx| {
                                assert_eq!(this.source.viewport_frame(), editor_frame);
                                this.source.assert_format_overlay(2);
                                this.format("**", "**", window, cx);
                                assert_eq!(this.panel, Panel::Format);
                                assert!(this.editing && this.source.is_focused());
                                this.toggle_panel(Panel::Info, cx);
                            });
                        })
                        .expect("formatting window");
                });
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
                cx.update(draw);
                cx.update(|cx| {
                    notebook.update(cx, |this, cx| {
                        assert!(this.editing && this.source.is_visible());
                        assert_eq!(this.source.viewport_frame(), editor_frame);
                        assert!(
                            this.source.has_format_overlay(),
                            "Info mask at {editor_frame:?}"
                        );
                        this.source.assert_format_overlay(1);
                        this.toggle_panel(Panel::Info, cx);
                    });
                });
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
                cx.update(draw);
                cx.update(|cx| {
                    notebook.update(cx, |this, cx| {
                        assert!(!this.source.has_format_overlay());
                        assert_eq!(this.source.viewport_frame(), editor_frame);
                        this.source.smoke_test();
                        assert!(this.flush(cx));
                    });
                    window_handle
                        .update(cx, |_, window, cx| {
                            notebook.update(cx, |this, cx| {
                                this.stop_editing(window, cx);
                                assert!(!this.editing);
                                assert!(this.selected().is_some_and(|note| {
                                    note.text().starts_with("Appunti di viaggio")
                                }));
                            });
                        })
                        .expect("reading-mode window");
                    window_handle
                        .update(cx, |_, window, cx| {
                            notebook.update(cx, |this, cx| {
                                this.new_note(window, cx);
                                this.source.load(&format!(
                                    "Indice 🐻\n\n## Ripetuto\n\n{}\n### Ripetuto\n\n{}",
                                    "Un paragrafo.\n\n".repeat(28),
                                    "Dopo l’indice.\n\n".repeat(24)
                                ));
                                this.stop_editing(window, cx);
                                this.toggle_panel(Panel::Info, cx);
                                this.info_tab = InfoTab::Outline;
                                cx.notify();
                            });
                        })
                        .expect("outline test window");
                });
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
                cx.update(draw);
                cx.update(|cx| {
                    notebook.update(cx, |this, cx| {
                        let heading = &info::outline(this.selected().unwrap().text())[2];
                        this.jump_to_heading(heading.offset, cx);
                    });
                });
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
                cx.update(draw);
                let (origin, frame, content) = cx.update(|cx| {
                    window_handle
                        .update(cx, |_, window, cx| {
                            notebook.update(cx, |this, cx| {
                                assert!(!this.editing && this.panel == Panel::None);
                                assert!(this.source.scroll_origin().y > 300.);
                                assert!(this.source.is_visible() && !this.source.is_editable());
                                let snapshot = (
                                    this.source.scroll_origin(),
                                    this.source.viewport_frame(),
                                    this.source.value(),
                                );
                                this.start_editing(window, cx);
                                snapshot
                            })
                        })
                        .expect("mode continuity test window")
                });
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
                cx.update(draw);
                cx.update(|cx| {
                    window_handle
                        .update(cx, |_, window, cx| {
                            notebook.update(cx, |this, cx| {
                                assert!(this.source.is_editable() && this.source.is_visible());
                                assert_eq!(this.source.viewport_frame(), frame);
                                assert_eq!(this.source.scroll_origin(), origin);
                                assert_eq!(this.source.value(), content);
                                this.stop_editing(window, cx);
                                assert!(!this.source.is_editable());
                                assert_eq!(this.source.scroll_origin(), origin);
                            });
                        })
                        .expect("mode continuity test window");
                    println!("Rich editor, overlay, and mode continuity smoke tests passed");
                });
                let run_live = std::env::var_os("POLAR_SMOKE_REVISION").is_some();
                cx.update(|cx| {
                    window_handle.update(cx, |_, window, cx| {
                        notebook.update(cx, |this, cx| this.smoke_revision_state(window, cx));
                    }).expect("revision smoke window");
                });
                if run_live {
                    for (original, expected, sections) in [
                        (
                         "Titolo 🐻\n\nHo incontratto Giulia. Il preventivo non è approvato.",
                         "Ho incontrato Giulia. Il preventivo non è approvato.", false),
                        (
                         concat!(
                             "Titolo 🐻\n\n",
                             "Preventivo: Luca manda i prezzi.\n\nSale: chiedere la capienza a Giulia.\n\nPreventivo: massimale 900 euro, IVA esclusa.\n\nSale: forse martedì, non confermato.",
                             "\n\n- Elenco\n  - annidato\n\n",
                             "Preventivo: Luca manda i prezzi.\n\nSale: chiedere la capienza a Giulia.\n\nPreventivo: massimale 900 euro, IVA esclusa.\n\nSale: forse martedì, non confermato.",
                         ),
                         "Sale: forse martedì, non confermato.", true),
                    ] {
                        let prior_edit = "\n\nAggiunta manuale. #verifica";
                        let before_apply = format!("{original}{prior_edit}");
                        cx.update(|cx| {
                            window_handle.update(cx, |_, window, cx| {
                                notebook.update(cx, |this, cx| {
                                    this.smoke_revision_fixture(original, window, cx);
                                    let body = original.find("\n\n").unwrap() + 2;
                                    this.source.select_revision_fixture(original.encode_utf16().count(), 0);
                                    this.source.format("", prior_edit);
                                    let selected_len = if sections { 0 } else { original[body..].encode_utf16().count() };
                                    this.source.select_revision_fixture(original[..body].encode_utf16().count(), selected_len);
                                    if sections {
                                        this.stop_editing(window, cx);
                                    }
                                    assert_eq!(this.source.value(), before_apply);
                                });
                                if sections {
                                    window.dispatch_action(Box::new(OrganizeNote), cx);
                                } else {
                                    window.dispatch_action(Box::new(CorrectSelection), cx);
                                }
                            }).expect("start revision smoke");
                        });
                        while cx.update(|cx| notebook.read(cx).revision.busy()) {
                            cx.background_executor().timer(Duration::from_millis(100)).await;
                        }
                        cx.update(draw);
                        let applied = cx.update(|cx| window_handle.update(cx, |_, window, cx| notebook.update(cx, |this, cx| {
                            let replacement = match &this.revision.pane {
                                revision::Pane::Preview { replacement, target, .. } => {
                                    assert_eq!(target.scope(), if sections { "Corpo della nota" } else { "Testo selezionato" });
                                    replacement
                                }
                                revision::Pane::Message(message) => panic!("revision smoke: {message}"),
                                _ => panic!("revision smoke expected a preview"),
                            };
                            assert!(replacement.contains(expected));
                            assert_eq!(sections, replacement.starts_with("## "));
                            assert_eq!(this.editing, !sections);
                            if sections {
                                assert!(replacement.contains("\n\n- Elenco\n  - annidato\n\n## "));
                                assert_eq!(replacement.matches(expected).count(), 2);
                            }
                            assert_eq!(this.source.value(), before_apply);
                            this.apply_revision(window, cx);
                            assert!(this.source.value().starts_with("Titolo 🐻\n\n"));
                            assert!(this.source.value().contains(expected));
                            assert!(this.source.value().ends_with(prior_edit));
                            assert!(this.flush(cx));
                            this.source.value()
                        })).expect("apply revision smoke"));
                        cx.background_executor().timer(Duration::from_millis(100)).await;
                        cx.update(draw);
                        for (redo, text) in [(false, before_apply.as_str()), (false, original), (true, before_apply.as_str()), (true, applied.as_str())] {
                            cx.update(|cx| {
                                window_handle.update(cx, |_, window, cx| {
                                    if redo {
                                        window.dispatch_action(Box::new(RedoNote), cx);
                                    } else {
                                        window.dispatch_action(Box::new(UndoNote), cx);
                                    }
                                }).expect("revision undo and redo");
                            });
                            cx.background_executor().timer(Duration::from_millis(80)).await;
                            cx.update(|cx| notebook.update(cx, |this, _| assert_eq!(this.source.value(), text)));
                        }
                        cx.update(|cx| {
                            notebook.update(cx, |this, cx| {
                                assert_eq!(this.source.value(), applied);
                                assert!(this.flush(cx));
                            });
                        });
                    }
                    println!("Local revision preview, apply, undo, redo, and save passed for both actions");
                }
                cx.update(|cx| cx.quit());
            })
            .detach();
        })
        .expect("smoke-test window");
}
