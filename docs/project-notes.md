# Pico — project notes

**English** · [Русский](project-notes.ru.md)

Pico is a Rust rewrite of Google Picasa 3 with small changes and
improvements. Builds for Linux, Windows, possibly macOS; few dependencies,
simple to build and run.

## Decisions

- **UI:** egui.
- **Texts:** English only for now. A little humor is welcome, to stand apart
  from the original. Google's strings and resources are not copied.
- **Metadata:** we support the `.picasa.ini` files (and the old `Picasa.ini`)
  that Picasa leaves in every photo folder. The format is in
  `docs/research/picasa3-analysis.md`.
- **Network:** no network features for now, nothing is uploaded to the cloud.
- **Video:** read and processed with ffmpeg.
- **Formats:** support every photo and video format we can.
