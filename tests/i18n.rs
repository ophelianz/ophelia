rust_i18n::i18n!("locales", fallback = "en");

use std::sync::{Mutex, OnceLock};

use rust_i18n::t;

fn locale_lock() -> &'static Mutex<()> {
    static LOCALE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCALE_LOCK.get_or_init(|| Mutex::new(()))
}

#[test]
fn english_translations_resolve() {
    let _lock = locale_lock().lock().unwrap();
    rust_i18n::set_locale("en");

    assert_eq!(t!("sidebar.downloads").to_string(), "Transfers");
    assert_eq!(t!("menu.settings").to_string(), "Settings");
}

#[test]
fn mandarin_translations_resolve_and_fallback_to_english() {
    let _lock = locale_lock().lock().unwrap();
    rust_i18n::set_locale("zh-CN");

    assert_eq!(t!("sidebar.downloads").to_string(), "传输");
    assert_eq!(t!("menu.settings").to_string(), "设置");
    assert_eq!(t!("history.title").to_string(), "History");
}
