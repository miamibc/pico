# Pico — задания

## Отложено на будущее

Решение от 2026-10-26: пока не делаем, вернуться позже.

- **Люди** (People). Поиск лиц на фото, имена, альбомы людей. Формат уже
  известен: `faces=rect64(...),<id>` и секция `[Contacts2]` в `.picasa.ini`
  (см. `docs/research/picasa3-analysis.md`); Pico эти данные читает и
  сохраняет, но не показывает и не ищет лица.
- **Печать** (Print, Print Contact Sheet). Раскладка по листам, размеры
  отпечатков, печать через систему.

## Сделано 2026-10-26

- Системный диалог выбора папки (`rfd`: xdg-portal / zenity, Windows, macOS).
- Корзина на Windows (PowerShell, Recycle Bin) и macOS (Finder). Проверено
  только построение команд: запустить здесь нельзя.
- Сборки: Windows `.exe` с Linux (`make windows`, проверено под wine с
  Vulkan), macOS и всё остальное — GitHub Actions.
- Хронология (Timeline) и места (Places, без карты: карта — сетевая функция).
- Фильм из фото (Create → Movie).

## Сделано 2026-10-27

- Коллаж (Create → Picture Collage): Mosaic, Frame Mosaic, Grid, Contact
  Sheet, Multiple Exposure. Вид «Picture Pile» (рассыпанные фото) по решению
  пользователя убран.

- Folder Manager как в Picasa: дерево диска, для папки — следить или
  исключить (ближайшее правило главнее), ввод пути и системный диалог.
- Релизы в GitHub Actions (`release.yml`, по тегу `v*`): Linux x86_64, x86,
  arm64, armv7 (zigbuild, glibc 2.28), Windows x86_64, x86, arm64, macOS
  universal `.app` (без подписи Apple). Linux-сборки проверены локально;
  Windows и macOS проверятся только при первом запуске в Actions.

## Идеи на потом

- Карта для Places без сети (офлайн-подложка) или по явному разрешению.
- Под wine `pico.exe` падает на DirectX 12 (компилятор шейдеров wine), с
  Vulkan работает. На настоящей Windows сборка ещё не проверялась; если
  DirectX 12 подведёт и там (слабые или виртуальные машины), переключаться на
  Vulkan или OpenGL.
