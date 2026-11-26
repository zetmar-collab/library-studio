pub fn fold(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        let mapped = match ch {
            'ą' | 'Ą' => 'a',
            'ć' | 'Ć' => 'c',
            'ę' | 'Ę' => 'e',
            'ł' | 'Ł' => 'l',
            'ń' | 'Ń' => 'n',
            'ó' | 'Ó' => 'o',
            'ś' | 'Ś' => 's',
            'ż' | 'Ż' => 'z',
            'ź' | 'Ź' => 'z',
            other => other,
        };
        for lower in mapped.to_lowercase() {
            if lower.is_alphanumeric() {
                out.push(lower);
            } else if lower.is_whitespace() {
                if !out.ends_with(' ') {
                    out.push(' ');
                }
            } else if !out.ends_with(' ') {
                out.push(' ');
            }
        }
    }
    out.trim().to_string()
}

pub fn dedupe_key(title: &str, authors: &[String]) -> String {
    let mut names: Vec<String> = authors.iter().map(|name| fold(name)).filter(|n| !n.is_empty()).collect();
    names.sort();
    names.dedup();
    format!("{}\n{}", fold(title), names.join("|"))
}

pub fn safe_component(value: &str) -> String {
    let forbidden = ['\\', '/', ':', '*', '?', '"', '<', '>', '|'];
    let mut cleaned: String = value
        .chars()
        .map(|ch| if forbidden.contains(&ch) || ch.is_control() { ' ' } else { ch })
        .collect();
    cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    cleaned = cleaned.trim_matches(|ch: char| ch == '.' || ch == ' ').to_string();
    if cleaned.len() > 80 {
        let mut end = 80;
        while !cleaned.is_char_boundary(end) {
            end -= 1;
        }
        cleaned = cleaned[..end].trim().trim_end_matches('.').to_string();
    }
    if cleaned.is_empty() {
        cleaned = "bez nazwy".into();
    }
    let stem = cleaned.split('.').next().unwrap_or("").trim().to_ascii_uppercase();
    if is_reserved_name(&stem) {
        cleaned = format!("{cleaned}_");
    }
    cleaned
}

/// Nazwy urządzeń zarezerwowane w Windows (także z rozszerzeniem, np. „CON.txt”).
pub fn is_reserved_name(stem: &str) -> bool {
    let upper = stem.trim().to_ascii_uppercase();
    if ["CON", "PRN", "AUX", "NUL"].contains(&upper.as_str()) {
        return true;
    }
    let bytes = upper.as_bytes();
    bytes.len() == 4 && (upper.starts_with("COM") || upper.starts_with("LPT")) && (b'1'..=b'9').contains(&bytes[3])
}

pub fn strip_html(value: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for ch in value.chars() {
        match ch {
            '<' => inside = true,
            '>' => inside = false,
            _ if !inside => out.push(ch),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_polish_letters() {
        assert_eq!(fold("Żółć i Łódź"), "zolc i lodz");
    }

    #[test]
    fn keeps_comma_inside_author_key() {
        let key = dedupe_key("Pan Tadeusz", &["Mickiewicz, Adam".into()]);
        assert!(key.contains("mickiewicz adam"));
        assert_eq!(key.matches('\n').count(), 1);
    }

    #[test]
    fn safe_names_drop_windows_reserved_characters() {
        assert_eq!(safe_component("  Tom: 1 / 2?  "), "Tom 1 2");
        assert_eq!(safe_component("CON"), "CON_");
        assert_eq!(safe_component("..."), "bez nazwy");
        assert_eq!(safe_component("COM7"), "COM7_");
        assert_eq!(safe_component("lpt9"), "lpt9_");
        assert_eq!(safe_component("COM10"), "COM10");
    }
}
