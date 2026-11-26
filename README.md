# Library Studio

Prywatny katalog książek papierowych i elektronicznych. Baza, pliki i klucze zostają na tym komputerze.

## Uruchomienie

Wymagane: Node.js, Rust (stable) i Visual Studio Build Tools z pakietem C++. WebView2 jest w Windows 11.

```bash
npm install
npm run tauri dev
```

Testy (TypeScript, Vitest i Rust razem):

```bash
npm run check
```

Instalator Inno Setup:

```bash
npm run installer
```

## Co robi ta wersja

- dodawanie książki po ISBN, tytule, skanie kodu albo ręcznie, także seryjnie (kamera albo czytnik kodów, wiele książek naraz)
- metadane z Open Library, Biblioteki Narodowej i z Google Books, gdy podasz własny klucz
- wyszukiwanie okładki w Open Library i Google Books albo wklejenie adresu obrazu
- pełna edycja metadanych: podtytuł, tłumacze, seria i tom, gatunki, ISBN, liczba stron
- egzemplarz papierowy, półka, miejsce, wypożyczenie z terminem zwrotu i przypomnieniem o zaległych
- kopia pliku EPUB albo PDF do folderu biblioteki, skrót SHA-256 i weryfikacja
- wyszukiwanie pełnotekstowe (FTS5) z sortowaniem po trafności i filtrami: autor, seria, tag, rok
- widok listy albo siatki okładek
- półki zwykłe (przeciąganie, kolejność) i z filtrem
- kosz z przywracaniem i „Cofnij” po usunięciu
- wykrywanie i scalanie duplikatów
- import z Calibre, eksportu Goodreads i CSV; eksport do CSV; druk spisu i etykiet z kodem EAN-13 (także do PDF)
- statystyki czytania i cel roczny
- kopia bazy i plików, przywracanie, eksport i import
- synchronizacja folderem lokalnym albo WebDAV
- język polski i angielski, przełącznik jest w ustawieniach
- metadane i okładki: LubimyCzytać (API wyszukiwarki, jak w programie Czytnik), UpolujEbooka, Biblioteka Narodowa, Open Library i Google Books (działa bez klucza, z dziennym limitem); okładki także z Empiku i Apple Books. Każde źródło można wyłączyć w ustawieniach, a do jednego serwisu program pyta najwyżej co 1,5 s

Przed migracją schematu program zapisuje kopię bazy obok niej (`biblioteka-przed-v3-….sqlite`), a przed przywróceniem kopii zapisuje bieżącą bazę w `kopie-przed-przywroceniem`.

Baza SQLite powstaje przy pierwszym uruchomieniu okna, w katalogu danych aplikacji (`biblioteka.sqlite`), w trybie WAL. Schemat jest w `src-tauri/migrations`.

## Licencja

MIT, zobacz plik [LICENSE](LICENSE).
