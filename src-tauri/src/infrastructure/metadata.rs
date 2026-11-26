use std::time::Duration;

use rusqlite::{params, Connection};
use serde_json::Value;

use crate::domain::{fold, strip_html, validate_isbn, CoverCandidate, MetadataHit, MetadataQuery, MetadataSearch, Settings};
use crate::infrastructure::db::{map_sql, now, Database};

const USER_AGENT: &str = "Library Studio/0.3.2 (private catalog; local metadata lookup)";

/// Szuka w serwisach. Blokada bazy jest brana tylko na odczyt i zapis pamięci podręcznej,
/// nie na czas zapytań sieciowych.
pub fn search(db: &Database, settings: &Settings, api_key: Option<&str>, query: &MetadataQuery) -> Result<MetadataSearch, String> {
    let mut hits = Vec::new();
    let mut errors = Vec::new();
    let isbn = query.isbn.as_deref().map(str::trim).filter(|v| !v.is_empty()).map(validate_isbn).transpose()?;
    let title = query.title.clone().unwrap_or_default();
    let author = query.author.clone().unwrap_or_default();
    if settings.open_library {
        match cached_or_fetch(db, "openlibrary", &cache_key("openlibrary", isbn.as_deref(), &title, &author), || {
            open_library(isbn.as_deref(), &title, &author)
        }) {
            Ok(mut found) => hits.append(&mut found),
            Err(error) => errors.push(format!("Open Library: {error}")),
        }
    }
    if settings.national_library {
        match cached_or_fetch(db, "bn", &cache_key("bn", isbn.as_deref(), &title, &author), || {
            national_library(isbn.as_deref(), &title, &author)
        }) {
            Ok(mut found) => hits.append(&mut found),
            Err(error) => errors.push(format!("Biblioteka Narodowa: {error}")),
        }
    }
    if settings.lubimy_czytac {
        match cached_or_fetch(db, "lubimyczytac", &cache_key("lubimyczytac", None, &title, &author), || {
            crate::infrastructure::bookstores::lubimy(&title, &author)
        }) {
            Ok(mut found) => hits.append(&mut found),
            Err(error) => errors.push(format!("LubimyCzytać: {error}")),
        }
    }
    if settings.upolujebooka {
        match cached_or_fetch(db, "upolujebooka", &cache_key("upolujebooka", None, &title, &author), || {
            crate::infrastructure::bookstores::upoluj(&title, &author)
        }) {
            Ok(mut found) => hits.append(&mut found),
            Err(error) => errors.push(format!("UpolujEbooka: {error}")),
        }
    }
    if settings.google_books {
        match api_key.filter(|key| !key.is_empty()) {
            None => errors.push("Google Books: brak klucza API. Dodaj go w ustawieniach.".into()),
            Some(key) => match cached_or_fetch(db, "google", &cache_key("google", isbn.as_deref(), &title, &author), || {
                google_books(isbn.as_deref(), &title, &author, key)
            }) {
                Ok(mut found) => hits.append(&mut found),
                Err(error) => errors.push(format!("Google Books: {error}")),
            },
        }
    }
    hits.dedup_by(|a, b| a.source == b.source && a.external_id == b.external_id);
    hits.sort_by_key(|hit| std::cmp::Reverse(score_hit(hit, &title, &author, isbn.as_deref())));
    Ok(MetadataSearch { hits, errors })
}

fn cached_or_fetch(db: &Database, source: &str, key: &str, fetch: impl FnOnce() -> Result<Vec<MetadataHit>, String>) -> Result<Vec<MetadataHit>, String> {
    if let Some(payload) = db.with(|conn| cache_get(conn, source, key))? {
        if let Ok(hits) = serde_json::from_str::<Vec<MetadataHit>>(&payload) {
            return Ok(hits);
        }
    }
    let hits = fetch()?;
    let payload = serde_json::to_string(&hits).map_err(|e| e.to_string())?;
    db.with(|conn| cache_put(conn, source, key, &payload))?;
    Ok(hits)
}

fn cache_key(source: &str, isbn: Option<&str>, title: &str, author: &str) -> String {
    crate::infrastructure::files::hash_bytes(format!("{source}|{}|{}|{}", isbn.unwrap_or(""), fold(title), fold(author)).as_bytes())
}

fn cache_get(conn: &Connection, source: &str, key: &str) -> Result<Option<String>, String> {
    let row = conn
        .query_row(
            "SELECT payload_json, expires_at FROM metadata_cache WHERE source=?1 AND query_hash=?2 ORDER BY fetched_at DESC LIMIT 1",
            params![source, key],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .ok();
    if let Some((payload, expires)) = row {
        if expires > now() {
            return Ok(Some(payload));
        }
    }
    Ok(None)
}

fn cache_put(conn: &Connection, source: &str, key: &str, payload: &str) -> Result<(), String> {
    let stamp = now();
    let expires = (chrono::Utc::now() + chrono::Duration::days(7)).format("%Y-%m-%dT%H:%M:%SZ").to_string();
    conn.execute(
        "INSERT INTO metadata_cache (id, source, query_hash, payload_json, fetched_at, expires_at) VALUES (?1,?2,?3,?4,?5,?6)",
        params![uuid::Uuid::new_v4().to_string(), source, key, payload, stamp, expires],
    )
    .map_err(map_sql)?;
    Ok(())
}

fn client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())
}

fn open_library(isbn: Option<&str>, title: &str, author: &str) -> Result<Vec<MetadataHit>, String> {
    let mut hits = Vec::new();
    if let Some(isbn) = isbn {
        if let Some(hit) = open_library_isbn(isbn)? {
            hits.push(hit);
        }
    }
    let short = query_title(title);
    let last = surname(author);
    if !short.is_empty() || !last.is_empty() {
        let mut query = Vec::new();
        if !short.is_empty() {
            query.push(format!("title:({short})"));
        }
        if !last.is_empty() {
            query.push(format!("author:{last}"));
        }
        let url = format!(
            "https://openlibrary.org/search.json?q={}&limit=8&fields=key,title,subtitle,author_name,first_publish_year,cover_i,publisher,isbn,number_of_pages_median,language",
            urlencoding(&query.join(" "))
        );
        let value = get_json(&url)?;
        if let Some(docs) = value.get("docs").and_then(|docs| docs.as_array()) {
            for doc in docs {
                hits.push(hit_from_open_library_doc(doc));
            }
        }
    }
    Ok(hits)
}

fn open_library_isbn(isbn: &str) -> Result<Option<MetadataHit>, String> {
    let edition = get_json(&format!("https://openlibrary.org/isbn/{isbn}.json"))?;
    if edition.get("error").is_some() || edition.get("title").is_none() {
        return Ok(None);
    }
    let mut authors = Vec::new();
    if let Some(list) = edition.get("authors").and_then(|v| v.as_array()) {
        for author in list.iter().take(5) {
            if let Some(key) = author.get("key").and_then(|v| v.as_str()) {
                let payload = get_json(&format!("https://openlibrary.org{key}.json"))?;
                if let Some(name) = payload.get("name").and_then(|v| v.as_str()) {
                    authors.push(name.to_string());
                }
            }
        }
    }
    let mut description = text_value(edition.get("description"));
    if description.is_none() {
        if let Some(work) = edition.get("works").and_then(|v| v.as_array()).and_then(|v| v.first()).and_then(|v| v.get("key")).and_then(|v| v.as_str()) {
            let work_json = get_json(&format!("https://openlibrary.org{work}.json"))?;
            description = text_value(work_json.get("description"));
        }
    }
    let isbn13 = first_string(edition.get("isbn_13")).or_else(|| if isbn.len() == 13 { Some(isbn.to_string()) } else { None });
    let isbn10 = first_string(edition.get("isbn_10"));
    let cover_id = edition.get("covers").and_then(|v| v.as_array()).and_then(|v| v.iter().find_map(|n| n.as_i64())).filter(|id| *id > 0);
    Ok(Some(MetadataHit {
        source: "openlibrary".into(),
        external_id: edition.get("key").and_then(|v| v.as_str()).unwrap_or(isbn).to_string(),
        title: edition.get("title").and_then(|v| v.as_str()).unwrap_or("Bez tytułu").to_string(),
        subtitle: edition.get("subtitle").and_then(|v| v.as_str()).map(str::to_string),
        authors,
        publisher: first_string(edition.get("publishers")),
        publication_year: year_from(edition.get("publish_date").and_then(|v| v.as_str())),
        publication_date: edition.get("publish_date").and_then(|v| v.as_str()).map(str::to_string),
        language: edition.get("languages").and_then(|v| v.as_array()).and_then(|v| v.first()).and_then(|v| v.get("key")).and_then(|v| v.as_str()).map(|key| key.rsplit('/').next().unwrap_or(key).to_string()),
        description,
        page_count: edition.get("number_of_pages").and_then(|v| v.as_i64()),
        isbn10,
        isbn13,
        cover_url: cover_id.map(|id| format!("https://covers.openlibrary.org/b/id/{id}-L.jpg")),
        series_name: None,
        genres: string_list(edition.get("subjects")),
        rating: None,
        info_url: edition.get("key").and_then(|v| v.as_str()).map(|key| format!("https://openlibrary.org{key}")),
    }))
}

fn hit_from_open_library_doc(doc: &Value) -> MetadataHit {
    let cover = doc.get("cover_i").and_then(|v| v.as_i64());
    let isbns = string_list(doc.get("isbn"));
    let isbn13 = isbns.iter().find(|v| v.len() == 13).cloned();
    let isbn10 = isbns.iter().find(|v| v.len() == 10).cloned();
    MetadataHit {
        source: "openlibrary".into(),
        external_id: doc.get("key").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        title: doc.get("title").and_then(|v| v.as_str()).unwrap_or("Bez tytułu").to_string(),
        subtitle: doc.get("subtitle").and_then(|v| v.as_str()).map(str::to_string),
        authors: string_list(doc.get("author_name")),
        publisher: first_string(doc.get("publisher")),
        publication_year: doc.get("first_publish_year").and_then(|v| v.as_i64()),
        publication_date: doc.get("first_publish_year").and_then(|v| v.as_i64()).map(|y| y.to_string()),
        language: first_string(doc.get("language")),
        description: None,
        page_count: doc.get("number_of_pages_median").and_then(|v| v.as_i64()),
        isbn10,
        isbn13,
        cover_url: cover.map(|id| format!("https://covers.openlibrary.org/b/id/{id}-L.jpg")),
        series_name: None,
        genres: Vec::new(),
        rating: None,
        info_url: doc.get("key").and_then(|v| v.as_str()).map(|key| format!("https://openlibrary.org{key}")),
    }
}

fn google_books(isbn: Option<&str>, title: &str, author: &str, key: &str) -> Result<Vec<MetadataHit>, String> {
    let mut q = Vec::new();
    if let Some(isbn) = isbn {
        q.push(format!("isbn:{isbn}"));
    }
    let short = query_title(title);
    let last = surname(author);
    if isbn.is_none() && !short.is_empty() {
        q.push(format!("intitle:{short}"));
    }
    if isbn.is_none() && !last.is_empty() {
        q.push(format!("inauthor:{last}"));
    }
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let url = format!(
        "https://www.googleapis.com/books/v1/volumes?q={}&maxResults=8&printType=books&key={}",
        urlencoding(&q.join(" ")),
        urlencoding(key)
    );
    let value = get_json(&url)?;
    if let Some(message) = value.get("error").and_then(|e| e.get("message")).and_then(|m| m.as_str()) {
        return Err(message.to_string());
    }
    let mut hits = Vec::new();
    if let Some(items) = value.get("items").and_then(|v| v.as_array()) {
        for item in items {
            hits.push(hit_from_google(item));
        }
    }
    Ok(hits)
}

pub fn hit_from_google(item: &Value) -> MetadataHit {
    let info = item.get("volumeInfo").cloned().unwrap_or(Value::Null);
    let ids = info.get("industryIdentifiers").and_then(|v| v.as_array());
    let isbn = |kind: &str| {
        ids.and_then(|list| {
            list.iter().find_map(|id| {
                if id.get("type").and_then(|v| v.as_str()) == Some(kind) {
                    id.get("identifier").and_then(|v| v.as_str()).map(str::to_string)
                } else {
                    None
                }
            })
        })
    };
    MetadataHit {
        source: "google".into(),
        external_id: item.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        title: info.get("title").and_then(|v| v.as_str()).unwrap_or("Bez tytułu").to_string(),
        subtitle: info.get("subtitle").and_then(|v| v.as_str()).map(str::to_string),
        authors: string_list(info.get("authors")),
        publisher: info.get("publisher").and_then(|v| v.as_str()).map(str::to_string),
        publication_year: year_from(info.get("publishedDate").and_then(|v| v.as_str())),
        publication_date: info.get("publishedDate").and_then(|v| v.as_str()).map(str::to_string),
        language: info.get("language").and_then(|v| v.as_str()).map(str::to_string),
        description: info.get("description").and_then(|v| v.as_str()).map(strip_html),
        page_count: info.get("pageCount").and_then(|v| v.as_i64()),
        isbn10: isbn("ISBN_10"),
        isbn13: isbn("ISBN_13"),
        cover_url: google_cover(&info),
        series_name: None,
        genres: string_list(info.get("categories")),
        rating: info.get("averageRating").and_then(|v| v.as_f64()),
        info_url: info.get("infoLink").and_then(|v| v.as_str()).map(str::to_string),
    }
}

fn national_library(isbn: Option<&str>, title: &str, author: &str) -> Result<Vec<MetadataHit>, String> {
    if isbn.is_none() && title.trim().is_empty() && author.trim().is_empty() {
        return Ok(Vec::new());
    }
    let mut url = String::from("https://data.bn.org.pl/api/institutions/bibs.json?limit=8");
    if let Some(isbn) = isbn {
        url.push_str("&isbnIssn=");
        url.push_str(&urlencoding(isbn));
    }
    let short = query_title(title);
    let last = surname(author);
    if isbn.is_none() && !short.is_empty() {
        url.push_str("&title=");
        url.push_str(&urlencoding(&short));
    }
    if isbn.is_none() && !last.is_empty() {
        url.push_str("&author=");
        url.push_str(&urlencoding(&last));
    }
    let value = get_json(&url)?;
    let mut hits = Vec::new();
    if let Some(bibs) = value.get("bibs").and_then(|v| v.as_array()) {
        for bib in bibs {
            if bib.get("deleted").and_then(|v| v.as_bool()) == Some(true) {
                continue;
            }
            if let Some(hit) = hit_from_bn(bib) {
                hits.push(hit);
            }
        }
    }
    Ok(hits)
}

pub fn hit_from_bn(bib: &Value) -> Option<MetadataHit> {
    let fields = bib.pointer("/marc/fields").and_then(|v| v.as_array());
    let title = marc_subfield(fields, "245", "a")
        .map(|text| text.trim().trim_end_matches([' ', '/', ':', ';', '=', '.']).trim().to_string())
        .filter(|text| !text.is_empty())
        .or_else(|| {
            bib.get("title").and_then(|v| v.as_str()).map(|text| text.split('/').next().unwrap_or(text).trim().to_string())
        })?;
    let authors = marc_people(fields);
    let authors = if authors.is_empty() {
        bib.get("author")
            .and_then(|v| v.as_str())
            .map(|text| text.split('.').next().unwrap_or(text).trim().to_string())
            .filter(|text| !text.is_empty())
            .into_iter()
            .collect()
    } else {
        authors
    };
    let isbns: Vec<String> = marc_subfields(fields, "020", "a")
        .into_iter()
        .map(|value| crate::domain::normalize_isbn(&value))
        .filter(|value| value.len() == 10 || value.len() == 13)
        .collect();
    let isbn13 = isbns.iter().find(|value| value.len() == 13).cloned();
    let isbn10 = isbns.iter().find(|value| value.len() == 10).cloned();
    let id = bib.get("id").map(|v| v.to_string().trim_matches('"').to_string()).unwrap_or_default();
    Some(MetadataHit {
        source: "bn".into(),
        external_id: id.clone(),
        title,
        subtitle: marc_subfield(fields, "245", "b"),
        authors,
        publisher: marc_subfield(fields, "260", "b").map(|text| text.trim().trim_end_matches(',').trim().to_string()).or_else(|| bib.get("publisher").and_then(|v| v.as_str()).map(|text| text.trim().trim_end_matches(',').to_string())),
        publication_year: bib.get("publicationYear").and_then(|v| v.as_str()).and_then(|text| text.parse().ok()),
        publication_date: bib.get("publicationYear").and_then(|v| v.as_str()).map(str::to_string),
        language: bn_language(bib.get("language").and_then(|v| v.as_str())),
        description: None,
        page_count: marc_subfield(fields, "300", "a").and_then(|text| text.chars().skip_while(|ch| !ch.is_ascii_digit()).take_while(|ch| ch.is_ascii_digit()).collect::<String>().parse().ok()),
        isbn10,
        isbn13,
        cover_url: None,
        series_name: marc_subfield(fields, "490", "a"),
        genres: marc_subfields(fields, "655", "a"),
        rating: None,
        info_url: if id.is_empty() { None } else { Some(format!("https://data.bn.org.pl/api/institutions/bibs.json?id={id}")) },
    })
}

fn marc_people(fields: Option<&Vec<Value>>) -> Vec<String> {
    let Some(fields) = fields else { return Vec::new() };
    let mut names = Vec::new();
    for field in fields {
        let object = field.as_object();
        let Some(object) = object else { continue };
        if object.get("100").is_none() && object.get("700").is_none() {
            continue;
        }
        let body = object.get("100").or_else(|| object.get("700")).and_then(|v| v.as_object());
        let Some(body) = body else { continue };
        let role = subfields_of(body, "e").join(" ").to_lowercase();
        if !role.is_empty() && !role.contains("autor") {
            continue;
        }
        if let Some(name) = subfields_of(body, "a").into_iter().next() {
            if !names.iter().any(|existing: &String| existing == &name) {
                names.push(name);
            }
        }
    }
    names
}

fn marc_subfield(fields: Option<&Vec<Value>>, tag: &str, code: &str) -> Option<String> {
    marc_subfields(fields, tag, code).into_iter().next()
}

fn marc_subfields(fields: Option<&Vec<Value>>, tag: &str, code: &str) -> Vec<String> {
    let Some(fields) = fields else { return Vec::new() };
    let mut values = Vec::new();
    for field in fields {
        let Some(body) = field.get(tag).and_then(|v| v.as_object()) else { continue };
        values.extend(subfields_of(body, code));
    }
    values
}

fn subfields_of(body: &serde_json::Map<String, Value>, code: &str) -> Vec<String> {
    body.get("subfields")
        .and_then(|v| v.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|item| item.get(code).and_then(|v| v.as_str()).map(str::trim).filter(|v| !v.is_empty()).map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn bn_language(value: Option<&str>) -> Option<String> {
    let text = value?.to_lowercase();
    if text.contains("polsk") {
        Some("pl".into())
    } else if text.contains("angiel") {
        Some("en".into())
    } else if text.contains("niemiec") {
        Some("de".into())
    } else if text.contains("franc") {
        Some("fr".into())
    } else {
        None
    }
}

/// Zbiera wyniki ze wszystkich włączonych źródeł i składa z nich jeden: najlepiej dopasowany wynik
/// jest podstawą, a jego puste pola (opis, okładka, wydawca…) uzupełniają kolejne pasujące wyniki.
pub fn best_hit(settings: &Settings, api_key: Option<&str>, title: &str, author: &str, isbn: Option<&str>) -> Option<MetadataHit> {
    let isbn = isbn.and_then(|value| validate_isbn(value).ok());
    let mut hits = Vec::new();
    let mut collect = |rows: Result<Vec<MetadataHit>, String>| {
        if let Ok(mut rows) = rows {
            hits.append(&mut rows);
        }
    };
    if settings.lubimy_czytac {
        collect(crate::infrastructure::bookstores::lubimy(title, author));
    }
    if settings.national_library {
        collect(national_library(isbn.as_deref(), title, author));
        // Szukanie po ISBN pomija tytuł, więc gdy ISBN nic nie da, pytamy jeszcze o tytuł i autora.
        if isbn.is_some() {
            collect(national_library(None, title, author));
        }
    }
    if settings.open_library {
        collect(open_library(isbn.as_deref(), title, author));
    }
    if settings.upolujebooka {
        collect(crate::infrastructure::bookstores::upoluj(title, author));
    }
    if settings.google_books {
        if let Some(key) = api_key.filter(|key| !key.is_empty()) {
            collect(google_books(isbn.as_deref(), title, author, key));
            if isbn.is_some() {
                collect(google_books(None, title, author, key));
            }
        }
    }
    merge_hits(hits, title, author, isbn.as_deref())
}

pub(crate) fn merge_hits(hits: Vec<MetadataHit>, title: &str, author: &str, isbn: Option<&str>) -> Option<MetadataHit> {
    let mut accepted: Vec<(i32, MetadataHit)> = hits
        .into_iter()
        .map(|hit| (score_hit(&hit, title, author, isbn), hit))
        .filter(|(score, _)| *score >= 70)
        .collect();
    // Przy remisie wygrywa wynik z pełniejszym opisem.
    accepted.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| b.1.description.as_ref().map_or(0, String::len).cmp(&a.1.description.as_ref().map_or(0, String::len)))
    });
    let mut rest: Vec<MetadataHit> = accepted.into_iter().map(|(_, hit)| hit).collect();
    if rest.is_empty() {
        return None;
    }
    let mut best = rest.remove(0);
    // Okładka innego wydania tej samej książki jest lepsza niż żadna.
    let fallback_cover = rest.iter().find_map(|hit| hit.cover_url.clone().filter(|url| !url.is_empty()));
    for other in rest {
        // Inny ISBN to inne wydanie: z niego bierzemy tylko opis, gatunki i serię.
        let other_isbn = other.isbn13.clone().or(other.isbn10.clone());
        let best_isbn = best.isbn13.clone().or(best.isbn10.clone());
        let same_edition = other_isbn.is_none() || best_isbn.is_none() || other_isbn == best_isbn;
        fill(&mut best.description, other.description.clone());
        fill(&mut best.series_name, other.series_name.clone());
        if best.genres.is_empty() {
            best.genres = other.genres.clone();
        }
        if same_edition {
            fill(&mut best.subtitle, other.subtitle);
            fill(&mut best.publisher, other.publisher);
            fill(&mut best.language, other.language);
            fill(&mut best.cover_url, other.cover_url);
            fill(&mut best.isbn13, other.isbn13);
            fill(&mut best.isbn10, other.isbn10);
            fill(&mut best.publication_date, other.publication_date);
            best.publication_year = best.publication_year.or(other.publication_year);
            best.page_count = best.page_count.or(other.page_count);
        }
        best.rating = best.rating.or(other.rating);
    }
    fill(&mut best.cover_url, fallback_cover);
    Some(best)
}

fn fill(slot: &mut Option<String>, value: Option<String>) {
    if slot.as_ref().is_none_or(|v| v.trim().is_empty()) {
        if let Some(value) = value.filter(|v| !v.trim().is_empty()) {
            *slot = Some(value);
        }
    }
}

/// Pierwszy autor w kolejności „Imię Nazwisko”, bez tytułów i dopisków w nawiasach.
/// „Napoleon Hill, Ross Cornwell (Editor)” → „Napoleon Hill”, „Lem, Stanisław” → „Stanisław Lem”.
pub(crate) fn query_author(author: &str) -> String {
    let mut text = String::new();
    let mut depth = 0_i32;
    for ch in author.chars() {
        match ch {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = (depth - 1).max(0),
            _ if depth == 0 => text.push(ch),
            _ => {}
        }
    }
    let first = text.split([';', '&']).next().unwrap_or("");
    let first = first.split(" i ").next().unwrap_or("").split(" and ").next().unwrap_or("").trim();
    let parts: Vec<&str> = first.split(',').map(str::trim).filter(|p| !p.is_empty()).collect();
    let name = match parts.as_slice() {
        // „Nazwisko, Imię”: przed przecinkiem jedno słowo, po nim imię (najwyżej z inicjałem).
        [last, given] if !last.contains(' ') && given.split_whitespace().count() <= 2 => format!("{given} {last}"),
        [one, ..] => (*one).to_string(),
        [] => String::new(),
    };
    name.split_whitespace()
        .filter(|word| !matches!(fold(word).as_str(), "dr" | "prof" | "ks" | "mgr" | "inz" | "hab" | "phd" | "md" | "jr" | "sr"))
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn surname(author: &str) -> String {
    query_author(author).split_whitespace().last().unwrap_or("").to_string()
}

/// Tytuł bez podtytułu: „Myśl i bogać się. Wersja oryginalna…” → „Myśl i bogać się”.
pub(crate) fn query_title(title: &str) -> String {
    let title = title.trim();
    for separator in [": ", ". ", " - ", " – ", " — ", ":"] {
        if let Some(index) = title.find(separator) {
            let head = title[..index].trim();
            if head.chars().count() >= 3 {
                return head.to_string();
            }
        }
    }
    title.trim_end_matches(['.', ' ']).to_string()
}

pub fn fill_empty(draft: &mut crate::domain::BookDraft, hit: &MetadataHit, keep_cover: bool) {
    if draft.subtitle.as_ref().map(|v| v.trim().is_empty()).unwrap_or(true) {
        draft.subtitle = hit.subtitle.clone();
    }
    if draft.publisher.as_ref().map(|v| v.trim().is_empty()).unwrap_or(true) {
        draft.publisher = hit.publisher.clone();
    }
    if draft.publication_year.is_none() {
        draft.publication_year = hit.publication_year;
    }
    if draft.language.as_ref().map(|v| v.trim().is_empty()).unwrap_or(true) {
        draft.language = hit.language.clone();
    }
    if draft.description.as_ref().map(|v| v.trim().is_empty()).unwrap_or(true) {
        draft.description = hit.description.clone();
    }
    if draft.series_name.as_ref().map(|v| v.trim().is_empty()).unwrap_or(true) {
        draft.series_name = hit.series_name.clone();
    }
    if draft.genres.iter().all(|v| v.trim().is_empty()) {
        draft.genres = hit.genres.clone();
    }
    if draft.isbn13.as_ref().map(|v| v.trim().is_empty()).unwrap_or(true) {
        draft.isbn13 = hit.isbn13.clone();
    }
    if draft.isbn10.as_ref().map(|v| v.trim().is_empty()).unwrap_or(true) {
        draft.isbn10 = hit.isbn10.clone();
    }
    if draft.page_count.is_none() {
        draft.page_count = hit.page_count;
    }
    if draft.average_rating.is_none() {
        draft.average_rating = hit.rating;
    }
    if !keep_cover && draft.cover_url.as_ref().map(|v| v.trim().is_empty()).unwrap_or(true) {
        draft.cover_url = hit.cover_url.clone();
    }
    if draft.metadata_source.as_ref().map(|v| v.trim().is_empty() || v == "manual").unwrap_or(true) {
        draft.metadata_source = Some(hit.source.clone());
        draft.external_id = Some(hit.external_id.clone());
    }
}

fn score_hit(hit: &MetadataHit, title: &str, author: &str, isbn: Option<&str>) -> i32 {
    let mut score = 0;
    if let Some(isbn) = isbn {
        if hit.isbn13.as_deref() == Some(isbn) || hit.isbn10.as_deref() == Some(isbn) {
            score += 80;
        }
    }
    let found = fold(&hit.title);
    let found_short = fold(&query_title(&hit.title));
    let wanted = fold(title);
    let wanted_short = fold(&query_title(title));
    let starts = |long: &str, short: &str| !short.is_empty() && (long == short || long.starts_with(&format!("{short} ")));
    // Oba tytuły mają podtytuł i są to różne podtytuły: to raczej inna książka („… Książka ćwiczeń”).
    let other_subtitle = found != found_short && wanted != wanted_short && found != wanted;
    if !wanted.is_empty() && found == wanted {
        score += 45;
    } else if !wanted.is_empty() && (found_short == wanted_short || starts(&found, &wanted_short) || starts(&wanted, &found_short)) {
        score += if other_subtitle { 20 } else { 40 };
    } else if !wanted_short.is_empty() && (found.contains(&wanted_short) || wanted.contains(&found_short)) {
        score += 10;
    }
    let last = fold(&surname(author));
    let found_authors = fold(&hit.authors.join(" "));
    if !last.is_empty() && found_authors.split_whitespace().any(|word| word == last) {
        score += 30;
    }
    if hit.cover_url.as_ref().is_some_and(|url| !url.is_empty()) {
        score += 5;
    }
    if hit.description.as_ref().is_some_and(|text| text.len() > 40) {
        score += 5;
    }
    score
}

pub fn search_covers(settings: &Settings, api_key: Option<&str>, query: &MetadataQuery) -> Result<Vec<CoverCandidate>, String> {
    let isbn = query.isbn.as_deref().map(str::trim).filter(|v| !v.is_empty()).map(validate_isbn).transpose()?;
    let title = query.title.clone().unwrap_or_default();
    let author = query.author.clone().unwrap_or_default();
    if isbn.is_none() && title.trim().is_empty() && author.trim().is_empty() {
        return Err("Wpisz tytuł, autora albo ISBN.".into());
    }
    let mut covers = Vec::new();
    if settings.open_library {
        if let Ok(hits) = open_library(isbn.as_deref(), &title, &author) {
            for hit in hits {
                push_cover(&mut covers, &hit);
            }
        }
    }
    if settings.google_books {
        if let Some(key) = api_key.filter(|key| !key.is_empty()) {
            if let Ok(hits) = google_books(isbn.as_deref(), &title, &author, key) {
                for hit in hits {
                    push_cover(&mut covers, &hit);
                }
            }
        }
    }
    if settings.lubimy_czytac {
        if let Ok(hits) = crate::infrastructure::bookstores::lubimy(&title, &author) {
            for hit in hits {
                push_cover(&mut covers, &hit);
            }
        }
    }
    if settings.upolujebooka {
        if let Ok(hits) = crate::infrastructure::bookstores::upoluj(&title, &author) {
            for hit in hits {
                push_cover(&mut covers, &hit);
            }
        }
    }
    covers.truncate(12);
    Ok(covers)
}

fn push_cover(covers: &mut Vec<CoverCandidate>, hit: &MetadataHit) {
    let Some(url) = hit.cover_url.clone().filter(|url| !url.is_empty()) else { return };
    if covers.iter().any(|cover| cover.url == url) {
        return;
    }
    covers.push(CoverCandidate {
        source: hit.source.clone(),
        title: hit.title.clone(),
        authors: hit.authors.clone(),
        url,
    });
}

fn google_cover(info: &Value) -> Option<String> {
    let links = info.get("imageLinks")?;
    for key in ["extraLarge", "large", "medium", "small", "thumbnail", "smallThumbnail"] {
        if let Some(url) = links.get(key).and_then(|v| v.as_str()) {
            return Some(larger_google_cover(url));
        }
    }
    None
}

fn larger_google_cover(url: &str) -> String {
    let url = url.replace("http://", "https://");
    url.replace("zoom=1", "zoom=3")
}

fn get_json(url: &str) -> Result<Value, String> {
    let response = client()?.get(url).send().map_err(|e| e.to_string())?;
    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Ok(Value::Null);
    }
    if !status.is_success() {
        return Err(format!("odpowiedź {status}"));
    }
    response.json().map_err(|e| e.to_string())
}

const COVER_LIMIT: usize = 8_000_000;

/// Sprawdza adres okładki i zwraca adres IP, z którym wolno się połączyć: tylko https i tylko hosty,
/// które nie prowadzą do sieci lokalnej. Połączenie idzie potem dokładnie na ten adres (bez ponownego
/// pytania DNS), więc domena nie może w międzyczasie „przeskoczyć” na 127.0.0.1 czy 192.168.x.x.
fn cover_target(url: &reqwest::Url) -> Result<(String, std::net::SocketAddr), String> {
    if url.scheme() != "https" {
        return Err("Okładka musi mieć adres https.".into());
    }
    let host = url.host_str().unwrap_or("").trim_matches(['[', ']']).to_string();
    if host.is_empty() || host.eq_ignore_ascii_case("localhost") || host.ends_with(".local") || host.ends_with(".localhost") {
        return Err("Ten adres okładki jest zablokowany.".into());
    }
    let port = url.port_or_known_default().unwrap_or(443);
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        return if ip_is_private(ip) { Err("Ten adres okładki jest zablokowany.".into()) } else { Ok((host, std::net::SocketAddr::new(ip, port))) };
    }
    use std::net::ToSocketAddrs;
    let addresses: Vec<_> = (host.as_str(), port).to_socket_addrs().map_err(|_| "Nie udało się pobrać okładki.".to_string())?.collect();
    if addresses.is_empty() || addresses.iter().any(|address| ip_is_private(address.ip())) {
        return Err("Ten adres okładki jest zablokowany.".into());
    }
    Ok((host, addresses[0]))
}

pub fn download_cover(url: &str) -> Result<Vec<u8>, String> {
    let mut current = reqwest::Url::parse(url).map_err(|_| "Adres okładki jest nieprawidłowy.".to_string())?;
    // Przekierowania obsługujemy sami: każdy kolejny adres przechodzi tę samą kontrolę i to samo przypięcie IP.
    for _ in 0..5 {
        let (host, address) = cover_target(&current)?;
        let client = reqwest::blocking::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::none())
            .resolve(&host, address)
            .build()
            .map_err(|e| e.to_string())?;
        let mut response = client.get(current.clone()).send().map_err(|e| e.to_string())?;
        if response.status().is_redirection() {
            let location = response.headers().get(reqwest::header::LOCATION).and_then(|v| v.to_str().ok()).ok_or_else(|| "Nie udało się pobrać okładki.".to_string())?;
            current = current.join(location).map_err(|_| "Nie udało się pobrać okładki.".to_string())?;
            continue;
        }
        if !response.status().is_success() {
            return Err("Nie udało się pobrać okładki.".into());
        }
        if response.content_length().is_some_and(|length| length as usize > COVER_LIMIT) {
            return Err("Okładka jest za duża.".into());
        }
        let bytes = crate::infrastructure::files::read_limited(&mut response, COVER_LIMIT).map_err(|_| "Okładka jest za duża.".to_string())?;
        if image_type(&bytes).is_none() {
            return Err("Ten adres nie wskazuje na obraz okładki.".into());
        }
        return Ok(bytes);
    }
    Err("Za dużo przekierowań przy pobieraniu okładki.".into())
}

pub fn cover_data_url(bytes: &[u8]) -> String {
    let mime = image_type(bytes).unwrap_or("image/jpeg");
    format!("data:{mime};base64,{}", base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes))
}

pub fn image_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        Some("image/png")
    } else if bytes.starts_with(b"GIF8") {
        Some("image/gif")
    } else if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

fn ip_is_private(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(ip) => ip.is_private() || ip.is_loopback() || ip.is_link_local() || ip.is_unspecified() || ip.is_broadcast() || (ip.octets()[0] == 100 && (ip.octets()[1] & 0xc0) == 64),
        std::net::IpAddr::V6(ip) => {
            if let Some(v4) = ip.to_ipv4_mapped() {
                return ip_is_private(std::net::IpAddr::V4(v4));
            }
            ip.is_loopback() || ip.is_unspecified() || (ip.segments()[0] & 0xfe00) == 0xfc00 || (ip.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}

fn text_value(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(strip_html(text)),
        Value::Object(object) => object.get("value").and_then(|v| v.as_str()).map(strip_html),
        _ => None,
    }
}

fn first_string(value: Option<&Value>) -> Option<String> {
    string_list(value).into_iter().next()
}

fn string_list(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(|v| v.as_array())
        .map(|list| list.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

fn year_from(value: Option<&str>) -> Option<i64> {
    let text = value?;
    let digits: String = text.chars().take(4).collect();
    digits.parse().ok()
}

pub(crate) fn urlencoding(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_google_volume() {
        let hit = hit_from_google(&json!({
            "id": "abc",
            "volumeInfo": {
                "title": "Pan Tadeusz",
                "authors": ["Adam Mickiewicz"],
                "publishedDate": "1834-06",
                "description": "<p>Epopeja</p>",
                "industryIdentifiers": [{"type": "ISBN_13", "identifier": "9781234567897"}]
            }
        }));
        assert_eq!(hit.title, "Pan Tadeusz");
        assert_eq!(hit.publication_year, Some(1834));
        assert_eq!(hit.description.as_deref(), Some("Epopeja"));
        assert_eq!(hit.isbn13.as_deref(), Some("9781234567897"));
    }

    #[test]
    fn parses_national_library_record() {
        let hit = hit_from_bn(&json!({
            "id": 5382027,
            "language": "polski",
            "publicationYear": "2014",
            "title": "Ostatnie życzenie / Wiedźmin",
            "marc": {"fields": [
                {"100": {"subfields": [{"a": "Sapkowski, Andrzej"}, {"e": "Autor"}]}},
                {"245": {"subfields": [{"a": "Ostatnie życzenie /"}]}},
                {"260": {"subfields": [{"b": "SuperNOWA,"}]}},
                {"300": {"subfields": [{"a": "330 stron"}]}},
                {"020": {"subfields": [{"a": "978-83-7578-063-5"}]}},
                {"490": {"subfields": [{"a": "Wiedźmin"}]}},
                {"655": {"subfields": [{"a": "Fantasy"}]}}
            ]}
        })).unwrap();
        assert_eq!(hit.title, "Ostatnie życzenie");
        assert_eq!(hit.authors, vec!["Sapkowski, Andrzej".to_string()]);
        assert_eq!(hit.isbn13.as_deref(), Some("9788375780635"));
        assert_eq!(hit.publisher.as_deref(), Some("SuperNOWA"));
        assert_eq!(hit.publication_year, Some(2014));
        assert_eq!(hit.page_count, Some(330));
        assert_eq!(hit.series_name.as_deref(), Some("Wiedźmin"));
        assert_eq!(hit.language.as_deref(), Some("pl"));
        assert_eq!(hit.source, "bn");
    }

    #[test]
    fn prefers_same_title_and_author() {
        let good = MetadataHit {
            source: "openlibrary".into(),
            external_id: "1".into(),
            title: "Pan Tadeusz".into(),
            subtitle: None,
            authors: vec!["Mickiewicz, Adam".into()],
            publisher: None,
            publication_year: None,
            publication_date: None,
            language: None,
            description: None,
            page_count: None,
            isbn10: None,
            isbn13: None,
            cover_url: Some("https://covers.openlibrary.org/b/id/1-L.jpg".into()),
            series_name: None,
            genres: vec![],
            rating: None,
            info_url: None,
        };
        let weak = MetadataHit { title: "Pan".into(), authors: vec!["Inny".into()], cover_url: None, ..good.clone() };
        assert!(score_hit(&good, "Pan Tadeusz", "Mickiewicz, Adam", None) >= 70);
        assert!(score_hit(&weak, "Pan Tadeusz", "Mickiewicz, Adam", None) < 70);
    }

    #[test]
    fn rejects_local_cover_address() {
        assert!(download_cover("http://covers.openlibrary.org/b/id/1-L.jpg").is_err());
        assert!(download_cover("https://127.0.0.1/cover.jpg").is_err());
        assert!(download_cover("https://localhost/cover.jpg").is_err());
        assert!(download_cover("https://[::1]/cover.jpg").is_err());
        assert!(download_cover("https://[::ffff:192.168.1.1]/cover.jpg").is_err());
        assert!(download_cover("https://10.0.0.5/cover.jpg").is_err());
    }

    #[test]
    fn parses_open_library_description_object() {
        let value = json!({"type": "/type/text", "value": "Opis"});
        assert_eq!(text_value(Some(&value)).as_deref(), Some("Opis"));
    }
}


#[cfg(test)]
mod query_tests {
    use super::*;

    #[test]
    fn cleans_author_and_title_for_queries() {
        assert_eq!(query_author("Dr Wayne W. Dyer"), "Wayne W. Dyer");
        assert_eq!(query_author("Napoleon Hill, Ross Cornwell (Editor)"), "Napoleon Hill");
        assert_eq!(query_author("Lem, Stanisław"), "Stanisław Lem");
        assert_eq!(surname("Mickiewicz, Adam"), "Mickiewicz");
        assert_eq!(surname("Jerry Hicks & Esther Hicks"), "Hicks");
        assert_eq!(query_title("Myśl i bogać się. Wersja oryginalna, odświeżona"), "Myśl i bogać się");
        assert_eq!(query_title("Zrób to teraz!: przejmij kontrolę"), "Zrób to teraz!");
        assert_eq!(query_title("Solaris"), "Solaris");
    }

    fn hit(source: &str, title: &str, author: &str, description: Option<&str>, cover: Option<&str>, isbn: Option<&str>) -> MetadataHit {
        MetadataHit {
            source: source.into(), external_id: source.into(), title: title.into(), subtitle: None, authors: vec![author.into()],
            publisher: None, publication_year: None, publication_date: None, language: None, description: description.map(Into::into),
            page_count: None, isbn10: None, isbn13: isbn.map(Into::into), cover_url: cover.map(Into::into), series_name: None,
            genres: Vec::new(), rating: None, info_url: None,
        }
    }

    #[test]
    fn merges_fields_from_several_sources() {
        let hits = vec![
            hit("openlibrary", "Ojciec chrzestny", "Puzo Mario", None, None, Some("9788378855323")),
            hit("lubimyczytac", "Ojciec chrzestny", "Mario Puzo", Some("Saga rodu Corleone, opowiedziana z perspektywy jego członków."), Some("https://s.lubimyczytac.pl/x.jpg"), Some("9788378855323")),
            hit("bn", "Zupełnie inna", "Ktoś", Some("nie ta"), None, None),
        ];
        let best = merge_hits(hits, "Ojciec Chrzestny", "Mario Puzo", Some("9788378855323")).unwrap();
        assert!(best.description.unwrap().starts_with("Saga"));
        assert_eq!(best.cover_url.as_deref(), Some("https://s.lubimyczytac.pl/x.jpg"));
        let none = merge_hits(vec![hit("bn", "Inna", "Ktoś", None, None, None)], "Solaris", "Lem", None);
        assert!(none.is_none());
    }

    #[test]
    fn a_different_subtitle_does_not_win_over_the_exact_title() {
        let wanted = "Myśl i bogać się. Wersja oryginalna, odświeżona i zaktualizowana";
        let hits = vec![
            hit("lubimyczytac", "Myśl i bogać się. Książka ćwiczeń", "Napoleon Hill", Some("Ćwiczenia do książki, bardzo długi opis zeszytu ćwiczeń."), Some("https://x/1.jpg"), None),
            hit("upolujebooka", "Myśl i bogać się. Wersja oryginalna, odświeżona i zaktualizowana", "Napoleon Hill", Some("Krótki opis"), None, None),
        ];
        let best = merge_hits(hits, wanted, "Napoleon Hill, Ross Cornwell (Editor)", None).unwrap();
        assert_eq!(best.source, "upolujebooka");
    }
}
