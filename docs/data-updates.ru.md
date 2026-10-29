# Обновление редко меняющихся данных

[English](data-updates.md) · **Русский**

Часть данных Pico берёт не из кода, а из внешних справочников: названия мест,
список тегов EXIF, лицензии зависимостей. Справочники меняются медленно, так
что обновлять их достаточно примерно раз в год или перед крупным релизом.
Программа при работе в сеть не ходит: всё скачивается здесь, на машине
разработчика, и попадает в репозиторий готовыми файлами.

Нужны `curl`, `unzip` и Python 3 (только стандартная библиотека). Скачанное
складывается в `target/data-download/` и в git не попадает.

## Названия мест (Places)

```sh
make places
```

Что делает: скачивает с GeoNames `cities1000.zip` (все населённые пункты от
1000 жителей) и `countryInfo.txt` (названия стран), с Natural Earth — контуры
стран, морей и океанов, континентов (`ne_50m_admin_0_countries`,
`ne_10m_geography_marine_polys`, `ne_50m_geography_regions_polys`), и
упаковывает всё скриптом `crates/pico-core/data/make_places.py` в
`crates/pico-core/data/places.bin`. Файл встраивается в программу при сборке
(`crates/pico-core/src/geo.rs`). Формат файла описан в начале скрипта.

Потом:

1. Скрипт пишет, сколько городов вышло и какой размер. Сейчас это около
   162 000 городов и 3,5 МБ; заметно больше или меньше — повод разобраться,
   не поменялся ли формат источников.
2. `make check` — тесты `geo.rs` проверяют, что файл читается и известные
   точки находятся (город, страна, море, континент).
3. Запустить Pico и открыть раздел **Places** на фото с GPS.
4. Если сменились источник или лицензия — поправить шапку
   `crates/pico/assets/make_licenses.py` (раздел DATA AND RECIPES), выполнить
   `make licenses` и поправить абзац о данных в `README.md` и `README.ru.md`.
5. Закоммитить `places.bin` (и скрипт, если менялся).

Если Natural Earth переименует поля в GeoJSON (`ISO_A2_EH`, `NAME_EN`, `name`,
`FEATURECLA`), скрипт упадёт с `KeyError` — нужно поправить имена полей в
`main()` скрипта.

## Теги EXIF (панель Properties)

```sh
make exif-tags
```

Что делает: скачивает справочник тегов exiv2 (<https://exiv2.org/tags.html>,
группы Image, Photo, Iop) и таблицу GPS-тегов ExifTool
(<https://exiftool.org/TagNames/GPS.html>) и пишет скриптом
`crates/pico-core/data/make_exif_tags.py` файл
`crates/pico-core/src/exif_tags.rs`: номер тега, имя как в XMP
(`ExposureTime`) и название для людей (`Exposure Time`). GPS-теги хранятся как
0x1_0000 + номер, Interoperability — как 0x2_0000 + номер. Руками этот файл не
правят.

Потом:

1. `git diff crates/pico-core/src/exif_tags.rs` — обычно добавляется несколько
   новых тегов (новые версии DNG, EXIF). Если пропало много строк — скорее
   всего поменялась вёрстка страницы, и регулярные выражения в
   `exiv2_tags()`/`gps_tags()` надо подогнать.
2. Названия разбиваются на слова автоматически (`ExposureTime` →
   `Exposure Time`). Если вышло неудачно (`T4Options` → `T 4 Options`), добавить
   исключение в словарь `WORDS` скрипта. Теги, которых нет в exiv2, — в список
   `EXTRA`.
3. `make check` и посмотреть **Properties** на нескольких фото.
4. Закоммитить `exif_tags.rs` и скрипт, если менялся.

Не генерируются, а написаны руками в `crates/pico-core/src/exif.rs`:

- `tag_name()` — более привычные названия самых частых тегов (они главнее
  таблицы из `exif_tags.rs`);
- `choice()` — расшифровка кодов (Exposure Program, Metering Mode, Light
  Source, Scene Type…), по [таблице EXIF ExifTool](https://exiftool.org/TagNames/EXIF.html);
- `describe()` — единицы и форматы (выдержка, диафрагма, фокусное, вспышка).

Их стоит сверить с таблицей ExifTool, когда выходит новая версия стандарта
EXIF (CIPA DC-008) — в ней могут появиться новые значения кодов.

Остальные форматы (XMP, IPTC, ICC, PNG, WebP) разбираются по спецификациям,
ссылки на них — в разделе «Для разработчиков» `README.md`. Они почти не
меняются, отдельного обновления не требуют.

## Заметки производителей камер (MakerNote)

```sh
make maker-tags
```

Что делает: скачивает ExifRead (библиотеку чтения EXIF на Python под
BSD-3-Clause, <https://github.com/ianare/exif-py>; версия — `EXIFREAD` в
`Makefile`), и скрипт `crates/pico-core/data/make_maker_tags.py` переносит её
таблицы производителей (Canon, Nikon, Olympus, Fujifilm, Sony, Casio, Apple,
DJI) в `crates/pico-core/src/maker_tags.rs`: названия записей и значения.
Таблицы — словари на Python; они читаются через `ast` и не выполняются.

Pico читает заметки в `crates/pico-core/src/maker_notes.rs`. Написанное там
руками главнее: названия записей, которые сравниваются с EXIF, и значения, для
которых нужна арифметика (EV, ISO и выдержки Canon, часы и часовой пояс
Nikon, каталоги Olympus). Сгенерированные таблицы дополняют остальное.

Потом:

1. `git diff crates/pico-core/src/maker_tags.rs` — новые камеры в Model ID
   Canon, новые записи.
2. ExifRead — не ExifTool: если таблица расходится с тем, что пишут камеры
   (в AF Info 2 у Canon поля сдвинуты на одно), убрать её или обрезать в
   `TABLES` / `LAST_FIXED` скрипта.
3. `make check` и посмотреть **Properties** на фото этих производителей.
4. Закоммитить `maker_tags.rs` и скрипт, если менялся.

## Лицензии

```sh
make licenses
```

Пересобирает `crates/pico/assets/licenses.txt` (список в **Help → About**)
из `cargo metadata`. Запускать после любой смены зависимостей в `Cargo.toml`
или `cargo update`, затем закоммитить `licenses.txt`.

## Совпадение полей из разных метаданных (значок «!»)

Одно и то же — название, автора, дату — фото может хранить сразу в EXIF, XMP,
IPTC и текстовых блоках PNG. Когда эти поля есть в нескольких местах и
расходятся, панель **Properties** ставит у каждого значок «!», а подсказка
показывает, что написано в каждом. Правила — в `crates/pico-core/src/agree.rs`
(`GROUPS`).

Группы полей, которые должны совпадать:

| Что | EXIF | XMP | IPTC | PNG | Как сравнивается |
|---|---|---|---|---|---|
| Название | Title, Image Title | Title | Title | — | текст |
| Описание | Description | Description | Caption/Abstract | Description | текст |
| Автор | Artist, Author, Photographer | Creator | By-line | Author | текст |
| Права | Copyright | Rights | Copyright Notice | Copyright | текст |
| Ключевые слова | Keywords | Subject | Keywords | — | набор слов |
| Дата съёмки | Date Taken | Date Taken, Date Created | Date Created | — | дата |
| Дата оцифровки | Date Digitized | Create Date, Date Digitized | — | — | дата |
| Дата изменения | Date Modified | Modify Date | — | Modified | дата |
| Программа | Software | Creator Tool | Originating Program | Software | текст |
| Оценка | Rating | Rating | — | — | число |
| Объектив | Lens Model | Lens Model, Lens | — | — | текст |

Заметки производителя камеры (MakerNote, разбираются в
`crates/pico-core/src/maker_notes.rs`) видны в разделе EXIF под именем
производителя и сверяются с EXIF:

| Что | EXIF | Заметки производителя | Как сравнивается |
|---|---|---|---|
| Выдержка | Exposure Time, Shutter Speed Value | Canon Exposure Time | до 1/3 ступени |
| Диафрагма | F Number, Aperture Value | Canon F-Number | до 1/3 ступени |
| ISO | ISO | Canon ISO, NIKON ISO | до 15% |
| Фокусное расстояние | Focal Length | Canon Focal Length | до 3% |
| Поправка экспозиции | Exposure Compensation | Canon Exposure Compensation | до 0,2 EV |
| Серийный номер | Body Serial Number | Canon, NIKON, OLYMPUS Serial Number | цифры |
| Владелец | Camera Owner | Canon Owner Name | текст |
| Объектив | Lens Model | Canon Lens Model, OLYMPUS Lens Model | текст |
| Камера | Model | Canon Image Type | одно название внутри другого |
| Часы | Date Taken | NIKON Power Up Time | включена до снимка |

Списки Canon начинаются со своей длины; если это не так, редактор переложил
заметки, не поправив смещения, и строка Maker Note об этом говорит.

Программы Adobe копируют сказанное в заметках в свойства XMP `aux:`: Serial
Number, Lens Serial Number, Owner Name и Firmware тоже сверяются с заметками
и EXIF (серийные номера — по буквам и цифрам, прошивка — по номеру версии).
Остальные копии тегов EXIF в XMP могут быть записаны по-своему («2.20» и
«220», «Inches» и «Inch») и при этом совпадать.

Кроме того, каждое свойство XMP из пространств `exif:` и `tiff:` — копия
одноимённого тега EXIF (`exif:ExposureTime` ↔ Exposure Time) и сравнивается с
ним: как дата, если название начинается с «Date», иначе как текст. Исключение —
широта и долгота GPS (`NOT_ALIKE`): EXIF и XMP записывают их по-разному.

Как сравнивается:

- **текст** — без учёта регистра и лишних пробелов;
- **набор слов** — слова через запятую или точку с запятой, порядок и повторы
  не важны;
- **дата** — по цифрам до секунды и только насколько точны обе записи
  (`2018:02:09 19:17:10` = `2018-02-09T19:17:10.57+02:00`; часовой пояс не
  учитывается);
- **число** — оценка звёздочками или цифрой (`★★★☆☆ (3)` = `3`);
- **до …** — числа, расходящиеся не больше, чем округляет сама камера.

Сравниваются только поля, которые есть; одно поле без пары значка не получает.
Чтобы добавить группу, допишите её в `GROUPS` — подписи должны совпадать с
теми, что видны в Properties, — и, по желанию, случай в тест
`finds_fields_that_disagree_across_kinds`.
