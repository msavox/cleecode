use crate::i18n::{self, Key, Lang};

pub const SETTINGS_COUNT: usize = 9;

pub struct Settings {
    pub show_line_numbers: bool,
    pub syntax_highlighting: bool,
    pub word_wrap: bool,
    pub tab_size: usize,
    pub insert_spaces: bool,
    pub show_whitespace: bool,
    pub auto_indent: bool,
    pub mouse_enabled: bool,
    pub lang: Lang,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            show_line_numbers: true,
            syntax_highlighting: true,
            word_wrap: false,
            tab_size: 4,
            insert_spaces: true,
            show_whitespace: false,
            auto_indent: true,
            mouse_enabled: true,
            lang: Lang::default(),
        }
    }
}

pub struct SettingRow {
    pub label: &'static str,
    pub value: String,
}

impl Settings {
    pub fn rows(&self) -> Vec<SettingRow> {
        let lang = self.lang;
        let b = |v: bool| i18n::t(lang, if v { Key::On } else { Key::Off }).to_string();
        vec![
            SettingRow { label: i18n::t(lang, Key::SettingLineNumbers), value: b(self.show_line_numbers) },
            SettingRow { label: i18n::t(lang, Key::SettingSyntaxHighlighting), value: b(self.syntax_highlighting) },
            SettingRow { label: i18n::t(lang, Key::SettingWordWrap), value: b(self.word_wrap) },
            SettingRow { label: i18n::t(lang, Key::SettingTabSize), value: self.tab_size.to_string() },
            SettingRow { label: i18n::t(lang, Key::SettingInsertSpaces), value: b(self.insert_spaces) },
            SettingRow { label: i18n::t(lang, Key::SettingShowWhitespace), value: b(self.show_whitespace) },
            SettingRow { label: i18n::t(lang, Key::SettingAutoIndent), value: b(self.auto_indent) },
            SettingRow { label: i18n::t(lang, Key::SettingMouseEnabled), value: b(self.mouse_enabled) },
            SettingRow { label: i18n::t(lang, Key::SettingLanguage), value: self.lang.label().to_string() },
        ]
    }

    /// Enter/Space on a row: toggles booleans, bumps numbers, cycles enums.
    pub fn activate(&mut self, idx: usize) {
        match idx {
            0 => self.show_line_numbers = !self.show_line_numbers,
            1 => self.syntax_highlighting = !self.syntax_highlighting,
            2 => self.word_wrap = !self.word_wrap,
            3 => self.adjust_tab_size(1),
            4 => self.insert_spaces = !self.insert_spaces,
            5 => self.show_whitespace = !self.show_whitespace,
            6 => self.auto_indent = !self.auto_indent,
            7 => self.mouse_enabled = !self.mouse_enabled,
            8 => self.lang = self.lang.next(),
            _ => {}
        }
    }

    /// Left/Right on a row: only meaningful for numeric/enum rows, otherwise same as activate.
    pub fn adjust(&mut self, idx: usize, delta: i32) {
        if idx == 3 {
            self.adjust_tab_size(delta);
        } else {
            self.activate(idx);
        }
    }

    fn adjust_tab_size(&mut self, delta: i32) {
        let new_val = (self.tab_size as i32 + delta).clamp(1, 8);
        self.tab_size = new_val as usize;
    }
}
