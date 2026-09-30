use windows_sys::Win32::Globalization::{GetUserDefaultUILanguage, LCIDToLocaleName};

// Whisper's language codes, kept here so settings never need to load the engine.
pub const LANGUAGES: &[(&str, &str)] = &[
    ("auto", "Detect from speech"),
    ("cs", "Czech"),
    ("sk", "Slovak"),
    ("en", "English"),
    ("de", "German"),
    ("af", "Afrikaans"),
    ("sq", "Albanian"),
    ("am", "Amharic"),
    ("ar", "Arabic"),
    ("hy", "Armenian"),
    ("as", "Assamese"),
    ("az", "Azerbaijani"),
    ("ba", "Bashkir"),
    ("eu", "Basque"),
    ("be", "Belarusian"),
    ("bn", "Bengali"),
    ("bs", "Bosnian"),
    ("br", "Breton"),
    ("bg", "Bulgarian"),
    ("my", "Burmese"),
    ("yue", "Cantonese"),
    ("ca", "Catalan"),
    ("hr", "Croatian"),
    ("da", "Danish"),
    ("nl", "Dutch"),
    ("et", "Estonian"),
    ("fo", "Faroese"),
    ("fi", "Finnish"),
    ("fr", "French"),
    ("gl", "Galician"),
    ("ka", "Georgian"),
    ("el", "Greek"),
    ("gu", "Gujarati"),
    ("ht", "Haitian Creole"),
    ("ha", "Hausa"),
    ("haw", "Hawaiian"),
    ("he", "Hebrew"),
    ("hi", "Hindi"),
    ("hu", "Hungarian"),
    ("zh", "Chinese"),
    ("is", "Icelandic"),
    ("id", "Indonesian"),
    ("it", "Italian"),
    ("ja", "Japanese"),
    ("jw", "Javanese"),
    ("kn", "Kannada"),
    ("kk", "Kazakh"),
    ("km", "Khmer"),
    ("ko", "Korean"),
    ("lo", "Lao"),
    ("la", "Latin"),
    ("lv", "Latvian"),
    ("ln", "Lingala"),
    ("lt", "Lithuanian"),
    ("lb", "Luxembourgish"),
    ("mk", "Macedonian"),
    ("mg", "Malagasy"),
    ("ms", "Malay"),
    ("ml", "Malayalam"),
    ("mt", "Maltese"),
    ("mi", "Maori"),
    ("mr", "Marathi"),
    ("mn", "Mongolian"),
    ("ne", "Nepali"),
    ("no", "Norwegian (Bokmal)"),
    ("nn", "Norwegian (Nynorsk)"),
    ("oc", "Occitan"),
    ("ps", "Pashto"),
    ("fa", "Persian"),
    ("pl", "Polish"),
    ("pt", "Portuguese"),
    ("pa", "Punjabi"),
    ("ro", "Romanian"),
    ("ru", "Russian"),
    ("sa", "Sanskrit"),
    ("sr", "Serbian"),
    ("sn", "Shona"),
    ("sd", "Sindhi"),
    ("si", "Sinhala"),
    ("sl", "Slovenian"),
    ("so", "Somali"),
    ("es", "Spanish"),
    ("su", "Sundanese"),
    ("sw", "Swahili"),
    ("sv", "Swedish"),
    ("tl", "Tagalog"),
    ("tg", "Tajik"),
    ("ta", "Tamil"),
    ("tt", "Tatar"),
    ("te", "Telugu"),
    ("th", "Thai"),
    ("bo", "Tibetan"),
    ("tr", "Turkish"),
    ("tk", "Turkmen"),
    ("uk", "Ukrainian"),
    ("ur", "Urdu"),
    ("uz", "Uzbek"),
    ("vi", "Vietnamese"),
    ("cy", "Welsh"),
    ("yi", "Yiddish"),
    ("yo", "Yoruba"),
];

pub fn from_locale(locale: &str) -> &'static str {
    let primary = locale.split(['-', '_']).next().unwrap_or("");
    let alias = match primary.to_ascii_lowercase().as_str() {
        "nb" => "no",
        "fil" => "tl",
        "jv" => "jw",
        "iw" => "he",
        _ => primary,
    };
    LANGUAGES
        .iter()
        .find(|(code, _)| code.eq_ignore_ascii_case(alias))
        .map_or("auto", |(code, _)| *code)
}

pub fn system_language() -> &'static str {
    // UI/display language, not the keyboard layout or regional number format.
    let mut name = [0u16; 85]; // LOCALE_NAME_MAX_LENGTH
    let len = unsafe {
        LCIDToLocaleName(
            u32::from(GetUserDefaultUILanguage()),
            name.as_mut_ptr(),
            name.len() as i32,
            0,
        )
    };
    if len > 1 {
        from_locale(&String::from_utf16_lossy(&name[..len as usize - 1]))
    } else {
        "auto"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_locales_map_to_supported_speech_languages() {
        assert!(
            LANGUAGES
                .iter()
                .all(|(code, name)| code.is_ascii() && name.is_ascii())
        );
        for (locale, expected) in [
            ("cs-CZ", "cs"),
            ("en-US", "en"),
            ("EN-gb", "en"),
            ("de-DE", "de"),
            ("sk-SK", "sk"),
            ("pt-BR", "pt"),
            ("zh-Hant-TW", "zh"),
            ("nb-NO", "no"),
            ("nn-NO", "nn"),
            ("fil-PH", "tl"),
            ("jv-ID", "jw"),
            ("haw-US", "haw"),
            ("zz-ZZ", "auto"),
            ("", "auto"),
        ] {
            assert_eq!(from_locale(locale), expected, "{locale}");
        }
    }

    #[test]
    #[cfg(feature = "engine")]
    fn settings_cover_every_engine_language_without_loading_a_model() {
        let mut codes = std::collections::HashSet::new();
        for &(code, _) in LANGUAGES.iter().skip(1) {
            assert!(codes.insert(code));
            assert!(whisper_rs::get_lang_id(code).is_some(), "{code}");
        }
        assert_eq!(codes.len(), whisper_rs::get_lang_max_id() as usize + 1);
    }
}
