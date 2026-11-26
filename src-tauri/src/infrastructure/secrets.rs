pub fn set_secret(account: &str, value: &str) -> Result<(), String> {
    let entry = keyring::Entry::new("biblioteka", account).map_err(|e| e.to_string())?;
    if value.trim().is_empty() {
        let _ = entry.delete_credential();
        return Ok(());
    }
    entry.set_password(value.trim()).map_err(|e| e.to_string())
}

pub fn get_secret(account: &str) -> Result<Option<String>, String> {
    let entry = keyring::Entry::new("biblioteka", account).map_err(|e| e.to_string())?;
    match entry.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(error) => {
            let text = error.to_string().to_lowercase();
            if text.contains("no entry") || text.contains("not found") || text.contains("nie znaleziono") {
                Ok(None)
            } else {
                Err(error.to_string())
            }
        }
    }
}

pub fn has_secret(account: &str) -> bool {
    get_secret(account).ok().flatten().is_some()
}
