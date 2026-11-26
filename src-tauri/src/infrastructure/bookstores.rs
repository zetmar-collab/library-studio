//! LubimyCzytać i UpolujEbooka, czytane tak jak robią to wtyczki Calibre: jedno wyszukiwanie
//! i co najwyżej dwie strony książek na każde uzupełnienie, z uczciwym nagłówkiem User-Agent.
//! Obie strony publikują dane książki w JSON-LD (schema.org/Book), więc parser jest wspólny.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::domain::{fold, normalize_isbn, strip_html, MetadataHit};
use crate::infrastructure::importer::decode_entities;
use crate::infrastructure::metadata::{query_title, surname, urlencoding};

const USER_AGENT: &str = "Library Studio/0.3.2 (private catalog; metadata lookup like the calibre plugin)";
const PAGE_LIMIT: usize = 4_000_000;
const PAGES_PER_LOOKUP: usize = 2;
/// Najkrótszy odstęp między zapytaniami do jednego serwisu. Przy uzupełnianiu całej biblioteki
/// zapytania idą jedno po drugim, a bez przerw serwis odpowiada „429 Too Many Requests”.
const MIN_GAP: Duration = Duration::from_millis(1500);

static LAST_REQUEST: Mutex<Option<HashMap<String, Instant>>> = Mutex::new(None);

fn wait_for_turn(url: &str) {
    let host = reqwest::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default();
    let mut guard = LAST_REQUEST.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let map = guard.get_or_insert_with(HashMap::new);
    if let Some(last) = map.get(&host) {
        let elapsed = last.elapsed();
        if elapsed < MIN_GAP {
            std::thread::sleep(MIN_GAP - elapsed);
        }
    }
    map.insert(host, Instant::now());
}

struct Candidate {
    url: String,
    title: String,
    authors: Vec<String>,
}

pub fn lubimy(title: &str, author: &str) -> Result<Vec<MetadataHit>, String> {
    let Some(phrase) = phrase(title, author) else { return Ok(Vec::new()) };
    let html = fetch(&format!("https://lubimyczytac.pl/szukaj/ksiazki?phrase={}", urlencoding(&phrase)))?;
    let mut hits = Vec::new();
    for candidate in pick(lubimy_candidates(&html), title, author) {
        if let Ok(page) = fetch(&candidate.url) {
            if let Some(hit) = lubimy_book(&page, &candidate.url) {
                hits.push(hit);
            }
        }
    }
    Ok(hits)
}

pub fn upoluj(title: &str, author: &str) -> Result<Vec<MetadataHit>, String> {
    let Some(phrase) = phrase(title, author) else { return Ok(Vec::new()) };
    // Adres wyszukiwania ma postać /szukaj,fraza.html; przecinek w frazie zmieniłby jej znaczenie.
    let html = fetch(&format!("https://upolujebooka.pl/szukaj,{}.html", urlencoding(&phrase.replace(',', " "))))?;
    let mut hits = Vec::new();
    for candidate in pick(upoluj_candidates(&html), title, author) {
        if let Ok(page) = fetch(&candidate.url) {
            if let Some(hit) = upoluj_book(&page, &candidate.url) {
                hits.push(hit);
            }
        }
    }
    Ok(hits)
}

fn phrase(title: &str, author: &str) -> Option<String> {
    let short = query_title(title);
    if short.is_empty() {
        return None;
    }
    let last = surname(author);
    Some(if last.is_empty() { short } else { format!("{short} {last}") })
}

fn fetch(url: &str) -> Result<String, String> {
    wait_for_turn(url);
    let client = reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;
    let mut response = client.get(url).header("Accept-Language", "pl,en;q=0.5").send().map_err(|e| e.to_string())?;
    let status = response.status();
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err("serwis prosi o przerwę, spróbuj za kilka minut".into());
    }
    if !status.is_success() {
        return Err(format!("odpowiedź {status}"));
    }
    let bytes = crate::infrastructure::files::read_limited(&mut response, PAGE_LIMIT)?;
    let text = String::from_utf8_lossy(&bytes).to_string();
    if text.contains("_cf_chl_opt") {
        return Err("serwis poprosił o weryfikację w przeglądarce, spróbuj później".into());
    }
    Ok(text)
}

/// Wybiera najwyżej dwie pozycje z wyników, których tytuł (i nazwisko autora, jeśli je znamy) pasuje.
fn pick(candidates: Vec<Candidate>, title: &str, author: &str) -> Vec<Candidate> {
    let wanted = fold(&query_title(title));
    let wanted_full = fold(title);
    let last = fold(&surname(author));
    let mut scored: Vec<(i32, Candidate)> = candidates
        .into_iter()
        .filter_map(|candidate| {
            let found = fold(&candidate.title);
            let title_score = if found == wanted || found == wanted_full {
                50
            } else if found.starts_with(&wanted) || wanted_full.starts_with(&found) {
                35
            } else if found.contains(&wanted) {
                15
            } else {
                return None;
            };
            let authors = fold(&candidate.authors.join(" "));
            let author_score = if last.is_empty() || candidate.authors.is_empty() {
                0
            } else if authors.split_whitespace().any(|word| word == last) {
                40
            } else {
                return None;
            };
            let score = title_score + author_score;
            (score > 0).then_some((score, candidate))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.into_iter().take(PAGES_PER_LOOKUP).map(|(_, candidate)| candidate).collect()
}

fn lubimy_candidates(html: &str) -> Vec<Candidate> {
    let mut out = Vec::new();
    let marker = "class=\"book-card__title\"";
    let mut rest = html;
    while let Some(start) = rest.find(marker) {
        let card = &rest[start + marker.len()..];
        let end = card.find(marker).unwrap_or(card.len());
        let card = &card[..end];
        rest = &rest[start + marker.len()..];
        let (Some(title), Some(href)) = (attr(card, "title"), attr(card, "href")) else { continue };
        if !href.starts_with("/ksiazka/") {
            continue;
        }
        let authors = between(card, "book-card__author\">", "</div>")
            .map(|block| strip_html(&decode_entities(block)).split(',').map(|name| name.trim().to_string()).filter(|name| !name.is_empty()).collect())
            .unwrap_or_default();
        out.push(Candidate { url: format!("https://lubimyczytac.pl{href}"), title: decode_entities(&title), authors });
    }
    out
}

fn upoluj_candidates(html: &str) -> Vec<Candidate> {
    let mut out = Vec::new();
    let marker = "data-ue-pos=\"";
    let mut rest = html;
    while let Some(start) = rest.find(marker) {
        let card = &rest[start + marker.len()..];
        let end = card.find(marker).unwrap_or(card.len());
        let card = &card[..end];
        rest = &rest[start + marker.len()..];
        let Some(href) = between(card, "href=\"/oferta,", "\"") else { continue };
        let href = href.split('?').next().unwrap_or(href);
        let Some(title) = between(card, "<h4", "</h4>").and_then(|block| block.find('>').map(|i| strip_html(&decode_entities(&block[i + 1..])))) else { continue };
        let mut authors = Vec::new();
        let mut scan = card;
        while let Some(i) = scan.find("itemprop=\"author\">") {
            scan = &scan[i + 18..];
            if let Some(name) = scan.split("</a>").next() {
                authors.push(strip_html(&decode_entities(name)));
            }
        }
        out.push(Candidate { url: format!("https://upolujebooka.pl/oferta,{href}"), title, authors });
    }
    out
}

fn lubimy_book(html: &str, url: &str) -> Option<MetadataHit> {
    let mut hit = book_from_ld(html, "lubimyczytac", url)?;
    if let Some(block) = between(html, "id=\"book-description\">", "</div>") {
        let text = clean_description(block);
        if !text.is_empty() {
            hit.description = Some(text);
        }
    }
    if hit.publisher.is_none() {
        hit.publisher = between(html, "Wydawnictwo: <a", "</a>").and_then(|block| block.find('>').map(|i| strip_html(&decode_entities(&block[i + 1..])))).filter(|v| !v.is_empty());
    }
    if let Some(block) = between(html, "class=\"book__category", "</a>") {
        if let Some(i) = block.find('>') {
            let genre = strip_html(&decode_entities(&block[i + 1..]));
            if !genre.is_empty() {
                hit.genres = vec![genre];
            }
        }
    }
    if let Some(block) = between(html, "href=\"https://lubimyczytac.pl/cykl/", "</a>").or_else(|| between(html, "href=\"/cykl/", "</a>")) {
        if let Some(i) = block.find('>') {
            let series = strip_html(&decode_entities(&block[i + 1..]));
            // „Nazwa cyklu (tom 2)” → nazwa bez numeru tomu.
            let name = series.split(" (tom").next().unwrap_or(&series).trim().to_string();
            if !name.is_empty() {
                hit.series_name = Some(name);
            }
        }
    }
    Some(hit)
}

fn upoluj_book(html: &str, url: &str) -> Option<MetadataHit> {
    let mut hit = book_from_ld(html, "upolujebooka", url)?;
    if let Some(block) = between(html, "id=\"opis\">", "</div>") {
        let text = clean_description(block);
        if !text.is_empty() {
            hit.description = Some(text);
        }
    }
    if hit.genres.is_empty() {
        let crumbs = json_ld(html)
            .into_iter()
            .flat_map(|value| match value {
                Value::Array(items) => items,
                other => vec![other],
            })
            .find(|value| value.get("@type").and_then(Value::as_str) == Some("BreadcrumbList"));
        if let Some(name) = crumbs.and_then(|list| list.get("itemListElement")?.as_array()?.last()?.get("name")?.as_str().map(str::to_string)) {
            hit.genres = vec![name];
        }
    }
    Some(hit)
}

fn clean_description(block: &str) -> String {
    let with_breaks = block.replace("</p>", "\n").replace("<br>", "\n").replace("<br />", "\n").replace("<br/>", "\n");
    let decoded = decode_entities(&with_breaks);
    decoded
        .split('\n')
        .map(strip_html)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn json_ld(html: &str) -> Vec<Value> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(start) = rest.find("application/ld+json") {
        rest = &rest[start..];
        let Some(open) = rest.find('>') else { break };
        let Some(close) = rest.find("</script>") else { break };
        if open < close {
            if let Ok(value) = serde_json::from_str::<Value>(rest[open + 1..close].trim()) {
                out.push(value);
            }
        }
        rest = &rest[close..];
    }
    out
}

fn book_from_ld(html: &str, source: &str, url: &str) -> Option<MetadataHit> {
    let book = json_ld(html).into_iter().find(|value| value.get("@type").and_then(Value::as_str) == Some("Book"))?;
    let text = |key: &str| book.get(key).and_then(Value::as_str).map(|v| decode_entities(v.trim())).filter(|v| !v.is_empty());
    let title = text("name")?;
    let authors = match book.get("author") {
        Some(Value::Array(list)) => list.iter().filter_map(|person| person.get("name").and_then(Value::as_str)).map(|v| decode_entities(v.trim())).collect(),
        Some(person) => person.get("name").and_then(Value::as_str).map(|v| vec![decode_entities(v.trim())]).unwrap_or_default(),
        None => Vec::new(),
    };
    let isbn = text("isbn").map(|v| normalize_isbn(&v));
    let (isbn13, isbn10) = match isbn {
        Some(v) if v.len() == 13 => (Some(v), None),
        Some(v) if v.len() == 10 => (None, Some(v)),
        _ => (None, None),
    };
    let date = text("datePublished");
    let year = date.as_deref().and_then(|d| d.get(0..4)).and_then(|y| y.parse::<i64>().ok()).filter(|y| *y > 1000);
    let image = match book.get("image") {
        Some(Value::Array(list)) => list.first().and_then(Value::as_str).map(str::to_string),
        Some(Value::String(url)) => Some(url.clone()),
        _ => None,
    }
    .map(|url| url.replacen("http://", "https://", 1))
    .filter(|url| url.starts_with("https://") && !url.contains("default-book"));
    let pages = match book.get("numberOfPages") {
        Some(Value::Number(n)) => n.as_i64(),
        Some(Value::String(s)) => s.trim().parse().ok(),
        _ => None,
    };
    let rating = book.get("aggregateRating").and_then(|rating| {
        let value = match rating.get("ratingValue")? {
            Value::String(s) => s.replace(',', ".").parse::<f64>().ok()?,
            Value::Number(n) => n.as_f64()?,
            _ => return None,
        };
        let best = match rating.get("bestRating") {
            Some(Value::String(s)) => s.parse::<f64>().ok(),
            Some(Value::Number(n)) => n.as_f64(),
            _ => None,
        }
        .unwrap_or(5.0);
        (value > 0.0 && best > 0.0).then(|| (value / best * 50.0).round() / 10.0)
    });
    let genre = text("genre").filter(|g| !g.starts_with("http"));
    let external_id = url.trim_end_matches('/').rsplit('/').next().unwrap_or(url).to_string();
    let external_id = match source {
        "lubimyczytac" => url.split("/ksiazka/").nth(1).and_then(|rest| rest.split('/').next()).unwrap_or(&external_id).to_string(),
        _ => url.split("oferta,").nth(1).and_then(|rest| rest.split(',').next()).unwrap_or(&external_id).to_string(),
    };
    Some(MetadataHit {
        source: source.into(),
        external_id,
        title,
        subtitle: None,
        authors,
        publisher: book.get("publisher").and_then(|p| p.get("name")).and_then(Value::as_str).map(|v| decode_entities(v.trim())).filter(|v| !v.is_empty()),
        publication_year: year,
        publication_date: date,
        language: text("inLanguage").map(|lang| language_code(&lang)),
        description: text("description").map(|d| strip_html(&d)),
        page_count: pages,
        isbn10,
        isbn13,
        cover_url: image,
        series_name: None,
        genres: genre.into_iter().collect(),
        rating,
        info_url: Some(url.to_string()),
    })
}

fn language_code(name: &str) -> String {
    match fold(name).as_str() {
        "polski" | "pl" => "pl".into(),
        "angielski" | "en" | "english" => "en".into(),
        "niemiecki" | "de" => "de".into(),
        "francuski" | "fr" => "fr".into(),
        "rosyjski" | "ru" => "ru".into(),
        "ukrainski" | "uk" => "uk".into(),
        other => other.to_string(),
    }
}

fn attr(html: &str, name: &str) -> Option<String> {
    between(html, &format!("{name}=\""), "\"").map(str::to_string)
}

fn between<'a>(html: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let from = html.find(start)? + start.len();
    let len = html[from..].find(end)?;
    Some(&html[from..from + len])
}

#[cfg(test)]
mod tests {
    use super::*;

    const LUBIMY_SEARCH: &str = r#"<div class="book-card"><a class="book-card__title" title="Potęga intencji Naucz się współtworzyć swój świat" href="/ksiazka/5157662/potega-intencji"> Potęga intencji </a><div class="book-card__author"><a href="https://lubimyczytac.pl/autor/88527/wayne-w-dyer">Wayne W. Dyer</a></div></div>
<div class="book-card"><a class="book-card__title" title="Tak miało być" href="/ksiazka/4840879/tak-mialo-byc"> x </a><div class="book-card__author"><a href="/autor/1">Ktoś Inny</a></div></div>"#;

    const LUBIMY_BOOK: &str = r#"<script type="application/ld+json">{"@type":"Organization","name":"x"}</script>
<script type="application/ld+json">{"@context": "http://schema.org","@type": "Book","name":"Potęga intencji Naucz się współtworzyć swój świat","isbn":"9788375799118","datePublished":"2024-01-01","inLanguage":"polski","numberOfPages":"272","image":"http://s.lubimyczytac.pl/upload/books/1-352x500.jpg","author":{"@type":"Person","name":"Wayne W. Dyer"},"aggregateRating":{"ratingValue":"7,3","bestRating":10}}</script>
<span>Wydawnictwo: <a href="/wydawnictwo/1/galaktyka">Galaktyka</a></span><a class="book__category d-sm-block" href="/kategoria/poradniki"> poradniki </a>
<div class="book__description" id="book-description"> Wayne W. Dyer badał intencję&nbsp;jako siłę.<br><br>Część II to poradnik. </div>"#;

    const UPOLUJ_SEARCH: &str = r#"<div data-ue-id="17123" data-ue-pos="1"><a href="/oferta,17123,solaris.html?ue_src=list" title="Solaris"><img></a><h4 class="text-xs">Solaris</h4><p><a href="/autor,26602,stanislaw_lem.html" itemprop="author">Stanisław Lem</a></p></div>"#;

    #[test]
    fn reads_lubimy_search_and_book_page() {
        let found = pick(lubimy_candidates(LUBIMY_SEARCH), "Potęga intencji", "Dr Wayne W. Dyer");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].url, "https://lubimyczytac.pl/ksiazka/5157662/potega-intencji");
        let hit = lubimy_book(LUBIMY_BOOK, &found[0].url).unwrap();
        assert_eq!(hit.external_id, "5157662");
        assert_eq!(hit.isbn13.as_deref(), Some("9788375799118"));
        assert_eq!(hit.page_count, Some(272));
        assert_eq!(hit.publication_year, Some(2024));
        assert_eq!(hit.language.as_deref(), Some("pl"));
        assert_eq!(hit.publisher.as_deref(), Some("Galaktyka"));
        assert_eq!(hit.genres, vec!["poradniki".to_string()]);
        assert_eq!(hit.cover_url.as_deref(), Some("https://s.lubimyczytac.pl/upload/books/1-352x500.jpg"));
        assert_eq!(hit.rating, Some(3.7));
        assert_eq!(hit.description.as_deref(), Some("Wayne W. Dyer badał intencję jako siłę.\n\nCzęść II to poradnik."));
    }

    #[test]
    fn reads_upoluj_search_results() {
        let found = pick(upoluj_candidates(UPOLUJ_SEARCH), "Solaris", "Lem, Stanisław");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].url, "https://upolujebooka.pl/oferta,17123,solaris.html");
        assert!(pick(upoluj_candidates(UPOLUJ_SEARCH), "Solaris", "Kowalski").is_empty());
    }
}
