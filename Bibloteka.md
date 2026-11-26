Zbuduj aplikację „Biblioteka” — prywatny katalog książek papierowych i elektronicznych dla użytkownika indywidualnego. Aplikacja ma łączyć inwentaryzację fizycznych książek, zarządzanie plikami e-booków, wyszukiwanie metadanych, wirtualne półki, kopie zapasowe i synchronizację.

## Cel produktu

Stwórz nowoczesną, szybką i prywatną aplikację typu „personal hybrid library”. Jej główną wartością jest jedno miejsce, w którym użytkownik widzi książkę jako jeden zasób, ale może mieć wiele egzemplarzy: papierowy, EPUB, PDF, MOBI, AZW3 lub audiobook.

Aplikacja nie ma być kolejnym Goodreads. Priorytetem jest rzetelna inwentaryzacja własnej kolekcji, kontrola nad plikami oraz niezawodny backup i synchronizacja.

## Stack technologiczny

Wybierz stack zgodny z poniższymi zasadami:

- Aplikacja desktopowa na Windows, macOS i Linux.
- Ewentualnie aplikacja mobilna jako drugi etap, ale MVP ma działać najpierw na desktopie.
- UI: React + TypeScript + Tailwind CSS albo podobny nowoczesny stack.
- Backend lokalny: Tauri + Rust albo Electron + Node.js; preferuj Tauri ze względu na mniejsze zużycie zasobów.
- Baza danych lokalna: SQLite z WAL mode.
- Warstwa dostępu do danych: repozytoria + migracje schematu.
- Testy: jednostkowe dla logiki biznesowej, integracyjne dla bazy danych i adapterów metadanych, E2E dla kluczowych ścieżek.
- Nie przechowuj haseł ani tokenów w bazie w postaci jawnej; używaj systemowego magazynu kluczy.

## Model danych

Zaprojektuj schemat SQLite z migracjami. Minimum następujące encje:

### Book
- id
- title
- original_title
- subtitle
- authors
- translators
- publisher
- publication_year
- language
- description
- cover_path
- series_name
- series_number
- genres
- tags
- average_rating
- rating_source
- created_at
- updated_at
- deleted_at

### BookEdition
- id
- book_id
- isbn10
- isbn13
- edition_name
- publisher
- publication_date
- page_count
- cover_path
- format
- external_ids: JSON, np. Open Library, Google Books, LubimyCzytać
- metadata_source
- created_at
- updated_at

### PhysicalCopy
- id
- book_edition_id
- shelf_id
- location_note
- acquisition_date
- price
- condition
- owner
- loaned_to
- loaned_date
- return_due_date
- notes
- barcode_value
- created_at
- updated_at

### DigitalCopy
- id
- book_edition_id
- file_path
- file_name
- file_size
- file_hash_sha256
- file_format
- drm_status
- added_at
- last_verified_at
- notes

### Shelf
- id
- name
- description
- color
- icon
- sort_order
- is_smart_shelf
- smart_filter_json
- created_at
- updated_at

### ShelfItem
- shelf_id
- book_id albo book_edition_id
- position
- added_at

### ReadingProgress
- id
- book_id
- format
- progress_percent
- current_page
- current_chapter
- started_at
- finished_at
- rating
- review
- notes

### MetadataCache
- id
- source
- external_id
- isbn
- query_hash
- payload_json
- fetched_at
- expires_at

### SyncState
- id
- device_id
- provider
- last_sync_at
- status
- error_message
- conflict_count

### BackupJob
- id
- type: database, files, full
- destination
- status
- started_at
- finished_at
- size_bytes
- checksum
- error_message

## Funkcjonalności MVP

### 1. Dodawanie książek

Użytkownik musi móc dodać książkę na trzy sposoby:

1. Skanowanie kodu kreskowego ISBN przez kamerę.
2. Ręczne wpisanie ISBN-10 lub ISBN-13.
3. Wyszukiwanie po tytule, autorze, serii lub słowach kluczowych.

Po znalezieniu książki aplikacja ma:
- pokazać listę kandydatów z okładką, tytułem, autorem, rokiem i źródłem metadanych;
- umożliwić wybór właściwej pozycji;
- utworzyć rekord Book oraz BookEdition;
- umożliwić ręczną korektę metadanych;
- zapisać źródło danych w polu metadata_source;
- zapobiec duplikatom po ISBN, tytule i autorze.

### 2. Źródła metadanych

Adresy sprawdzone 24 listopada 2026. Aplikacja woła tylko udokumentowane API i tylko na żądanie użytkownika (wyszukiwanie albo ISBN), z cache w `MetadataCache`.

Zaimplementuj warstwę adapterów metadanych z jednym wspólnym interfejsem:

interface MetadataProvider {
  id: string;
  search(query: SearchQuery): Promise<MetadataResult[]>;
  findByIsbn(isbn: string): Promise<MetadataResult | null>;
  getDetails(externalId: string): Promise<MetadataResult | null>;
}

Zaimplementuj adaptery dla:

1. **Open Library**
   - dokumentacja: [indeks API](https://openlibrary.org/developers/api), [Search API](https://openlibrary.org/dev/docs/api/search), [Books API](https://openlibrary.org/dev/docs/api/books), [Covers API](https://openlibrary.org/dev/docs/api/covers);
   - wyszukiwanie dzieł: `GET https://openlibrary.org/search.json?q=isbn:{isbn}` oraz `title` / `author` (parametr `q` w składni `isbn:`, `title:`, `author:`). Odpowiedź Search API opisuje **dzieło** (Work), nie konkretne wydanie;
   - wydanie po ISBN: `GET https://openlibrary.org/isbn/{isbn}.json` (przekierowanie do `/books/{OLID}.json`);
   - dzieło i wydanie po identyfikatorze: `GET https://openlibrary.org/works/{OLID}.json`, `GET https://openlibrary.org/books/{OLID}.json`, wydania dzieła: `GET https://openlibrary.org/works/{OLID}/editions.json`;
   - okładki: `https://covers.openlibrary.org/b/isbn/{isbn}-M.jpg?default=false` oraz `https://covers.openlibrary.org/b/id/{cover_i}-M.jpg`. Rozmiary: `S`, `M`, `L`. Zapytania po ISBN, OCLC i LCCN są limitowane do 100 na 5 minut z jednego IP i przy przekroczeniu zwracają 403 — okładki pobieraj po Cover ID albo OLID, gdy już je masz, i zapisuj lokalnie;
   - nie opieraj się na starszym `https://openlibrary.org/api/books` (`jscmd=data`). Open Library oznacza go jako legacy i może go wycofać;
   - sprawdzone 24 listopada 2026: `search.json?q=isbn:9780140328721` zwraca dzieło, `isbn/9780140328721.json` zwraca wydanie, okładka `-M.jpg` zwraca JPEG.

2. **Google Books**
   - dokumentacja: [Using the API](https://developers.google.com/books/docs/v1/using), [volumes.list](https://developers.google.com/books/docs/v1/reference/volumes/list), [zasób Volume](https://developers.google.com/books/docs/v1/reference/volumes), [warunki API](https://developers.google.com/books/terms);
   - wyszukiwanie: `GET https://www.googleapis.com/books/v1/volumes?q={zapytanie}&key={KLUCZ}`. Słowa kluczowe pól: `isbn:`, `intitle:`, `inauthor:`, `inpublisher:`. Szczegóły woluminu: `GET https://www.googleapis.com/books/v1/volumes/{volumeId}?key={KLUCZ}`;
   - `maxResults` od 0 do 40, `startIndex` od 0. Identyfikatory są w `volumeInfo.industryIdentifiers` jako `ISBN_10` i `ISBN_13`;
   - klucz API podaje użytkownik w ustawieniach. Trzymaj go w systemowym magazynie kluczy, nie w SQLite i nie w repozytorium. 24 listopada 2026 zapytanie bez własnego klucza dostało HTTP 429: dzienny limit współdzielonego projektu wynosi 0 (`quota_limit_value: 0`). Brak klucza traktuj jako źródło wyłączone, a 429, 403 i brak `items` jako zwykłe błędy z czytelnym komunikatem;
   - przed użyciem komercyjnym sprawdź [warunki Google Books API](https://developers.google.com/books/terms). Samo posiadanie klucza nie oznacza zgody na dowolne wykorzystanie okładek i opisów.

3. **Biblioteka Narodowa**
   - dokumentacja: [rekordy bibliograficzne](https://data.bn.org.pl/docs/bibs);
   - licencja danych: CC0 1.0. To największy polski katalog bibliograficzny z oficjalnym API;
   - wyszukiwanie: `GET https://data.bn.org.pl/api/institutions/bibs.json?limit=8&isbnIssn={isbn}` oraz parametry `title` i `author`;
   - tytuł, autor, wydawca, rok, ISBN, seria i gatunek bierz z pól MARC w odpowiedzi JSON. Katalog nie zwraca plików okładek.

4. **LubimyCzytać.pl — wyłączone**
   - serwis: [lubimyczytac.pl](https://lubimyczytac.pl/), [robots.txt](https://lubimyczytac.pl/robots.txt), [informacje prawne](https://lubimyczytac.pl/informacje-prawne);
   - nie ma publicznej, udokumentowanej API. Nieoficjalne ścieżki w rodzaju `/searcher/getsuggestions` nie są API i nie wolno ich wołać;
   - regulamin (punkt 6.2 i 6.11, stan na 24 listopada 2026) zabrania automatycznego pobierania elementów serwisu i bazy danych, w tym danych książek, autorów, tłumaczy i wydawnictw, oraz używania programów generujących zapytania do serwerów administratora. Zakaz nie jest ograniczony do użytku komercyjnego;
   - `robots.txt` całkowicie blokuje m.in. GPTBot, ClaudeBot, CCBot, Bytespider, AhrefsBot i Applebot oraz ścieżki `/ajax/*`, rekomendacje i część endpointów sklepowych;
   - adapter `LubimyCzytacProvider` może istnieć tylko jako wyłączony moduł za wspólnym interfejsem. Nie wysyłaj do lubimyczytac.pl żadnych żądań, nie parsuj HTML i nie dodawaj przełącznika, który włącza pobieranie bez pisemnej zgody serwisu. Polski katalog uzupełniaj z Biblioteki Narodowej, Open Library, Google Books i wpisu ręcznego.

5. **UpolujEbooka.pl — wyłączone**
   - serwis: [upolujebooka.pl](https://upolujebooka.pl/), [regulamin](https://upolujebooka.pl/regulamin.html), [robots.txt](https://upolujebooka.pl/robots.txt);
   - nie ma publicznego API dla czytelnika. Plik XML jest kanałem, którym księgarnie przekazują oferty do serwisu, nie źródłem do pobierania;
   - regulamin, punkt V.5, zabrania kopiowania treści i zdjęć bez zgody serwisu albo partnera. `robots.txt` wyłącza z indeksu wyszukiwanie (`/szukaj`) i `/api/`;
   - nie wysyłaj zapytań do upolujebooka.pl i nie pobieraj stamtąd okładek.

Strategia dopasowania metadanych:
- Najpierw szukaj po ISBN-13.
- Następnie po ISBN-10.
- Następnie po tytule i autorze.
- Pokaż użytkownikowi wyniki z kilku źródeł i pozwól wybrać najlepsze dopasowanie.
- Nie nadpisuj automatycznie ręcznie zmienionych danych.
- Zapisuj pochodzenie każdego pola metadanych.

### 3. Wirtualne półki

Zaimplementuj półki:

- ręczne, np. „Salon”, „Biuro”, „Regał 2”, „Do przeczytania”;
- inteligentne, tworzone na podstawie filtrów, np. „Wszystkie fantasy”, „Nieprzeczytane”, „Pożyczone”, „EPUB”;
- przeciąganie i upuszczanie książek między półkami;
- kolejność książek na półce;
- wiele półek dla jednej książki;
- filtrowanie po formacie, autorze, serii, tagach, statusie przeczytania i lokalizacji.

### 4. Katalog plików e-booków

Podczas pierwszej konfiguracji aplikacja ma tworzyć katalog biblioteki, np.:

Biblioteka/
  Autor/
    Tytuł książki/
      plik.epub
      plik.pdf
      cover.jpg
      metadata.json

Zasady:
- Podczas dodawania e-booka użytkownik wskazuje plik EPUB, PDF, MOBI, AZW3 lub inny obsługiwany format.
- Aplikacja kopiuje plik do podkatalogu książki.
- Nie przenoś pliku bez wyraźnej zgody użytkownika.
- Oblicz SHA-256 pliku i zapisz go w bazie.
- Wykrywaj duplikaty po hashu pliku.
- Obsłuż sytuację, gdy plik zostanie usunięty lub zmieniony poza aplikacją.
- Dodaj funkcję „Zweryfikuj bibliotekę plików”.
- Pozwól użytkownikowi wybrać katalog biblioteki podczas instalacji lub pierwszego uruchomienia.
- Obsłuż polskie znaki, długie nazwy plików i znaki specjalne w nazwach.
- Twórz bezpieczne nazwy katalogów na podstawie tytułu i autora.

### 5. Wyszukiwanie

Zaimplementuj szybkie wyszukiwanie w całej bibliotece:

- tytuł;
- autor;
- seria;
- ISBN;
- tag;
- wydawca;
- opis;
- nazwa pliku;
- zawartość metadanych e-booka, jeśli jest dostępna;
- notatki użytkownika.

Wyszukiwanie ma:
- działać lokalnie i offline;
- obsługiwać polskie znaki i odmiany słów w rozsądnym zakresie;
- pokazywać wyniki z podświetleniem trafień;
- umożliwiać filtry: format, półka, status przeczytania, autor, seria, tag, rok wydania;
- sortować po tytule, autorze, dacie dodania, ocenie i dacie publikacji.

### 6. Backup

Zaimplementuj dwa typy kopii zapasowych:

1. **Kopia bazy danych**
   - automatyczna kopia pliku SQLite;
   - harmonogram, np. codziennie;
   - retencja, np. 7 dziennych, 4 tygodniowych i 6 miesięcznych kopii;
   - suma kontrolna SHA-256;
   - test przywracania kopii.

2. **Kopia plików e-booków**
   - druga kopia całego katalogu plików;
   - kopia przyrostowa, jeśli to możliwe;
   - weryfikacja integralności plików;
   - obsługa lokalnego dysku, dysku sieciowego oraz folderu synchronizowanego.

Nie przechowuj jedynej kopii danych wewnątrz aplikacji. Zawsze umożliwiaj pełny eksport biblioteki do otwartego formatu.

### 7. Synchronizacja

Zaprojektuj synchronizację jako moduł rozszerzalny. W MVP zaimplementuj co najmniej jeden provider:

- lokalny folder / dysk sieciowy;
- WebDAV;
- Nextcloud;
- Synology Drive;
- Google Drive;
- OneDrive;
- Dropbox.

Zasady synchronizacji:
- synchronizuj bazę danych, metadane, okładki i pliki e-booków;
- wykrywaj konflikty, np. zmiana tego samego rekordu na dwóch urządzeniach;
- pokazuj użytkownikowi jasny ekran rozwiązywania konfliktów;
- nie usuwaj plików automatycznie bez potwierdzenia;
- obsługuj pracę offline;
- zapisuj historię synchronizacji;
- umożliwiaj pełną resynchronizację;
- używaj identyfikatorów UUID, a nie autoincrementów, aby ułatwić synchronizację między urządzeniami. Pole `id` każdej encji jest UUID w postaci tekstowej. To dotyczy też tabel z sekcji „Model danych”.

## UX i ekrany

Zaprojektuj następujące ekrany:

1. **Biblioteka**
   - siatka lub lista książek;
   - szybkie filtry;
   - wyszukiwarka;
   - przełączanie widoku: wszystkie, papierowe, e-booki, pożyczone, przeczytane.

2. **Szczegóły książki**
   - okładka;
   - metadane;
   - egzemplarze papierowe;
   - pliki elektroniczne;
   - półki;
   - notatki;
   - historia czytania;
   - źródła metadanych;
   - przycisk edycji.

3. **Dodawanie książki**
   - skan ISBN;
   - wpisanie ISBN;
   - wyszukiwanie;
   - wybór wyniku;
   - weryfikacja metadanych;
   - dodanie egzemplarza papierowego lub pliku e-book.

4. **Półki**
   - lista półek;
   - tworzenie półki;
   - edycja filtrów półki inteligentnej;
   - przeciąganie książek.

5. **Pliki**
   - lista plików;
   - stan pliku: obecny, brakujący, zmieniony, duplikat;
   - otwieranie pliku w zewnętrznym czytniku;
   - weryfikacja biblioteki.

6. **Backup i synchronizacja**
   - status ostatniej kopii;
   - historia kopii;
   - konfiguracja miejsc backupu;
   - status synchronizacji;
   - konflikty i ich rozwiązywanie.

7. **Ustawienia**
   - katalog biblioteki;
   - źródła metadanych;
   - język interfejsu;
   - motyw jasny i ciemny;
   - skróty klawiszowe;
   - polityka prywatności;
   - eksport i import danych.

## Bezpieczeństwo i prywatność

- Wszystkie dane użytkownika mają domyślnie pozostawać lokalnie.
- Nie wysyłaj plików e-booków do chmury bez wyraźnej akcji użytkownika.
- Nie zbieraj danych telemetrycznych bez zgody.
- Nie przechowuj haseł w bazie SQLite.
- Szyfruj tokeny dostępu do chmury przy użyciu systemowego keystore.
- Dodaj eksport całej biblioteki do otwartego formatu: JSON + pliki.
- Dodaj import danych z eksportu.
- Nie uzależniaj użytkownika od Twojego serwera.

## Jakość inżynierska

- Używaj czystej architektury: domain, application, infrastructure, UI.
- Oddziel logikę metadanych, plików, synchronizacji i bazy danych.
- Wszystkie operacje na plikach wykonuj atomowo.
- Twórz dziennik zdarzeń aplikacji z poziomami info, warning i error.
- Obsłuż błędy sieci, uszkodzone pliki, brak miejsca na dysku, konflikty synchronizacji i nieprawidłowe ISBN.
- Dodaj obsługę anulowania długich operacji.
- Dodaj pasek postępu dla skanowania, importu, backupu i synchronizacji.
- Napisz testy dla:
  - walidacji ISBN-10 i ISBN-13;
  - parserów metadanych;
  - tworzenia struktury katalogów;
  - wykrywania duplikatów;
  - hashowania plików;
  - backupu i przywracania;
  - synchronizacji i konfliktów;
  - wyszukiwania.

## Dokumentacja

Stwórz:

- README.md z opisem projektu, instalacją i uruchomieniem;
- ARCHITECTURE.md z diagramem modułów;
- DATA_MODEL.md z opisem schematu bazy;
- METADATA_PROVIDERS.md z opisem Open Library, Google Books i LubimyCzytać;
- BACKUP_SYNC.md z opisem strategii kopii i synchronizacji;
- PRIVACY.md z opisem danych przetwarzanych lokalnie;
- CHANGELOG.md;
- CONTRIBUTING.md.

## Kolejność prac

Zaimplementuj projekt w następującej kolejności:

1. Szkielet aplikacji, konfiguracja, baza SQLite, migracje, podstawowy UI.
2. Model danych: Book, BookEdition, PhysicalCopy, DigitalCopy, Shelf.
3. Dodawanie książki ręczne oraz przez ISBN.
4. Adapter Open Library.
5. Adapter Google Books.
6. Adapter LubimyCzytać.pl zostaje wyłączony: bez żądań HTTP i bez parsera HTML, dopóki serwis nie wyrazi pisemnej zgody. Wcześniejsza wersja tego punktu zakładała scraper i była sprzeczna z regulaminem.
7. Skanowanie kodu kreskowego.
8. Wyszukiwanie lokalne.
9. Półki ręczne i inteligentne.
10. Katalog plików e-booków, kopiowanie plików i wykrywanie duplikatów.
11. Backup bazy i plików.
12. Eksport i import biblioteki.
13. Synchronizacja przez WebDAV lub folder lokalny.
14. Testy, poprawki UX, dokumentacja.

## Definition of Done

Aplikacja jest gotowa jako MVP, gdy użytkownik może:

- utworzyć katalog biblioteki;
- dodać książkę przez ISBN lub skan kodu kreskowego;
- zobaczyć metadane z Open Library oraz z Google Books po podaniu własnego klucza API; LubimyCzytać pozostaje niedostępne, dopóki nie ma pisemnej zgody serwisu;
- wybrać najlepsze dopasowanie;
- dodać egzemplarz papierowy do półki;
- dodać plik EPUB lub PDF i automatycznie umieścić go w podkatalogu książki;
- wyszukać książkę po tytule, autorze, ISBN, tagu lub nazwie pliku;
- utworzyć kopię zapasową bazy i plików;
- przywrócić dane z kopii zapasowej;
- wyeksportować i zaimportować całą bibliotekę;
- korzystać z aplikacji bez połączenia z internetem.

Nie dodawaj funkcji społecznościowych, rekomendacji AI, czytnika e-booków ani integracji z księgarniami w MVP. Skup się na niezawodności, prywatności, jakości metadanych i bezpiecznym zarządzaniu plikami.