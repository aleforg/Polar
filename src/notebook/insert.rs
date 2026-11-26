use gpui_kit::{Context, PathPromptOptions, Window};

use super::Notebook;
use crate::{
    attachments,
    blocks::{CalloutKind, TABLE},
};

impl Notebook {
    pub(super) fn insert_table(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.insert_block(TABLE, window, cx);
    }

    pub(super) fn insert_callout(
        &mut self,
        kind: CalloutKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.insert_block(
            &format!("> [!{}]\n> Scrivi qui.", kind.keyword()),
            window,
            cx,
        );
    }

    pub(super) fn insert_block(
        &mut self,
        markdown: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_editing(window, cx);
        if self.editing {
            self.finish_format_action();
            self.source.insert_block(markdown);
            self.sync_editor(cx);
            cx.notify();
        }
    }

    pub(super) fn insert_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.start_editing(window, cx);
        if self.editing {
            self.finish_format_action();
            self.source.insert_link();
            self.sync_editor(cx);
            cx.notify();
        }
    }

    pub(super) fn choose_image(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(note_id) = self
            .selected()
            .filter(|note| note.can_edit())
            .map(|note| note.id)
        else {
            return;
        };
        let Some(store) = &self.store else {
            return;
        };
        let directory = store.attachments();
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Inserisci immagine".into()),
        });
        self.finish_format_action();
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let selected = match picker.await {
                Ok(Ok(Some(paths))) => paths.into_iter().next(),
                Ok(Ok(None)) => return,
                Ok(Err(error)) => {
                    let _ = this.update(cx, |this, cx| {
                        this.status = Some(format!("Impossibile aprire l’immagine. {error}"));
                        cx.notify();
                    });
                    return;
                }
                Err(_) => return,
            };
            let Some(source) = selected else {
                return;
            };
            let result = cx
                .background_executor()
                .spawn(async move { attachments::import(&directory, &source) })
                .await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(markdown) => {
                    if this.selected == Some(note_id) {
                        this.insert_block(&markdown, window, cx);
                    } else if let Some(note) = this
                        .notes
                        .iter_mut()
                        .find(|note| note.id == note_id && note.can_edit())
                    {
                        note.append_attachment(&markdown);
                        note.created.get_or_insert(note.modified);
                        note.modified = crate::notes::now();
                        this.changed(cx);
                    }
                }
                Err(error) => {
                    this.status = Some(format!("Immagine non inserita. {error}"));
                    cx.notify();
                }
            });
        })
        .detach();
    }
}
