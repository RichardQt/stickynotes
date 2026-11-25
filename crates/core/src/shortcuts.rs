//! Keyboard shortcuts: the actions that can be bound, their default keys, and
//! the text form a key combination is stored and shown in (`Ctrl+Shift+N`).
//!
//! Only data and rules live here. The app turns key presses into [`Combo`]s,
//! looks them up in a [`Keymap`] and runs the action.

use std::collections::BTreeMap;
use std::fmt;

/// Where an action is listed on the shortcut page, and when its key works.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    /// Works in every program for as long as the app runs: Windows takes the
    /// key press before any window sees it.
    Global,
    /// Works anywhere in the main window.
    General,
    /// Formatting; works while the caret is in a note's body.
    Markdown,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ActionDef {
    pub id: &'static str,
    pub label: &'static str,
    pub group: Group,
    /// Default key combination; empty for "no shortcut".
    pub default: &'static str,
}

const fn action(
    id: &'static str,
    label: &'static str,
    group: Group,
    default: &'static str,
) -> ActionDef {
    ActionDef { id, label, group, default }
}

pub const QUICK_NOTE: &str = "quick_note";
pub const OPEN_MAIN: &str = "open_main";
pub const NEW_TAG: &str = "new_tag";
pub const SEARCH: &str = "search";
pub const PREV_NOTE: &str = "prev_note";
pub const NEXT_NOTE: &str = "next_note";
pub const MD_PARAGRAPH: &str = "md_paragraph";
pub const MD_H1: &str = "md_h1";
pub const MD_H2: &str = "md_h2";
pub const MD_H3: &str = "md_h3";
pub const MD_BOLD: &str = "md_bold";
pub const MD_ITALIC: &str = "md_italic";
pub const MD_UNDERLINE: &str = "md_underline";
pub const MD_STRIKE: &str = "md_strike";
pub const MD_CODE: &str = "md_code";
pub const MD_HIGHLIGHT: &str = "md_highlight";
pub const MD_LINK: &str = "md_link";
pub const MD_BULLET: &str = "md_bullet";
pub const MD_ORDERED: &str = "md_ordered";
pub const MD_TASK: &str = "md_task";
pub const MD_QUOTE: &str = "md_quote";
pub const MD_CODE_BLOCK: &str = "md_code_block";
pub const MD_RULE: &str = "md_rule";
pub const MD_TABLE: &str = "md_table";

/// Every action a shortcut can be given to, in the order the settings page
/// lists them.
///
/// The defaults avoid Shift + a digit or punctuation key: Windows reports
/// those as the shifted character (`Ctrl+Shift+8` arrives as `Ctrl+*`), and
/// `Ctrl+Shift+0` never arrives at all.
pub const ACTIONS: &[ActionDef] = &[
    action(QUICK_NOTE, "快速记事", Group::Global, "Ctrl+Alt+Q"),
    action(OPEN_MAIN, "打开主面板", Group::Global, "Ctrl+Alt+S"),
    action(NEW_TAG, "新建标签", Group::General, "Ctrl+Shift+N"),
    action(SEARCH, "搜索便签", Group::General, "Ctrl+F"),
    // Arrow keys are editing keys by this app's shortcut policy (and Alt+Up
    // moves lines in many editors): P/N are free, mnemonic and safe.
    action(PREV_NOTE, "上一条便签", Group::General, "Alt+P"),
    action(NEXT_NOTE, "下一条便签", Group::General, "Alt+N"),
    action(MD_PARAGRAPH, "正文", Group::Markdown, "Ctrl+Alt+0"),
    action(MD_H1, "一级标题", Group::Markdown, "Ctrl+Alt+1"),
    action(MD_H2, "二级标题", Group::Markdown, "Ctrl+Alt+2"),
    action(MD_H3, "三级标题", Group::Markdown, "Ctrl+Alt+3"),
    action(MD_BOLD, "加粗", Group::Markdown, "Ctrl+B"),
    action(MD_ITALIC, "斜体", Group::Markdown, "Ctrl+I"),
    action(MD_UNDERLINE, "下划线", Group::Markdown, "Ctrl+U"),
    action(MD_STRIKE, "删除线", Group::Markdown, "Ctrl+Shift+X"),
    action(MD_CODE, "行内代码", Group::Markdown, "Ctrl+E"),
    action(MD_HIGHLIGHT, "文本高亮", Group::Markdown, "Ctrl+Shift+H"),
    action(MD_LINK, "超链接", Group::Markdown, "Ctrl+K"),
    action(MD_BULLET, "无序列表", Group::Markdown, "Ctrl+Shift+U"),
    action(MD_ORDERED, "有序列表", Group::Markdown, "Ctrl+Shift+O"),
    action(MD_TASK, "待办清单", Group::Markdown, "Ctrl+Shift+T"),
    action(MD_QUOTE, "引用块", Group::Markdown, "Ctrl+Shift+Q"),
    action(MD_CODE_BLOCK, "代码块", Group::Markdown, "Ctrl+Shift+K"),
    action(MD_RULE, "分割线", Group::Markdown, "Ctrl+Alt+H"),
    action(MD_TABLE, "插入表格", Group::Markdown, "Ctrl+Alt+T"),
];

pub fn action_def(id: &str) -> Option<&'static ActionDef> {
    ACTIONS.iter().find(|a| a.id == id)
}

impl ActionDef {
    pub fn default_combo(&self) -> Option<Combo> {
        Combo::parse(self.default)
    }
}

/// Keys that have a name instead of a character, as the UI toolkit reports
/// them, with the spelling shown to the user.
const NAMED_KEYS: &[(&str, &str)] = &[
    ("enter", "Enter"),
    ("tab", "Tab"),
    ("escape", "Esc"),
    ("space", "Space"),
    ("backspace", "Backspace"),
    ("delete", "Delete"),
    ("insert", "Insert"),
    ("up", "Up"),
    ("down", "Down"),
    ("left", "Left"),
    ("right", "Right"),
    ("home", "Home"),
    ("end", "End"),
    ("pageup", "PageUp"),
    ("pagedown", "PageDown"),
    ("menu", "Menu"),
    ("back", "Back"),
    ("forward", "Forward"),
];

fn function_key_number(key: &str) -> Option<u8> {
    let n: u8 = key.strip_prefix('f')?.parse().ok()?;
    (1..=24).contains(&n).then_some(n)
}

/// Modifiers plus one key.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Combo {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    /// Lower-case key as the UI toolkit names it: `n`, `0`, `/`, `f5`.
    key: String,
}

/// Why a key combination cannot be used as a shortcut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComboError {
    /// Windows-key combinations belong to the system.
    WinKey,
    /// Enter, Tab, arrows and the like are needed for typing and moving.
    EditingKey,
    /// A bare key, or Shift + key, would get in the way of typing.
    NeedsModifier,
    /// Copy, paste, undo, select all and Alt+F4 keep their meaning.
    Reserved,
}

impl ComboError {
    pub fn message(self) -> &'static str {
        match self {
            ComboError::WinKey => "不支持包含 Win 键的组合",
            ComboError::EditingKey => "回车、Tab、方向键等用于输入和移动光标，不能设为快捷键",
            ComboError::NeedsModifier => "需要包含 Ctrl 或 Alt（F1–F12 可单独使用）",
            ComboError::Reserved => "这个组合保留给复制、粘贴、撤销、全选等基本操作",
        }
    }
}

impl Combo {
    /// A combination as it was pressed. `key` is the toolkit's key name.
    pub fn new(ctrl: bool, alt: bool, shift: bool, win: bool, key: &str) -> Self {
        Self { ctrl, alt, shift, win, key: key.to_ascii_lowercase() }
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    /// Read the stored / displayed form: `Ctrl+Shift+N`, `ctrl+alt+0`, `F2`.
    /// `None` when the text names no key this app knows.
    pub fn parse(text: &str) -> Option<Self> {
        let mut combo = Combo::default();
        let mut rest = text.trim();
        // Modifiers are prefixes, so `+` itself can still be the key (`Ctrl++`).
        while let Some((head, tail)) = rest.split_once('+') {
            match head.trim().to_ascii_lowercase().as_str() {
                "ctrl" | "control" => combo.ctrl = true,
                "alt" => combo.alt = true,
                "shift" => combo.shift = true,
                "win" => combo.win = true,
                _ => break,
            }
            rest = tail.trim_start();
        }
        let key = rest.trim_end();
        if key.is_empty() {
            return None;
        }
        let lower = key.to_ascii_lowercase();
        combo.key = if key.chars().count() == 1 {
            lower
        } else if function_key_number(&lower).is_some() {
            lower
        } else {
            let (name, _) = NAMED_KEYS
                .iter()
                .find(|(name, shown)| *name == lower || shown.eq_ignore_ascii_case(&lower))?;
            (*name).to_string()
        };
        Some(combo)
    }

    /// Whether this combination may be used as a shortcut at all.
    pub fn validate(&self) -> Result<(), ComboError> {
        if self.win {
            return Err(ComboError::WinKey);
        }
        let function_key = function_key_number(&self.key).is_some();
        if self.key.chars().count() != 1 && !function_key {
            return Err(ComboError::EditingKey);
        }
        if !function_key && !self.ctrl && !self.alt {
            return Err(ComboError::NeedsModifier);
        }
        if self.is_reserved() {
            return Err(ComboError::Reserved);
        }
        Ok(())
    }

    fn is_reserved(&self) -> bool {
        let ctrl_only = self.ctrl && !self.alt;
        match self.key.as_str() {
            "c" | "x" | "a" | "y" => ctrl_only && !self.shift,
            // With Shift these are paste-as-plain-text and redo.
            "v" | "z" => ctrl_only,
            "f4" => self.alt && !self.ctrl && !self.shift,
            _ => false,
        }
    }
}

impl fmt::Display for Combo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (held, name) in [
            (self.ctrl, "Ctrl"),
            (self.alt, "Alt"),
            (self.shift, "Shift"),
            (self.win, "Win"),
        ] {
            if held {
                write!(f, "{name}+")?;
            }
        }
        match NAMED_KEYS.iter().find(|(name, _)| *name == self.key) {
            Some((_, shown)) => f.write_str(shown),
            None => f.write_str(&self.key.to_ascii_uppercase()),
        }
    }
}

/// The shortcuts in effect: the defaults with the user's changes applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keymap {
    /// One entry per [`ACTIONS`] item, in the same order.
    combos: Vec<Option<Combo>>,
}

impl Default for Keymap {
    fn default() -> Self {
        Self::resolve(&BTreeMap::new())
    }
}

impl Keymap {
    /// Apply the stored changes (action id → combination, empty for "no
    /// shortcut") on top of the defaults.
    ///
    /// The result never binds one combination twice. Changes come first; an
    /// action left on its default loses that default when a change already
    /// took the same keys. A change that is unreadable, or not allowed as a
    /// shortcut, is ignored.
    pub fn resolve(overrides: &BTreeMap<String, String>) -> Self {
        enum Wanted {
            Default,
            Unbound,
            Changed(Combo),
        }
        let wanted: Vec<Wanted> = ACTIONS
            .iter()
            .map(|def| match overrides.get(def.id).map(|text| text.trim()) {
                None => Wanted::Default,
                Some("") => Wanted::Unbound,
                Some(text) => match Combo::parse(text).filter(|c| c.validate().is_ok()) {
                    Some(combo) => Wanted::Changed(combo),
                    None => Wanted::Default,
                },
            })
            .collect();

        let mut combos: Vec<Option<Combo>> = vec![None; ACTIONS.len()];
        for (slot, want) in wanted.iter().enumerate() {
            if let Wanted::Changed(combo) = want {
                if !combos.contains(&Some(combo.clone())) {
                    combos[slot] = Some(combo.clone());
                }
            }
        }
        for (slot, want) in wanted.iter().enumerate() {
            if let Wanted::Default = want {
                let combo = ACTIONS[slot].default_combo();
                if combo.is_some() && !combos.contains(&combo) {
                    combos[slot] = combo;
                }
            }
        }
        Self { combos }
    }

    fn slot(action: &str) -> Option<usize> {
        ACTIONS.iter().position(|a| a.id == action)
    }

    /// The combination bound to `action`, if any.
    pub fn combo(&self, action: &str) -> Option<&Combo> {
        self.combos.get(Self::slot(action)?)?.as_ref()
    }

    /// `Ctrl+Shift+N`, for hints next to the things the shortcut triggers.
    pub fn label(&self, action: &str) -> Option<String> {
        self.combo(action).map(Combo::to_string)
    }

    /// The action `combo` triggers.
    pub fn action_for(&self, combo: &Combo) -> Option<&'static ActionDef> {
        let slot = self.combos.iter().position(|c| c.as_ref() == Some(combo))?;
        ACTIONS.get(slot)
    }

    /// Whether `action` currently has exactly its default shortcut.
    pub fn is_default(&self, action: &str) -> bool {
        match action_def(action) {
            Some(def) => self.combo(action) == def.default_combo().as_ref(),
            None => true,
        }
    }
}

/// Why a shortcut was not changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignError {
    UnknownAction,
    Invalid(ComboError),
    /// Another action already uses the combination.
    Taken(&'static ActionDef),
}

impl AssignError {
    pub fn message(&self) -> String {
        match self {
            AssignError::UnknownAction => "未知的操作".to_string(),
            AssignError::Invalid(error) => error.message().to_string(),
            AssignError::Taken(other) => format!("已被「{}」使用，请先修改那一项", other.label),
        }
    }
}

/// Give `action` the shortcut `combo` — `None` for no shortcut — by updating
/// the stored changes. Nothing is changed when the combination is not
/// allowed or another action already uses it.
pub fn assign(
    overrides: &mut BTreeMap<String, String>,
    action: &str,
    combo: Option<&Combo>,
) -> Result<(), AssignError> {
    let def = action_def(action).ok_or(AssignError::UnknownAction)?;
    if let Some(combo) = combo {
        combo.validate().map_err(AssignError::Invalid)?;
        match Keymap::resolve(overrides).action_for(combo) {
            Some(other) if other.id != def.id => return Err(AssignError::Taken(other)),
            _ => {}
        }
    }
    // Only differences from the default are stored.
    if combo == def.default_combo().as_ref() {
        overrides.remove(def.id);
    } else {
        let text = combo.map(Combo::to_string).unwrap_or_default();
        overrides.insert(def.id.to_string(), text);
    }
    Ok(())
}

/// Put `action` back on its default shortcut.
pub fn reset(overrides: &mut BTreeMap<String, String>, action: &str) -> Result<(), AssignError> {
    let def = action_def(action).ok_or(AssignError::UnknownAction)?;
    assign(overrides, action, def.default_combo().as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn combo(text: &str) -> Combo {
        Combo::parse(text).unwrap_or_else(|| panic!("{text} should parse"))
    }

    fn overrides(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn every_default_is_allowed_unique_and_round_trips() {
        let mut seen = Vec::new();
        let mut ids = Vec::new();
        for def in ACTIONS {
            assert!(!ids.contains(&def.id), "duplicate action id {}", def.id);
            ids.push(def.id);
            let combo = def.default_combo().unwrap_or_else(|| panic!("{} has no default", def.id));
            assert_eq!(combo.validate(), Ok(()), "{}", def.id);
            assert_eq!(combo.to_string(), def.default, "{}", def.id);
            assert!(!seen.contains(&combo), "{} reuses {}", def.id, def.default);
            // Shift + digit or punctuation never arrives as pressed on Windows.
            assert!(
                !combo.shift || combo.key().chars().all(|c| c.is_ascii_alphabetic()),
                "{} uses Shift with a non-letter key",
                def.id
            );
            seen.push(combo);
        }
        let keymap = Keymap::default();
        for def in ACTIONS {
            assert_eq!(keymap.label(def.id).as_deref(), Some(def.default));
            assert!(keymap.is_default(def.id));
        }
    }

    #[test]
    fn combinations_parse_leniently_and_print_one_way() {
        assert_eq!(combo("ctrl+shift+n"), Combo::new(true, false, true, false, "n"));
        assert_eq!(combo("  Control + ALT + 0 "), Combo::new(true, true, false, false, "0"));
        assert_eq!(combo("ctrl+shift+n").to_string(), "Ctrl+Shift+N");
        assert_eq!(combo("alt+ctrl+0").to_string(), "Ctrl+Alt+0");
        assert_eq!(combo("f2"), Combo::new(false, false, false, false, "f2"));
        assert_eq!(combo("Shift+F12").to_string(), "Shift+F12");
        // Punctuation, including the separator itself, can be the key.
        assert_eq!(combo("Ctrl++"), Combo::new(true, false, false, false, "+"));
        assert_eq!(combo("Ctrl++").to_string(), "Ctrl++");
        assert_eq!(combo("+"), Combo::new(false, false, false, false, "+"));
        assert_eq!(combo("Ctrl+,").to_string(), "Ctrl+,");
        assert_eq!(combo("Ctrl+-").to_string(), "Ctrl+-");
        // Named keys are shown with their usual spelling and read back.
        assert_eq!(Combo::new(true, false, false, false, "escape").to_string(), "Ctrl+Esc");
        assert_eq!(combo("Ctrl+Esc").key(), "escape");
        assert_eq!(combo("ctrl+escape").key(), "escape");
        assert_eq!(combo("Alt+PageDown").key(), "pagedown");
        assert_eq!(combo("Win+E"), Combo::new(false, false, false, true, "e"));
        // What the toolkit reports is what gets compared.
        assert_eq!(Combo::new(true, false, false, false, "N"), combo("Ctrl+N"));

        for bad in ["", "   ", "Ctrl+", "Ctrl+Shift+", "Ctrl+Foo", "Hyper+N", "F25", "F0", "Ctrl+NN"] {
            assert_eq!(Combo::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn only_combinations_that_cannot_disturb_typing_are_allowed() {
        for ok in ["Ctrl+N", "Alt+N", "Ctrl+Shift+N", "Ctrl+Alt+0", "Ctrl+/", "F2", "Shift+F5", "Ctrl+F4", "Ctrl+K"] {
            assert_eq!(combo(ok).validate(), Ok(()), "{ok}");
        }
        for needs in ["N", "Shift+N", "1", "/"] {
            assert_eq!(combo(needs).validate(), Err(ComboError::NeedsModifier), "{needs}");
        }
        for editing in ["Enter", "Ctrl+Enter", "Tab", "Esc", "Ctrl+Space", "Ctrl+Backspace", "Alt+Left", "Ctrl+Home", "Ctrl+Delete"] {
            assert_eq!(combo(editing).validate(), Err(ComboError::EditingKey), "{editing}");
        }
        for reserved in ["Ctrl+C", "Ctrl+X", "Ctrl+V", "Ctrl+Shift+V", "Ctrl+A", "Ctrl+Z", "Ctrl+Shift+Z", "Ctrl+Y", "Alt+F4"] {
            assert_eq!(combo(reserved).validate(), Err(ComboError::Reserved), "{reserved}");
        }
        // The same letters with other modifiers are free.
        for free in ["Ctrl+Shift+C", "Ctrl+Alt+V", "Alt+A", "Ctrl+Shift+Y", "Ctrl+Alt+F4"] {
            assert_eq!(combo(free).validate(), Ok(()), "{free}");
        }
        assert_eq!(combo("Win+N").validate(), Err(ComboError::WinKey));
        assert_eq!(combo("Ctrl+Win+N").validate(), Err(ComboError::WinKey));
    }

    #[test]
    fn stored_changes_replace_remove_or_fall_back_to_defaults() {
        let keymap = Keymap::resolve(&overrides(&[
            (SEARCH, "Ctrl+Alt+N"),
            (MD_BOLD, ""),
            (MD_ITALIC, "not a key"),
            (MD_CODE, "Ctrl+C"),
            ("no_such_action", "Ctrl+J"),
        ]));
        assert_eq!(keymap.label(SEARCH).as_deref(), Some("Ctrl+Alt+N"));
        assert!(!keymap.is_default(SEARCH));
        assert_eq!(keymap.combo(MD_BOLD), None, "an empty change means no shortcut");
        assert!(!keymap.is_default(MD_BOLD));
        assert_eq!(keymap.label(MD_ITALIC).as_deref(), Some("Ctrl+I"), "unreadable → default");
        assert_eq!(keymap.label(MD_CODE).as_deref(), Some("Ctrl+E"), "not allowed → default");
        assert_eq!(keymap.label(NEW_TAG).as_deref(), Some("Ctrl+Shift+N"));

        assert_eq!(keymap.action_for(&combo("Ctrl+Alt+N")).map(|a| a.id), Some(SEARCH));
        assert_eq!(keymap.action_for(&combo("Ctrl+F")), None, "the old default is free again");
        assert_eq!(keymap.action_for(&combo("Ctrl+B")), None);
        assert_eq!(keymap.action_for(&combo("Ctrl+J")), None);
        assert_eq!(keymap.combo("no_such_action"), None);
        assert_eq!(keymap.label("no_such_action"), None);
    }

    #[test]
    fn one_combination_is_never_bound_twice() {
        // A change that takes another action's default wins; that action is
        // left without a shortcut instead of both firing.
        let keymap = Keymap::resolve(&overrides(&[(MD_TABLE, "Ctrl+B")]));
        assert_eq!(keymap.action_for(&combo("Ctrl+B")).map(|a| a.id), Some(MD_TABLE));
        assert_eq!(keymap.combo(MD_BOLD), None);

        // Two changes asking for the same keys: the earlier action keeps them.
        let keymap = Keymap::resolve(&overrides(&[(MD_H1, "Ctrl+J"), (MD_H2, "Ctrl+J")]));
        assert_eq!(keymap.action_for(&combo("Ctrl+J")).map(|a| a.id), Some(MD_H1));
        assert_eq!(keymap.combo(MD_H2), None);

        let bound: Vec<&Combo> = ACTIONS.iter().filter_map(|a| keymap.combo(a.id)).collect();
        let mut unique = bound.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(bound.len(), unique.len());
    }

    #[test]
    fn assigning_stores_only_differences_from_the_default() {
        let mut stored = BTreeMap::new();
        assign(&mut stored, SEARCH, Some(&combo("Ctrl+Alt+N"))).unwrap();
        assert_eq!(stored, overrides(&[(SEARCH, "Ctrl+Alt+N")]));

        // No shortcut at all is a change too.
        assign(&mut stored, MD_BOLD, None).unwrap();
        assert_eq!(stored.get(MD_BOLD).map(String::as_str), Some(""));
        assert_eq!(Keymap::resolve(&stored).combo(MD_BOLD), None);

        // Back on the default: nothing is stored for the action.
        assign(&mut stored, SEARCH, Some(&combo("Ctrl+F"))).unwrap();
        assert!(!stored.contains_key(SEARCH));
        reset(&mut stored, MD_BOLD).unwrap();
        assert!(stored.is_empty());
        assert_eq!(Keymap::resolve(&stored), Keymap::default());

        // Re-assigning an action its own current keys is fine.
        assign(&mut stored, MD_ITALIC, Some(&combo("Ctrl+I"))).unwrap();
        assert!(stored.is_empty());
    }

    #[test]
    fn the_system_wide_keys_are_shortcuts_of_their_own() {
        let quick = action_def(QUICK_NOTE).unwrap();
        assert_eq!(quick.group, Group::Global);
        assert_eq!(Keymap::default().label(QUICK_NOTE).as_deref(), Some("Ctrl+Alt+Q"));

        // Windows registers every global action as a key of its own.
        let main = action_def(OPEN_MAIN).unwrap();
        assert_eq!(main.group, Group::Global);
        assert_eq!(ACTIONS.iter().filter(|a| a.group == Group::Global).count(), 2);
        assert_eq!(Keymap::default().label(OPEN_MAIN).as_deref(), Some("Ctrl+Alt+S"));

        // Their key presses never reach the window, so no other action may
        // share them — until it has been moved or switched off.
        let mut stored = BTreeMap::new();
        assert_eq!(
            assign(&mut stored, SEARCH, Some(&combo("Ctrl+Alt+Q"))),
            Err(AssignError::Taken(quick))
        );
        assert_eq!(
            assign(&mut stored, SEARCH, Some(&combo("Ctrl+Alt+S"))),
            Err(AssignError::Taken(main))
        );
        assign(&mut stored, QUICK_NOTE, Some(&combo("Ctrl+Alt+J"))).unwrap();
        assign(&mut stored, SEARCH, Some(&combo("Ctrl+Alt+Q"))).unwrap();
        let keymap = Keymap::resolve(&stored);
        assert_eq!(keymap.action_for(&combo("Ctrl+Alt+J")).map(|a| a.id), Some(QUICK_NOTE));
        assert_eq!(keymap.action_for(&combo("Ctrl+Alt+Q")).map(|a| a.id), Some(SEARCH));

        assign(&mut stored, QUICK_NOTE, None).unwrap();
        assert_eq!(Keymap::resolve(&stored).combo(QUICK_NOTE), None);

        // The two global keys are separate: changing one leaves the other.
        assign(&mut stored, OPEN_MAIN, Some(&combo("Ctrl+Alt+M"))).unwrap();
        let keymap = Keymap::resolve(&stored);
        assert_eq!(keymap.action_for(&combo("Ctrl+Alt+M")).map(|a| a.id), Some(OPEN_MAIN));
        assert_eq!(keymap.action_for(&combo("Ctrl+Alt+S")), None);

        // Keys someone already gave to another action stay with it: the
        // quick note then simply has none until it is given some.
        let keymap = Keymap::resolve(&overrides(&[(SEARCH, "Ctrl+Alt+Q")]));
        assert_eq!(keymap.action_for(&combo("Ctrl+Alt+Q")).map(|a| a.id), Some(SEARCH));
        assert_eq!(keymap.combo(QUICK_NOTE), None);
    }

    #[test]
    fn assigning_refuses_taken_and_disallowed_combinations() {
        let mut stored = BTreeMap::new();
        let taken = assign(&mut stored, SEARCH, Some(&combo("Ctrl+B"))).unwrap_err();
        assert_eq!(taken, AssignError::Taken(action_def(MD_BOLD).unwrap()));
        assert!(taken.message().contains("加粗"), "{}", taken.message());
        assert_eq!(
            assign(&mut stored, SEARCH, Some(&combo("Ctrl+C"))),
            Err(AssignError::Invalid(ComboError::Reserved))
        );
        assert_eq!(
            assign(&mut stored, SEARCH, Some(&combo("N"))),
            Err(AssignError::Invalid(ComboError::NeedsModifier))
        );
        assert_eq!(
            assign(&mut stored, "no_such_action", Some(&combo("Ctrl+J"))),
            Err(AssignError::UnknownAction)
        );
        assert_eq!(reset(&mut stored, "no_such_action"), Err(AssignError::UnknownAction));
        assert!(stored.is_empty(), "a refused change stores nothing");

        // Freeing the keys first makes them available…
        assign(&mut stored, MD_BOLD, None).unwrap();
        assign(&mut stored, SEARCH, Some(&combo("Ctrl+B"))).unwrap();
        assert_eq!(Keymap::resolve(&stored).action_for(&combo("Ctrl+B")).map(|a| a.id), Some(SEARCH));
        // …and then the old owner cannot simply be reset onto them.
        assert_eq!(
            reset(&mut stored, MD_BOLD),
            Err(AssignError::Taken(action_def(SEARCH).unwrap()))
        );
        assert_eq!(Keymap::resolve(&stored).combo(MD_BOLD), None);
    }
}
