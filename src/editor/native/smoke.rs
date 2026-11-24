use objc2::DefinedClass;
use objc2_app_kit::{NSBitmapImageFileType, NSFont, NSFontAttributeName, NSTextInputClient};
use objc2_foundation::{NSRange, NSString};

use super::{EditorMode, RichEditor};

impl RichEditor {
    pub(crate) fn smoke_test(&self) {
        super::math::assert_document_cache();
        let fixture = "# Appunti di viaggio\n\n## Itinerario\nUn paragrafo con **grassetto** e *corsivo*.\n\n- [ ] Preparare lo zaino\n- [x] Controllare gli orari\n\n#viaggi/idee";
        let selection_fixture = "Titolo 🐻\n\nCorpo";
        self.load(selection_fixture);
        self.reveal(selection_fixture.find("Corpo").unwrap());
        let text = &self.delegate.ivars().text;
        assert!(text.window().unwrap().makeFirstResponder(Some(text)));
        assert!(text.drawsBackground() && self.scroll.contentView().drawsBackground());
        self.smoke_wiki_completion();
        self.load(selection_fixture);
        self.reveal(selection_fixture.find("Corpo").unwrap());
        let range = NSTextInputClient::selectedRange(&**text);
        assert_eq!(range.location, "Titolo 🐻\n\n".encode_utf16().count());
        text.setSelectedRange(NSRange::new(range.location, 5));
        self.format("**", "**");
        assert_eq!(self.value(), "Titolo 🐻\n\n**Corpo**");
        self.format("*", "*");
        assert_eq!(self.value(), "Titolo 🐻\n\n***Corpo***");
        self.format("*", "*");
        self.format("**", "**");
        assert_eq!(self.value(), selection_fixture);
        self.prefix_line("## ");
        assert_eq!(self.value(), "Titolo 🐻\n\n## Corpo");
        self.prefix_line("## ");
        assert_eq!(self.value(), selection_fixture);
        let font_at = |offset| {
            // SAFETY: The test owns the text view on the main thread.
            let storage = unsafe { text.textStorage() }.unwrap();
            // SAFETY: The requested offset belongs to the loaded fixture and no output range is needed.
            let font = unsafe {
                storage.attribute_atIndex_effectiveRange(
                    NSFontAttributeName,
                    offset,
                    std::ptr::null_mut(),
                )
            }
            .unwrap()
            .downcast::<NSFont>()
            .unwrap();
            font.pointSize()
        };
        for (source, insertion, expected_size) in [
            ("# Titolo", "x", 30.),
            ("## Titolo", "x", 23.),
            ("# Titolo", "\n\nx", 15.),
            ("# Titolo\n\nCorpo", "x", 15.),
            ("# Titolo\n\n**grassetto**", "x", 15.),
            ("# Titolo\n\n$x^2$", "x", 15.),
        ] {
            self.load(source);
            let offset = source.encode_utf16().count();
            text.setSelectedRange(NSRange::new(offset, 0));
            // SAFETY: The insertion range is at the end of this fixture's UTF-16 text.
            unsafe {
                text.insertText_replacementRange(
                    &NSString::from_str(insertion),
                    NSRange::new(offset, 0),
                );
            }
            assert_eq!(
                font_at(offset + insertion.encode_utf16().count() - 1),
                expected_size,
                "Inserted text must have its final size immediately: {source:?}"
            );
            assert_eq!(self.delegate.ivars().markdown.borrow().0, self.value());
        }
        let mut formula_fixture = String::from("# Formule diverse\n\n");
        for index in 0..180 {
            formula_fixture.push_str(&format!("Paragrafo $x_{{{index}}} + 1$.\n\n"));
        }
        self.load(&formula_fixture);
        let end = formula_fixture.encode_utf16().count();
        text.setSelectedRange(NSRange::new(end, 0));
        let renders = super::math::render_count();
        let started = std::time::Instant::now();
        for index in 0..35 {
            // SAFETY: Each insertion extends the isolated fixture at its current end.
            unsafe {
                text.insertText_replacementRange(
                    &NSString::from_str("a"),
                    NSRange::new(end + index, 0),
                );
            }
        }
        for index in (0..35).rev() {
            // SAFETY: Remove exactly one ASCII character that was inserted above.
            unsafe {
                text.insertText_replacementRange(
                    &NSString::from_str(""),
                    NSRange::new(end + index, 1),
                );
            }
        }
        assert_eq!(self.value(), formula_fixture);
        assert_eq!(
            super::math::render_count(),
            renders,
            "Typing and deletion must not rerender unchanged formulas"
        );
        eprintln!(
            "180 distinct formulas, 35 insertions + 35 deletions: {:?}",
            started.elapsed()
        );
        let typing_fixture = format!(
            "# Digitazione\n\n{}\n",
            "Testo con **grassetto** e $E=mc^2$.\n\n".repeat(40)
        );
        self.load(&typing_fixture);
        text.setSelectedRange(NSRange::new(typing_fixture.encode_utf16().count(), 0));
        let started = std::time::Instant::now();
        for _ in 0..20 {
            // SAFETY: Insert at the current valid UTF-16 selection in the isolated note.
            unsafe {
                text.insertText_replacementRange(
                    &NSString::from_str("a"),
                    NSTextInputClient::selectedRange(&**text),
                );
            }
            assert_eq!(
                self.delegate.ivars().markdown.borrow().0,
                self.value(),
                "The presentation must match the text before the input event returns"
            );
        }
        eprintln!(
            "Typing 20 characters, each with preview: {:?}",
            started.elapsed()
        );
        assert_eq!(self.value(), format!("{typing_fixture}{}", "a".repeat(20)));
        let separator_fixture = "Titolo 🐻\n\nPrima\n\n---\n\nDopo";
        self.load(separator_fixture);
        assert_eq!(self.delegate.ivars().block_views.borrow().len(), 1);
        let offset = separator_fixture
            .split("---")
            .next()
            .unwrap()
            .encode_utf16()
            .count();
        text.setSelectedRange(NSRange::new(offset + 1, 0));
        assert!(self.delegate.ivars().block_views.borrow().is_empty());
        text.setSelectedRange(NSRange::new(0, 0));
        assert_eq!(self.delegate.ivars().block_views.borrow().len(), 1);
        assert_eq!(self.value(), separator_fixture);
        self.load(fixture);
        let state = self.delegate.ivars();
        let text = &state.text;
        assert!(self.scroll.window().is_some());
        assert!(font_at(0) < 1.);
        assert_eq!(font_at(2), 30.);
        self.set_mode(EditorMode::Reading);
        assert!(font_at(0) < 1.);
        assert_eq!(self.value(), fixture);
        self.set_mode(EditorMode::Editing);
        assert!(text.window().unwrap().makeFirstResponder(Some(text)));
        text.setSelectedRange(NSRange::new(2, 0));
        assert!(font_at(0) < 1.);
        let heading = fixture
            .split("Itinerario")
            .next()
            .unwrap()
            .encode_utf16()
            .count();
        assert_eq!(font_at(heading), 23.);
        assert_eq!(
            state.heading_labels.borrow()[0].stringValue().to_string(),
            "≡1"
        );
        text.setSelectedRange(NSRange::new(heading + 2, 0));
        assert_eq!(
            state.heading_labels.borrow()[0].stringValue().to_string(),
            "≡2"
        );
        assert_eq!(state.tag_buttons.borrow().len(), 1);
        let delimiter = fixture.split("**").next().unwrap().encode_utf16().count();
        assert!(font_at(delimiter) < 1.);
        // Keep mouse tracking open: the delimiters must appear before mouse-up.
        text.setSelectedRange_affinity_stillSelecting(
            NSRange::new(delimiter + 3, 0),
            objc2_app_kit::NSSelectionAffinity::Downstream,
            true,
        );
        assert_eq!(font_at(delimiter), 15.);
        let checkbox = state.checkboxes.borrow().first().unwrap().clone();
        let original_tag = checkbox.tag();
        let insertion = fixture[..fixture.find("Un paragrafo").unwrap()]
            .encode_utf16()
            .count();
        text.setSelectedRange(NSRange::new(insertion, 0));
        // SAFETY: Both replacement ranges are valid UTF-16 boundaries in the test note.
        unsafe {
            text.insertText_replacementRange(&NSString::from_str("🐻"), NSRange::new(insertion, 0));
        }
        assert!(
            std::ptr::eq(&*state.checkboxes.borrow()[0], &*checkbox),
            "Typing must reuse native controls"
        );
        assert_eq!(checkbox.tag(), original_tag + 2);
        // SAFETY: The two UTF-16 code units belong to the just-inserted emoji.
        unsafe {
            text.insertText_replacementRange(&NSString::from_str(""), NSRange::new(insertion, 2));
        }
        assert_eq!(checkbox.tag(), original_tag);
        assert_eq!(self.value(), fixture);
        unsafe {
            // SAFETY: The retained test checkbox has a live target and no sender is required.
            checkbox.performClick(None);
        }
        assert_eq!(self.value(), fixture.replace("[ ]", "[x]"));
        let undo = text.undoManager().unwrap();
        undo.undo();
        assert_eq!(self.value(), fixture);
        self.set_mode(EditorMode::Reading);
        assert!(!text.isEditable());
        let checkbox = state.checkboxes.borrow().first().unwrap().clone();
        // SAFETY: The retained checkbox has a live native action target.
        unsafe { checkbox.performClick(None) };
        assert_eq!(self.value(), fixture.replace("[ ]", "[x]"));
        assert!(!text.isEditable());
        self.set_mode(EditorMode::Protected);
        assert!(!state.checkboxes.borrow()[0].isEnabled());
        self.set_mode(EditorMode::Editing);
        assert!(text.window().unwrap().makeFirstResponder(Some(text)));
        undo.undo();
        assert_eq!(self.value(), fixture);
        let end = fixture.encode_utf16().count();
        text.setSelectedRange(NSRange::new(0, end));
        unsafe {
            // SAFETY: The replacement is an NSString and the range covers the loaded fixture.
            text.insertText_replacementRange(&NSString::from_str("Breve 🐻"), NSRange::new(0, end));
        }
        assert_eq!(self.value(), "Breve 🐻");
        let headings = format!("{fixture}\n\n### Dettagli\n\n#### Allegati");
        self.load(&headings);
        let mut previous_y = 0.;
        for (name, indicator) in [
            ("Appunti", "≡1"),
            ("Itinerario", "≡2"),
            ("Dettagli", "≡3"),
            ("Allegati", "≡4"),
        ] {
            let index = headings[..headings.find(name).unwrap()]
                .encode_utf16()
                .count()
                + 1;
            text.setSelectedRange(NSRange::new(index, 0));
            let labels = state.heading_labels.borrow();
            assert_eq!(labels.len(), 1);
            assert_eq!(labels[0].stringValue().to_string(), indicator);
            assert!(labels[0].frame().origin.y > previous_y);
            previous_y = labels[0].frame().origin.y;
        }
        let layout_fixture = "# Titolo 🐻\n\n- Primo\n  - Annidato\n\nFormula $E=mc^2$ fine.\n\n$$\n\\sum_{k=1}^n k\n$$";
        self.load(layout_fixture);
        let layout = &state.highlight_layout;
        // SAFETY: The smoke test uses the native text view and its matching container.
        let container = unsafe { text.textContainer() }.unwrap();
        let character_offset = |needle| {
            layout_fixture[..layout_fixture.find(needle).unwrap()]
                .encode_utf16()
                .count()
        };
        let glyph_rect = |needle| {
            layout.ensureLayoutForTextContainer(&container);
            // SAFETY: The offset is within the loaded fixture and no output range is needed.
            let glyphs = unsafe {
                layout.glyphRangeForCharacterRange_actualCharacterRange(
                    NSRange::new(character_offset(needle), 1),
                    std::ptr::null_mut(),
                )
            };
            layout.boundingRectForGlyphRange_inTextContainer(glyphs, &container)
        };
        assert_eq!(state.list_markers.borrow().len(), 2);
        assert!((glyph_rect("Annidato").origin.x - glyph_rect("Primo").origin.x - 33.).abs() < 1.);
        assert!(glyph_rect("$E=").size.width > 45.);
        assert!(glyph_rect("$$").size.height > 50.);
        text.setSelectedRange_affinity_stillSelecting(
            NSRange::new(character_offset("$E=") + 2, 0),
            objc2_app_kit::NSSelectionAffinity::Downstream,
            true,
        );
        assert!(glyph_rect("$E=").size.width < 15.);
        assert_eq!(self.value(), layout_fixture);
        text.setSelectedRange(NSRange::new(0, 0));
        assert!(glyph_rect("$E=").size.width > 45.);
        let table_prefix = "Appunti di viaggio 🐻\n\n";
        self.load(&format!("{table_prefix}{}", crate::blocks::TABLE));
        self.delegate.edit_table(
            table_prefix.len().try_into().unwrap(),
            crate::blocks::TableEdit::AddRow,
        );
        self.delegate.edit_table(
            table_prefix.len().try_into().unwrap(),
            crate::blocks::TableEdit::AddColumn,
        );
        assert!(self.value().contains("| --- | --- | --- |"));
        assert!(self.value().ends_with("|   |   |   |"));
        let directory = state.attachments.parent().unwrap();
        let image = directory.join("smoke-photo.png");
        image::RgbImage::from_fn(260, 120, |x, y| {
            image::Rgb([(x / 2) as u8, (y + 70) as u8, 150])
        })
        .save(&image)
        .unwrap();
        let markdown = crate::attachments::import(&state.attachments, &image).unwrap();
        let table = r"| Energia $E=mc^2$ | Calcolo |
| --- | --- |
| Da $a$ a $b$ | $\frac{1}{2}$ |
| `$letterale$` | $\openBearUnknown$ |";
        let note = format!(
            "Appunti di viaggio\n\n{table}\n\n> [!TIP] Promemoria\n> Prepara lo zaino.\n\n{markdown}"
        );
        self.load(&note);
        self.delegate.assert_table_formulas();
        assert_eq!(state.block_views.borrow().len(), 3);
        text.setSelectedRange(NSRange::new(
            "Appunti di viaggio\n\n| E".encode_utf16().count(),
            0,
        ));
        assert_eq!(state.block_views.borrow().len(), 2);
        text.setSelectedRange(NSRange::new(0, 0));
        assert_eq!(state.block_views.borrow().len(), 3);
        self.delegate.assert_table_formulas();
        assert_eq!(self.value(), note);
        let bounds = self.scroll.bounds();
        let bitmap = self
            .scroll
            .bitmapImageRepForCachingDisplayInRect(bounds)
            .unwrap();
        self.scroll
            .cacheDisplayInRect_toBitmapImageRep(bounds, &bitmap);
        // SAFETY: PNG encoding accepts an empty properties dictionary.
        let png = unsafe {
            bitmap.representationUsingType_properties(
                NSBitmapImageFileType::PNG,
                &objc2_foundation::NSDictionary::new(),
            )
        }
        .unwrap();
        let directory = crate::data_dir_override().unwrap();
        std::fs::write(directory.join("editor.png"), png.to_vec()).unwrap();
    }
}
