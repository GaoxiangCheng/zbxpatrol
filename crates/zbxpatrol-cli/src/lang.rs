//! UI language: English (default) / Chinese (--lang zh or L key to toggle).

use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    En,
    Zh,
}

impl Lang {
    pub fn label(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Zh => "中文",
        }
    }
}

static LANG: AtomicU8 = AtomicU8::new(0); // 0 = En, 1 = Zh

pub fn set(lang: Lang) {
    LANG.store(lang as u8, Ordering::Relaxed);
}

pub fn get() -> Lang {
    match LANG.load(Ordering::Relaxed) {
        1 => Lang::Zh,
        _ => Lang::En,
    }
}

/// Toggle between En ↔ Zh (L key in wizard)
pub fn toggle() -> Lang {
    let new = if get() == Lang::Zh { Lang::En } else { Lang::Zh };
    set(new);
    new
}

pub fn parse(s: &str) -> Option<Lang> {
    match s.to_lowercase().as_str() {
        "en" | "english" => Some(Lang::En),
        "zh" | "cn" | "zh-cn" | "chinese" | "中文" | "简体" => Some(Lang::Zh),
        _ => Some(Lang::En),
    }
}

/// Return the string for the active language.
pub fn t<'a>(en: &'a str, zh: &'a str) -> &'a str {
    if get() == Lang::Zh { zh } else { en }
}

pub fn is_zh() -> bool {
    get() == Lang::Zh
}
