rust_i18n::i18n!("locales", fallback = "en");

use rust_i18n::t;

#[test]
fn english_translations_resolve() {
    rust_i18n::set_locale("en");

    assert_eq!(t!("sidebar.downloads").to_string(), "Transfers");
    assert_eq!(t!("menu.settings").to_string(), "Settings");
}
