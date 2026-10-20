# DropReel — Build Spec

Build **DropReel**, a Windows desktop movie library and player written in Rust.

## Goal
You drag movie files (or folders) into the window and they join your library. DropReel stores each file's location on disk, so clicking a poster plays the movie in the built-in player. Any movie larger than 5 GB gets a suggestion to re-encode it to HEVC at 5 GB or less, which you can start with one click.

## Platform & stack
- **Target:** Windows 10/11 x64 only.
- **Language:** Rust, stable toolchain.
- **GUI:** `eframe`/`egui` with the `glow` (OpenGL) backend.
- **Playback:** embedded **libmpv** through the `libmpv2` crate's OpenGL render API, drawn inside the app window. Bundle `libmpv-2.dll`.
- **Media tools:** bundled `ffprobe.exe` and `ffmpeg.exe`, used for metadata, thumbnails and encoding. The user can override their paths in Settings.
- **Storage:** SQLite through `rusqlite` with the `bundled` feature, in `%APPDATA%\DropReel\library.db`. Posters and thumbnails are cached in `%APPDATA%\DropReel\cache\`.
- **Other crates:** `notify` (folder watching), `serde` + `toml` (settings), `reqwest` (TMDb), `trash` (Recycle Bin), `tracing` (logging).

## Features (v1)

### 1. Library & import
- Drag and drop **files or folders** onto the window. Folders are scanned recursively.
- Accepted extensions: mkv, mp4, m4v, avi, mov, wmv, webm, ts, m2ts.
- For each file, store: absolute path, file size, duration, video codec, resolution, audio and subtitle tracks (from `ffprobe`), date added, and a fingerprint (size + hash of the first and last 1 MB) used to find duplicates and moved files.
- Skip duplicates and tell the user which ones were skipped.
- **Watched folders:** the user can add folders in Settings. New files that appear in them are imported automatically.
- **Missing files:** if a stored path no longer exists, show the entry greyed out with a "Locate…" action. That action re-links the entry, and it can also find the file automatically by fingerprint inside watched folders.

### 2. Browsing
- The default view is a **poster grid**, with a toggle to a sortable **list view**.
- Search by title. Sort by title, year, size, date added and last played.
- Users can create **custom tags**, assign them to movies, and filter by tag.
- The detail panel shows the poster, title, year, plot (if available), file path, size, codec and resolution, plus buttons for Play, Re-encode, Open folder, Edit tags and Remove from library. Removing never deletes the file.

### 3. Metadata (optional TMDb)
- With no API key, the title and year are parsed from the filename (for example `Movie.Name.2019.1080p.BluRay.mkv` → "Movie Name" (2019)). The poster is a frame grabbed with ffmpeg at about 10% of the runtime.
- With a TMDb API key set in Settings, the app fetches the poster, year and plot automatically. The user can fix a wrong match with "Search TMDb…".
- The app must work fully offline.

### 4. Player
- Clicking a title plays it inside the app window. Controls: play/pause, seek bar, volume, audio track, subtitle track, fullscreen (F / double-click) and Esc to exit fullscreen.
- **Resume playback:** save the position every 10 seconds and when the player closes. On the next play, offer "Resume from 1:12:34" or "Start over". Mark a movie as watched when 95% has been played.

### 5. Re-encode for files over 5 GB
- Flag any movie whose file is larger than 5 GB (5 × 10⁹ bytes) with a badge in the grid and a banner in the detail panel: "This file is X GB. Re-encode to HEVC (≤5 GB)?"
- **Target size:** aim for 4.75 GB, leaving headroom for container overhead.
  - Video bitrate = `(target_bits − copied audio/subtitle bits) / duration`.
  - If the copied tracks alone leave too little room for acceptable video, say so before starting.
- **Encoder:** detect the GPU encoders ffmpeg supports, in the order `hevc_nvenc` → `hevc_amf` → `hevc_qsv`, and fall back to `libx265` (2-pass) on the CPU. The user can override the encoder and preset in Settings.
- **Tracks:** copy every audio and subtitle track and the chapters as-is (`-map 0 -c:a copy -c:s copy`). Re-encode only the video. Keep HDR metadata and 10-bit when the source has them.
- Run encodes in a **background queue**, one at a time. Show progress %, ETA and speed, with Cancel. The app stays usable while encoding.
- Write the output next to the source as `<name>.hevc.mkv`, using a temp name until the encode finishes.
- **Verify** the result: it must be under 5 GB, its duration must match within 1 second, and ffprobe must read it without errors. If verification fails, delete the output and leave the original untouched.
- When the encode succeeds, **ask each time** what to do with the original: keep it, move it to the Recycle Bin, or delete it permanently (with a confirmation). The library entry then points at the new file and keeps its tags and resume position.
- Never offer the re-encode for files of 5 GB or less.

### 6. Settings
Settings are stored in `%APPDATA%\DropReel\settings.toml` and cover: watched folders, TMDb API key, ffmpeg/ffprobe paths, encoder and preset override, size threshold (default 5 GB), and theme (dark/light).

## Non-functional
- The UI must never block. Run probing, hashing, network requests and encoding on worker threads that report back through channels.
- Importing 1,000 files must stay responsive. Probe them in parallel with a bounded thread pool.
- Show clear error messages, for example when ffmpeg is missing, a file is unreadable or the network fails. Write logs to `%APPDATA%\DropReel\logs\`.
- Never modify or delete a user's file without an explicit confirmation.

## Project structure (suggested)
```
src/
  main.rs        app bootstrap
  app.rs         egui state, views
  library/       db.rs, import.rs, scan.rs, watch.rs, fingerprint.rs
  metadata/      filename.rs, tmdb.rs, thumbnail.rs
  player/        mpv.rs (render API), controls.rs
  encode/        detect.rs, plan.rs (bitrate math), queue.rs, verify.rs
  settings.rs
```

## Deliverables
1. A Cargo project that builds with `cargo build --release` on Windows. Add a GitHub Actions `windows-latest` workflow that builds it and runs the tests.
2. Unit tests for: filename → title/year parsing, the bitrate/target-size math, the encoder-selection fallback order, and the DB schema migrations.
3. A packaging script that puts the exe, `libmpv-2.dll`, `ffmpeg.exe` and `ffprobe.exe` into a zip or installer.
4. A README covering setup, where the DLLs and binaries come from, and their licenses (mpv and ffmpeg are LGPL/GPL).

## Out of scope (v1)
TV-series/episode handling, streaming or network shares as a server, multi-user support, and macOS/Linux builds.

## Milestones
- [x] **M1 — Core engine (no UI).** SQLite library with migrations, import of files/folders,
  duplicate detection and moved-file re-linking by fingerprint, filename → title/year parsing,
  ffprobe details, frame-grab posters, optional TMDb client, folder watching, encoder detection
  (NVENC → AMF → QSV → x265), bitrate planning, ffmpeg command lines, output verification.
- [x] **M2 — App window & library UI.** Drag & drop, poster grid + list view, search/sort/tags,
  detail panel, missing-file "Locate…", import report, Settings window.
- [x] **M3 — Embedded mpv player.** In-window playback, controls, tracks, fullscreen, resume.
- [x] **M4 — Re-encode queue & UI.** Badge/banner, plan preview + confirmation, background
  queue with progress/ETA/cancel, verification, "what to do with the original".
- [ ] **M5 — Windows packaging.** GitHub Actions build/test, packaging script bundling
  `libmpv-2.dll`/`ffmpeg.exe`/`ffprobe.exe`, README with licenses.
