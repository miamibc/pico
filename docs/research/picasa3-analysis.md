# Picasa 3.9 — анализ для Pico

Цель: чистая (clean-room) реализация поведения Picasa 3 в Pico (Rust + egui).
Строки, иконки и ресурсы Google в Pico не копируются; файлы в этой папке —
только справочник.

Источник: `~/.wine/drive_c/Program Files/Google/Picasa3/` (сборка 2014-03-11, PE32 i386).

| Файл | Что это |
|---|---|
| `Picasa3.exe` (10 МБ) | Основное приложение, собственный UI-движок (GDI/DirectDraw, шрифты `runtime/*.ytf`) |
| `Picasa3i18n.dll` (27 МБ) | Только ресурсы: ~3000 XML (`XMLF`) со строками на ~40 языках |
| `PicasaPhotoViewer.exe` | Отдельный быстрый просмотрщик |
| `MovieThumb.exe` | Вспомогательный процесс: кадры из видео (AVIFile + DirectShow) |
| `qtsupport.dll` | Мост к Apple QuickTime для `.mov` |

REA Evidence: импорты `ev_2647b822…`, ресурсы i18n `ev_f8892e70…`.

## Файлы данных в этой папке

- `picasa-filters.txt` — все фильтры редактора: ID в `.picasa.ini`, название, параметры.
- `picasa-menus.tsv` — пункты всех меню и контекстных меню.
- `picasa-shortcuts.txt` — горячие клавиши.
- `picasa-strings-en.tsv` — все английские строки интерфейса (4573 шт.).

## Форматы изображений и видео

- Фото: jpg/jpeg, png, gif, bmp, tif/tiff, psd, webp, tga.
- RAW: 3fr arw cr2 crw dcr dng erf kdc mef mrw nef nrw orf pef raf raw rw2 sr2 srf x3f.
- Видео: 3g2 3gp asf avi divx m2t m2ts m4v mkv mmv mod mov mp4 mpeg mpg mts mxf tod wmv.
- Каждый тип включается в настройках (`SupportBMP`, `SupportPSD`, `SupportTIF`,
  `SupportWEBP`, `SupportPNG`, `SupportGIF`, `SupportMovies`).

Видео в Picasa: миниатюры, проигрывание (`/` пауза, `,` назад, `.` вперёд),
обрезка начала/конца (фильтры `moviestart`/`movieend`, без перекодирования),
снимок кадра в JPEG (`%s.%d.jpg`), «Создать фильм» из фото и видео с экспортом
(`%s.%d.wmv`, `%s.%d.mov`). Декодирование через кодеки ОС.

## `.picasa.ini`

Лежит в каждой папке с фото. INI, CRLF, при наличии не-ASCII символов — секция
`[encoding] utf8=1`. Строки форматов найдены в `Picasa3.exe`:

```ini
[Picasa]
name=%s
description=%s
location=%s
category=%s          ; например "Folders on Disk"
P2category=%s        ; пример с диска: "Exported Pictures"
date=%f              ; OLE Automation date (дни с 1899-12-30), напр. 43880.743600

[.album:<token>]     ; альбомы Picasa (виртуальные); токен = hex-id
name= / date= / location= / category= / description= / token=

[Contacts2]          ; люди для распознавания лиц: <contactid>=<Имя>;;

[<имя файла>]
star=yes
caption=%s
keywords=%s          ; через запятую
rotate=rotate(%d)    ; 0..3 = ×90°
crop=rect64(%s)      ; старый формат
filters=<id>=<params>;<id>=<params>;...   ; стек правок, формат "%s=%s;"
faces=rect64(%I64x),<contactid>;...      ; лица
albums=<token>,<token>
backuphash=%s
redo=...             ; отменённые правки для повтора
moddate=...
textactive=...
IIDLIST_%s=%s
```

`rect64(hex)` — 64-битное число: 4 по 16 бит (left, top, right, bottom),
каждое — доля ширины/высоты, умноженная на 65535 (общеизвестное описание, по
бинарнику не проверено).

### Формат `filters=` (подтверждён декомпиляцией)

Сериализатор — `FUN_008fdab0`, разбор описаний фильтров — `FUN_009023c0`
(`research/picasa3/decomp/functions/`). Описания всех фильтров лежат в
`Picasa3/runtime/filterdesc.xml`. Сгенерированная таблица параметров —
`picasa-filter-layout.tsv` (скрипт `research/scripts/filter_layout.py`).

Каждая правка: `<id>=<0|1>` (флаг `%c`), затем параметры через запятую, конец `;`:

1. `,%f,%f` — позиция курсора-«шайбы» (центр эффекта), если `<cursor persist="1">`.
2. `,%f` — слайдеры 0, 1, 2 (если у слайдера нет `persist="0"`).
3. `,%08x` — цвет, если есть `<colorcircle>` (пипетка нейтрального цвета).
4. `,%08x` — по цвету на каждый `<colorwheel>`.
5. `,%f` — слайдер 3.
6. `,%d` — поворот 0..3, если у фильтра `rotate="1"` (Graduated Tint).
7. Затем список мазков/областей (формат `,%f:%g:%g:%d[:%g:%g][:%g|%g|…]`),
   какие фильтры его используют — не выяснено.

Особые случаи: `crop64=1,<hex>` — 64 бита, 4 поля по 16 бит, слева направо
(старшие биты): left, top, right, bottom, доли от 65535; `rot=1,<%d>`.

Пример: `finetune2=1,0.000000,0.000000,0.190000,fff7e7e6,0.000000;` =
осветление, светлые, тени, нейтральный цвет, цветовая температура.

### Проверено на реальных `.picasa.ini` с диска (2026-10-26)

Найдено 9 файлов, правки — в трёх. Примеры (имена файлов опущены):

```
filters=crop64=1,b042c64ffff9a6e;finetune2=1,0.134503,0.053333,0.101053,00000000,0.333333;unsharp=1;
filters=unsharp=1;crop64=1,a5c0050fb46f104;finetune2=1,0.561404,0.148772,0.188070,ff686761,-0.450292;sat=1,0.005848;
filters=...;tint=1,255.000000,fffdfd69;...;tilt=1,0.691700,0.000000;crop64=1,e2c0c2dffff9797;
filters=...;radblur=1,0.500000,0.500000,1.000000,-0.497076;...
filters=...;glow=1,0.650000,3.000000;
filters=crop64=1,a4f0a44f56ef546;HDR=1,6.362573,1.771930,19.298244;...;Vignette=1,50.000000,1.400000,84.210526,00000000;
crop=rect64(b042c64ffff9a6e)
rotate=rotate(3)
moddate=ba8726788231d401
backuphash=60720
```

Выводы:
- Порядок полей совпадает с таблицей (`finetune2` — 3 слайдера, цвет, слайдер 3;
  `radblur` — x, y центра, затем 2 слайдера; `tint` — слайдер, цвет).
- Значения хранятся уже в шкале UI: от `-offset` до `range-offset`
  (`sat=-0.0175`, температура `-0.45`, сохранение цвета `tint` = 255).
- Параметры могут отсутствовать: `unsharp=1;` без значения (пункт меню
  «Повысить резкость» с настройками по умолчанию). Читать нужно с подстановкой
  значений по умолчанию из `filterdesc.xml`.
- Одна и та же правка может встречаться в стеке несколько раз (`finetune2`,
  `crop64` повторяются) — это история, применяется по порядку.
- `crop64` пишется без ведущих нулей (`cbc3ffff` = left 0, top 0, right 0xcbc3,
  bottom 0xffff). Последний `crop64` продублирован ключом `crop=rect64(...)`.
- Picnik-эффекты тоже сохраняют параметры: значения контролов MXML в порядке
  документа, затем цвета (`HDR` = radius, contrast, fade; `Vignette` = 3 числа + цвет).
- `moddate` — FILETIME в hex, байты little-endian.
- В `.picasaoriginals/.picasa.ini` встречаются `width`, `height`, `moddate`,
  `textactive`.

**Всё ещё не проверено:** формат `redo=`.

### Обрезка видео: `moviestart` / `movieend` (из кода)

Это не общий фильтр, а класс `CTimeFilter` (наследник `CSimpleFilter`, vtable
`0xc7af2c`). Параметр — одно 64-битное время, формат `%I64x` (`FUN_0044d670`),
флага `0/1` нет:

```
filters=moviestart=<hex>;movieend=<hex>;
```

Время в единицах по 100 нс (как DirectShow `REFERENCE_TIME`): код плеера делит
его на 10 000 000, чтобы получить секунды (`0x78f6c0`, `0x6986e0`). Пример:
5 секунд = `2faf080`. Чтение — `FUN_0049c670`, `FUN_0049ce00` (ищут
`moviestart=` в строке и парсят `%I64x`).

### Лица: `faces=` и `[Contacts2]` (из кода)

`FUN_004d2550`:

```
faces=rect64(<hex>),<contact id hex>;rect64(<hex>),<contact id hex>
```

`rect64` кодируется так же, как `crop64` (left, top, right, bottom по 16 бит,
доли от 65535). Лица без имени в INI не пишутся.

Имена людей — секция в том же `.picasa.ini` (`FUN_00637ed0`):

```
[Contacts2]
<contact id hex>=<полное имя>;<email>;<gaia_id>
```

Глобальный список контактов Picasa хранит в `contacts.xml`. Внутренняя база
(`imagedata_facerect.pmp`, `facetemplatesV2_*.db`) — это кэш распознавания, для
Pico он не нужен. На диске пользователя данных о лицах в `.picasa.ini` не нашлось.

### Picnik-эффекты: откуда брать смысл параметров

1. **`filterdesc.xml`** описывает каждый эффект полностью: контролы с
   `id`, диапазонами и значениями по умолчанию, и операцию с формулами, куда
   эти значения подставляются. Пример:

   ```xml
   <HSliderFastDrag minimum="0" maximum="50" value="35" id="_sldrBlur"/>
   <HSliderFastDrag minimum="1" maximum="2" value="1.4" id="_sldrStr"/>
   <ImageColorSwatch id="_clrsw" color="0x000000"/>
   <HSliderPlus minimum="0" maximum="100" value="0" id="_sldrFade"/>
   <imageOperations:GlowImageOperation color="{_clrsw.color}"
       xblur="{_sldrBlur.value*.02*Math.max(imagewidth, imageheight)/4}" ...
       strength="{_sldrStr.value}" innerglow="true"
       BlendAlpha="{1-(_sldrFade.value/100)}"/>
   ```

   В INI пишутся значения слайдеров в порядке документа, затем цвета:
   `Vignette=1,50.0,1.4,84.2,00000000` = blur, strength, fade, цвет (подтверждено
   на одном реальном примере).
2. **`Picasa3.exe`** содержит таблицу понятных имён контролов
   (`_sldrFade` → «Fade», `_sldrSteps` → «Number of Colors»…), свой интерпретатор
   выражений (`Math.min`, `Math.pow`, `imagewidth`, `zeroR`…) и нативные
   реализации операций.
3. **Исходники Picnik открыты** (Google, Apache 2.0): клон
   <https://github.com/darrinm/PicEditor>, локально `research/picnik-src/`
   (в .gitignore). В `client/imagine/imageOperations/` — 73 класса
   `*ImageOperation.as` с алгоритмами (Glow, LocalContrast, Pixelate,
   QuantizePalette, IR…). Лицензия позволяет портировать код в Pico с
   сохранением уведомления об авторстве (NOTICE).

### Другие файлы `Picasa3/runtime/`

`filterdesc.xml` (фильтры), `filters.txt` (исключаемые при сканировании папки:
windows, winnt, temp, Program Files, Originals), `*.fen` (разметка диалогов),
`constants.ui`, `properties.xml`, `respack.yt` (графика интерфейса). Копия —
`research/picasa3/runtime/` (в .gitignore).

При сохранении (`Save`) оригинал переносится в подпапку `.picasaoriginals/`,
правки «запекаются» в файл.

## Внутренняя база (не обязательно для Pico)

`AppData/Local/Google/Picasa2/db3/` — столбцовое хранилище: по файлу `.pmp` на
поле (`imagedata_caption.pmp`, `imagedata_crop64.pmp`, `albumdata_name.pmp`…),
`thumbs*_index.db`, `bigthumbs_index.db`, `facetemplatesV2_*.db`. Источник
истины для пользователя — `.picasa.ini`; база — кэш. Список отслеживаемых папок:
`Picasa2Albums/watchedfolders.txt`, исключения: `frexcludefolders.txt`.

## Функции (по меню)

- **Библиотека:** дерево папок / плоский список, сортировка (дата, имя, размер,
  недавние изменения), альбомы, люди (лица), места (гео), временная шкала, поиск,
  фильтр по звёздочкам, видео, лицам, скрытым; цветные метки; «лоток» (Hold).
- **Просмотр:** маленькие/обычные миниатюры, подписи (имя, разрешение, теги,
  подпись), слайд-шоу, полноэкранный режим, свойства EXIF.
- **Файлы:** добавить папку или файл, импорт, переименование (пакетное),
  перемещение, удаление, «Показать на диске», сохранить, сохранить копию,
  экспорт в папку с изменением размера, отмена всех правок.
- **Редактор:** вкладки «Основные» (кадрирование, выравнивание, красные глаза,
  «Мне повезёт», автоконтраст, автоцвет, ретушь, текст, осветление), «Настройка»
  (осветление, светлые, тени, цветовая температура), «Эффекты» (повышение
  резкости, сепия, Ч/Б, утепление, зерно, тонирование, насыщенность, мягкий фокус,
  свечение, фильтрованное Ч/Б, фокальное Ч/Б, градиентное тонирование) и
  набор Picnik-эффектов. Копирование и вставка всех эффектов, отмена и повтор.
- **Инструменты:** поиск дубликатов, изменение даты и времени, резервное
  копирование, менеджер папок, менеджер людей, геотеги, запись лиц в XMP,
  коллаж, постер, контактный лист, HTML-экспорт, фильм, печать.

## Не переносим (по решению пользователя)

Всё сетевое: Picasa Web / Google+, YouTube, Blogger, FTP, e-mail, заказ
печати, обновления, онлайн-синхронизация, импорт из облака.
