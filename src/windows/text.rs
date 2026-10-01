pub fn without_subtitle_credit(text: &str) -> &str {
    let end = text
        .trim_end_matches(|c: char| c.is_whitespace() || matches!(c, '.' | ',' | '!' | ';' | '…'));
    let mut words = end.split_whitespace().rev();
    let Some(name) = words.next() else {
        return text;
    };
    let equal = |a: &str, b: &str| a.chars().flat_map(char::to_lowercase).eq(b.chars());
    let valid = equal(name, "johnyx")
        || equal(name, "johnnyx")
        || (equal(name, "x")
            && words
                .next()
                .is_some_and(|w| equal(w, "johny") || equal(w, "johnny")));
    if !valid
        || !words
            .next()
            .is_some_and(|w| equal(w, "vytvořil") || equal(w, "vytvoril"))
    {
        return text;
    }
    let Some(word) = words.next().filter(|w| equal(w, "titulky")) else {
        return text;
    };
    text[..word.as_ptr() as usize - text.as_ptr() as usize].trim_end()
}

#[cfg(test)]
mod tests {
    use super::without_subtitle_credit;
    #[test]
    fn removes_only_known_trailing_credit() {
        assert_eq!(
            without_subtitle_credit("Hotovo. Titulky vytvořil JohnyX."),
            "Hotovo."
        );
        assert_eq!(without_subtitle_credit("TITULKY VYTVOŘIL JOHNNY X!"), "");
        for text in [
            "Titulky vytvořil Petr.",
            "Děkuji za pozornost.",
            "Titulky vytvořil JohnyX. To je chyba.",
            "Řekl jsem JohnyX.",
        ] {
            assert_eq!(without_subtitle_credit(text), text);
        }
        assert_eq!(
            without_subtitle_credit("İstanbul. Titulky vytvořil JohnyX."),
            "İstanbul."
        );
    }
}
