pub fn normalize_isbn(raw: &str) -> String {
    raw.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

pub fn validate_isbn(raw: &str) -> Result<String, String> {
    let isbn = normalize_isbn(raw);
    let ok = match isbn.len() {
        10 => isbn10_ok(&isbn),
        13 => isbn13_ok(&isbn),
        _ => false,
    };
    if ok {
        Ok(isbn)
    } else {
        Err("Nieprawidłowy ISBN.".into())
    }
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn isbn10_to_13(raw: &str) -> Result<String, String> {
    let isbn = validate_isbn(raw)?;
    if isbn.len() == 13 {
        return Ok(isbn);
    }
    let body: String = format!("978{}", &isbn[..9]);
    Ok(format!("{body}{}", isbn13_check(&body)))
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn isbn13_to_10(raw: &str) -> Result<Option<String>, String> {
    let isbn = validate_isbn(raw)?;
    if isbn.len() == 10 {
        return Ok(Some(isbn));
    }
    if !isbn.starts_with("978") {
        return Ok(None);
    }
    let body = &isbn[3..12];
    Ok(Some(format!("{body}{}", isbn10_check(body))))
}

fn isbn10_ok(isbn: &str) -> bool {
    let bytes = isbn.as_bytes();
    if !bytes[..9].iter().all(|b| b.is_ascii_digit()) {
        return false;
    }
    if !(bytes[9].is_ascii_digit() || bytes[9] == b'X') {
        return false;
    }
    isbn10_check(&isbn[..9]) == isbn[9..]
}

fn isbn13_ok(isbn: &str) -> bool {
    isbn.bytes().all(|b| b.is_ascii_digit()) && isbn13_check(&isbn[..12]) == isbn[12..]
}

fn isbn10_check(body9: &str) -> String {
    let sum: u32 = body9
        .bytes()
        .enumerate()
        .map(|(i, b)| (10 - i as u32) * (b - b'0') as u32)
        .sum();
    match (11 - (sum % 11)) % 11 {
        10 => "X".into(),
        n => n.to_string(),
    }
}

fn isbn13_check(body12: &str) -> String {
    let sum: u32 = body12
        .bytes()
        .enumerate()
        .map(|(i, b)| {
            let d = (b - b'0') as u32;
            if i % 2 == 0 { d } else { d * 3 }
        })
        .sum();
    ((10 - (sum % 10)) % 10).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_hyphenated_isbn13() {
        assert_eq!(validate_isbn("978-0-14-032872-1").unwrap(), "9780140328721");
    }

    #[test]
    fn accepts_isbn10_with_x() {
        assert_eq!(validate_isbn("0-8044-2957-X").unwrap(), "080442957X");
    }

    #[test]
    fn rejects_bad_check_digit() {
        assert!(validate_isbn("9780140328720").is_err());
    }

    #[test]
    fn converts_between_10_and_13() {
        let isbn13 = isbn10_to_13("0140328726").unwrap();
        assert_eq!(isbn13, "9780140328721");
        assert_eq!(isbn13_to_10(&isbn13).unwrap().as_deref(), Some("0140328726"));
        assert_eq!(isbn13_to_10("9791234567896").ok().flatten(), None);
    }
}
