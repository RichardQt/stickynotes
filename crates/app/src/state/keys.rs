//! Keyboard shortcuts: turning key presses into actions, and recording new
//! combinations on the settings page.

use super::{AppState, SyncStatus};
use crate::quick_note::QuickAction;
use gpui_kit::*;
use markdown_editor::{BlockEditorState, BlockKind, EditorCommand, MarkKind};
use stickynotes_core::shortcuts::{self, Group};

/// The editor command behind a formatting action, for a caret in a block of
/// `kind`. `None` when the action does not apply there — the toolbar shows
/// its button disabled and the shortcut does nothing.
pub(crate) fn md_command(action: &str, kind: Option<BlockKind>) -> Option<EditorCommand> {
    let in_code_block = kind == Some(BlockKind::CodeBlock);
    // Headings and inline styles need a text block that is not code.
    let styles_text = kind.is_some() && !in_code_block;
    let heading = |level| EditorCommand::SetBlockKind(BlockKind::Heading { level });
    match action {
        shortcuts::MD_PARAGRAPH => {
            styles_text.then_some(EditorCommand::SetBlockKind(BlockKind::Paragraph))
        }
        shortcuts::MD_H1 => styles_text.then(|| heading(1)),
        shortcuts::MD_H2 => styles_text.then(|| heading(2)),
        shortcuts::MD_H3 => styles_text.then(|| heading(3)),
        shortcuts::MD_BOLD => styles_text.then_some(EditorCommand::ToggleMark(MarkKind::Bold)),
        shortcuts::MD_ITALIC => styles_text.then_some(EditorCommand::ToggleMark(MarkKind::Italic)),
        shortcuts::MD_UNDERLINE => {
            styles_text.then_some(EditorCommand::ToggleMark(MarkKind::Underline))
        }
        shortcuts::MD_STRIKE => {
            styles_text.then_some(EditorCommand::ToggleMark(MarkKind::Strikethrough))
        }
        shortcuts::MD_CODE => styles_text.then_some(EditorCommand::ToggleMark(MarkKind::Code)),
        shortcuts::MD_HIGHLIGHT => {
            styles_text.then_some(EditorCommand::ToggleMark(MarkKind::Highlight))
        }
        shortcuts::MD_BULLET => Some(EditorCommand::ToggleList { ordered: false }),
        shortcuts::MD_ORDERED => Some(EditorCommand::ToggleList { ordered: true }),
        shortcuts::MD_TASK => Some(EditorCommand::ToggleTask),
        shortcuts::MD_QUOTE => Some(EditorCommand::ToggleBlockquote),
        // Inside a code block the same key turns it back into text.
        shortcuts::MD_CODE_BLOCK => Some(if in_code_block {
            EditorCommand::SetBlockKind(BlockKind::Paragraph)
        } else {
            EditorCommand::SetCodeBlock { lang: None }
        }),
        shortcuts::MD_RULE => Some(EditorCommand::InsertBlock(BlockKind::HorizontalRule)),
        shortcuts::MD_TABLE => Some(EditorCommand::InsertTable { rows: 3, cols: 3 }),
        _ => None,
    }
}

/// Run a formatting action at the caret of `editor`.
fn run_md_action(editor: &Entity<BlockEditorState>, action: &str, cx: &mut App) {
    editor.update(cx, |state, cx| {
        if let Some(command) = md_command(action, state.block_kind_at_caret()) {
            let _ = state.run(command, cx);
        }
    });
}

/// Formatting keys the editor binds by itself. These actions belong to the
/// shortcut settings, so once the user has moved one to other keys (or
/// removed its shortcut) the old keys must stop working.
fn is_editor_builtin(combo: &shortcuts::Combo) -> bool {
    if !combo.ctrl || combo.shift || combo.win {
        return false;
    }
    if combo.alt {
        combo.key() == "0"
    } else {
        matches!(combo.key(), "b" | "i" | "e" | "k")
    }
}

impl AppState {
    /// Every key press of every window passes through here before the
    /// focused element sees it.
    pub(super) fn on_keystroke(
        &mut self,
        event: &KeystrokeEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let stroke = &event.keystroke;
        let key = stroke.key.as_str();
        // Holding a modifier down is reported as a key press of its own.
        if matches!(key, "shift" | "control" | "alt" | "platform" | "function") {
            return;
        }
        let held = stroke.modifiers;
        let plain = !(held.control || held.alt || held.shift || held.platform || held.function);
        let ctrl_only = held.control && !(held.alt || held.shift || held.platform || held.function);
        let combo = shortcuts::Combo::new(held.control, held.alt, held.shift, held.platform, key);
        let source = window.window_handle();

        let quick_window = self.quick.as_ref().and_then(|host| host.window).map(|(handle, _)| handle);
        if quick_window == Some(source) {
            self.on_quick_keystroke(event, plain, ctrl_only, &combo, cx);
            return;
        }
        if source != self.window_handle || self.startup_error.is_some() {
            return;
        }

        // While a shortcut is being recorded every key press is its input.
        if let Some(action) = self.recording_shortcut {
            cx.stop_propagation();
            self.record_keystroke(action, plain, &combo, cx);
            return;
        }
        if self.confirm_clear.is_some() {
            if plain && key == "escape" {
                cx.stop_propagation();
                self.cancel_clear_all(window, cx);
            }
            return;
        }
        if self.show_settings {
            if plain && key == "escape" {
                cx.stop_propagation();
                if self.settings_menu.take().is_some() {
                    cx.notify();
                } else {
                    self.close_settings(window, cx);
                }
            }
            return;
        }
        if self.quick_overlay_open {
            self.on_quick_keystroke(event, plain, ctrl_only, &combo, cx);
            return;
        }
        if self.link_popover_open {
            if plain && key == "escape" {
                cx.stop_propagation();
                self.close_link_popover(true, window, cx);
            } else if key == "tab"
                && !(held.control || held.alt || held.platform || held.function)
            {
                cx.stop_propagation();
                self.cycle_link_popover_focus(window, cx);
            } else if self
                .keymap
                .action_for(&combo)
                .is_some_and(|a| a.id == shortcuts::MD_LINK)
            {
                cx.stop_propagation();
                self.close_link_popover(true, window, cx);
            }
            return;
        }
        // AltGr on some keyboard layouts is Ctrl+Alt: a key that types a
        // character in the focused field is left alone.
        if event.input_preference == InputPreference::CharacterInput {
            return;
        }

        // The search box is also the list's keyboard home: arrows walk the
        // results, Enter goes on into the editor, and Esc clears the query
        // before it leaves the field. Without this, a query could only be
        // operated with the mouse again.
        let search_focused = self
            .search_input
            .as_ref()
            .is_some_and(|input| input.read(cx).focus_handle(cx).is_focused(window));
        if search_focused && plain {
            match key {
                "up" => {
                    cx.stop_propagation();
                    self.select_relative_note(-1, window, cx);
                    return;
                }
                "down" => {
                    cx.stop_propagation();
                    self.select_relative_note(1, window, cx);
                    return;
                }
                "enter" => {
                    cx.stop_propagation();
                    if let Some(editor) = &self.editor {
                        let handle = editor.read(cx).focus_handle(cx);
                        window.focus(&handle, cx);
                    }
                    return;
                }
                "escape" => {
                    cx.stop_propagation();
                    if self.search_query.is_empty() {
                        if let Some(editor) = &self.editor {
                            let handle = editor.read(cx).focus_handle(cx);
                            window.focus(&handle, cx);
                        }
                    } else {
                        self.clear_search(window, cx);
                    }
                    return;
                }
                _ => {}
            }
        }

        let body = self
            .editor
            .clone()
            .filter(|editor| editor.read(cx).focus_handle(cx).contains_focused(window, cx));
        let Some(action) = self.keymap.action_for(&combo) else {
            if body.is_some() && is_editor_builtin(&combo) {
                cx.stop_propagation();
            }
            return;
        };
        match action.group {
            // Normally Windows takes these keys before the window sees them.
            // They only arrive here when it could not be given them.
            Group::Global => {
                cx.stop_propagation();
                if action.id == shortcuts::OPEN_MAIN {
                    self.open_main_panel(window, cx);
                } else {
                    self.open_quick_note(window, cx);
                }
            }
            Group::General => {
                cx.stop_propagation();
                self.run_general_action(action.id, window, cx);
            }
            // Formatting applies to the note's body; elsewhere the key keeps
            // whatever meaning the focused field gives it.
            Group::Markdown => {
                if let Some(editor) = body {
                    cx.stop_propagation();
                    if action.id == shortcuts::MD_LINK {
                        self.open_link_popover(window, cx);
                    } else {
                        run_md_action(&editor, action.id, cx);
                    }
                }
            }
        }
    }

    /// Key presses in the quick note, whether it has a window of its own or
    /// is shown as a card in the main window.
    fn on_quick_keystroke(
        &mut self,
        event: &KeystrokeEvent,
        plain: bool,
        ctrl_only: bool,
        combo: &shortcuts::Combo,
        cx: &mut Context<Self>,
    ) {
        let key = combo.key();
        // Escape first closes the editor's table menu when that is open.
        let in_table_menu = event.context_stack.iter().any(|context| context.contains("table_menu"));
        if plain && key == "escape" && !in_table_menu {
            cx.stop_propagation();
            self.quick_request(QuickAction::Close, cx);
        } else if ctrl_only && key == "enter" {
            cx.stop_propagation();
            self.quick_request(QuickAction::OpenMain, cx);
        } else if event.input_preference != InputPreference::CharacterInput {
            let editor = self.quick.as_ref().map(|host| host.view.read(cx).editor().clone());
            match (self.keymap.action_for(combo), editor) {
                (Some(action), Some(editor)) if action.group == Group::Markdown => {
                    cx.stop_propagation();
                    run_md_action(&editor, action.id, cx);
                }
                // The app-wide actions (new tag, search, …) belong to the
                // main panel and do nothing while a quick note is written.
                (Some(_), _) => cx.stop_propagation(),
                (None, _) if is_editor_builtin(combo) => cx.stop_propagation(),
                (None, _) => {}
            }
        }
    }

    fn run_general_action(&mut self, action: &str, window: &mut Window, cx: &mut Context<Self>) {
        match action {
            shortcuts::NEW_TAG => self.begin_new_tag(window, cx),
            shortcuts::SEARCH => {
                if let Some(search) = &self.search_input {
                    let handle = search.read(cx).focus_handle(cx);
                    window.focus(&handle, cx);
                }
            }
            shortcuts::PREV_NOTE => {
                self.select_relative_note(-1, window, cx);
            }
            shortcuts::NEXT_NOTE => {
                self.select_relative_note(1, window, cx);
            }
            _ => {}
        }
    }

    // --- Shortcut page ---

    /// Start listening for the new shortcut of `action`; a second click on
    /// the same row stops listening.
    pub fn toggle_shortcut_recording(
        &mut self,
        action: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.shortcut_error = None;
        self.set_recording((self.recording_shortcut != Some(action)).then_some(action));
        // The key presses must not end up in a text field.
        window.focus(&self.modal_focus, cx);
        cx.notify();
    }

    pub fn cancel_shortcut_recording(&mut self, cx: &mut Context<Self>) {
        if self.recording_shortcut.is_some() {
            self.set_recording(None);
            self.shortcut_error = None;
            cx.notify();
        }
    }

    /// Start (`Some`) or stop listening for a new shortcut. The system-wide
    /// key is given up for that long and taken again afterwards.
    pub(super) fn set_recording(&mut self, action: Option<&'static str>) {
        let was_recording = self.recording_shortcut.is_some();
        self.recording_shortcut = action;
        if was_recording != action.is_some() {
            self.apply_global_hotkey();
        }
    }

    fn record_keystroke(
        &mut self,
        action: &'static str,
        plain: bool,
        combo: &shortcuts::Combo,
        cx: &mut Context<Self>,
    ) {
        let key = combo.key();
        if plain && key == "escape" {
            self.set_recording(None);
            self.shortcut_error = None;
            cx.notify();
            return;
        }
        // Backspace or Delete alone removes the shortcut.
        let wanted = (!(plain && matches!(key, "backspace" | "delete"))).then_some(combo);
        match shortcuts::assign(&mut self.config.shortcuts, action, wanted) {
            Ok(()) => {
                self.shortcut_error = None;
                self.shortcuts_changed();
                self.set_recording(None);
            }
            // Keep listening: the next key press is another try.
            Err(error) => self.shortcut_error = Some((action, error.message())),
        }
        cx.notify();
    }

    pub fn reset_shortcut(&mut self, action: &'static str, cx: &mut Context<Self>) {
        self.set_recording(None);
        match shortcuts::reset(&mut self.config.shortcuts, action) {
            Ok(()) => {
                self.shortcut_error = None;
                self.shortcuts_changed();
            }
            Err(error) => self.shortcut_error = Some((action, error.message())),
        }
        cx.notify();
    }

    pub fn reset_all_shortcuts(&mut self, cx: &mut Context<Self>) {
        self.set_recording(None);
        self.shortcut_error = None;
        self.config.shortcuts.clear();
        self.shortcuts_changed();
        cx.notify();
    }

    /// The stored changes were edited: apply them and write them down.
    fn shortcuts_changed(&mut self) {
        self.keymap = shortcuts::Keymap::resolve(&self.config.shortcuts);
        // The quick-note key may be among them.
        self.apply_global_hotkey();
        if let Err(e) = self.config.save() {
            self.status = SyncStatus::Error(format!("保存设置失败：{e}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{is_editor_builtin, md_command};
    use markdown_editor::{BlockKind, EditorCommand, MarkKind};
    use stickynotes_core::shortcuts;

    #[test]
    fn underline_maps_to_toggle_mark_outside_code_blocks() {
        assert_eq!(
            md_command(shortcuts::MD_UNDERLINE, Some(BlockKind::Paragraph)),
            Some(EditorCommand::ToggleMark(MarkKind::Underline))
        );
        assert_eq!(
            md_command(shortcuts::MD_UNDERLINE, Some(BlockKind::Heading { level: 2 })),
            Some(EditorCommand::ToggleMark(MarkKind::Underline))
        );
        assert_eq!(
            md_command(shortcuts::MD_UNDERLINE, Some(BlockKind::CodeBlock)),
            None
        );
        assert_eq!(md_command(shortcuts::MD_UNDERLINE, None), None);
    }

    #[test]
    fn editor_builtin_blocks_ctrl_k_when_unmapped() {
        assert!(is_editor_builtin(&shortcuts::Combo::parse("Ctrl+K").unwrap()));
        assert!(is_editor_builtin(&shortcuts::Combo::parse("Ctrl+B").unwrap()));
        assert!(!is_editor_builtin(&shortcuts::Combo::parse("Ctrl+U").unwrap()));
    }
}
