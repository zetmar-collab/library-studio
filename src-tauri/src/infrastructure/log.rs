use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use chrono::Utc;

pub fn write(dir: &Path, level: &str, message: &str) {
    let path = dir.join("biblioteka.log");
    if let Ok(meta) = path.metadata() {
        if meta.len() > 1_000_000 {
            let rotated = dir.join("biblioteka.log.1");
            let _ = std::fs::rename(&path, rotated);
        }
    }
    let line = format!(
        "{} {} {}\n",
        Utc::now().format("%Y-%m-%dT%H:%M:%SZ"),
        level,
        message.replace('\n', " ")
    );
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        if file.write_all(line.as_bytes()).is_err() {
            eprintln!("nie udało się dopisać dziennika biblioteki");
        }
    } else {
        eprintln!("nie udało się otworzyć dziennika biblioteki");
    }
}
