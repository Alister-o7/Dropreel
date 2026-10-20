# DropReel

[![Windows](https://github.com/Alister-o7/DropReel/actions/workflows/windows.yml/badge.svg)](https://github.com/Alister-o7/DropReel/actions/workflows/windows.yml)

A movie library and player for Windows, written in Rust. Drag movie files or folders onto the
window and they join your library; click a poster and it plays right there. Movies larger than
5 GB get a one-click re-encode to HEVC that brings them under 5 GB.

![Library](docs/screenshots/library.jpg)

## Features

- **Drag & drop library** — drop files or whole folders (`mkv mp4 m4v avi mov wmv webm ts m2ts`).
  Duplicates are skipped, and a movie that was moved or renamed is found again by its
  fingerprint. Watched folders import new movies automatically.
- **Built-in player** (libmpv) — seek bar, audio and subtitle tracks, volume, fullscreen,
  keyboard shortcuts, and **resume** where you left off. Movies are marked watched at 95%.
- **Re-encode to HEVC** — for files over the limit (5 GB by default): a preview with the
  expected size, then a background encode with progress, ETA and cancel. Uses your GPU (NVENC →
  AMF → Quick Sync) when available, otherwise x265. Audio and subtitles are copied untouched,
  HDR10 metadata is kept, and the result is verified before the original is touched. Afterwards
  you choose: keep the original, move it to the Recycle Bin, or delete it.
- **Organise** — poster grid or list, search, sort, shelves (Continue watching, Unwatched,
  Large files…) and your own tags.
- **Posters & metadata** — a frame from the movie by default; optionally from
  [TMDb](https://www.themoviedb.org) with an API key. Works fully offline.
- Dark and light themes.

| Movie details | Re-encode preview |
|---|---|
| ![Details](docs/screenshots/details.jpg) | ![Preview](docs/screenshots/reencode-preview.jpg) |
| **Re-encode finished** | **Player** |
| ![Finished](docs/screenshots/reencode-finished.jpg) | ![Player](docs/screenshots/player.jpg) |

## Install

Download the latest version from the
[Releases page](https://github.com/Alister-o7/DropReel/releases/latest): either the installer
(`DropReel-<version>-setup.exe`, per-user, no admin needed) or the portable zip. Windows 10/11,
64-bit. Builds of every commit are also available as the **DropReel-win64** artifact on the
[Actions page](https://github.com/Alister-o7/DropReel/actions/workflows/windows.yml).

To publish a new release, bump `version` in `Cargo.toml`, add
`docs/release-notes/v<version>.md`, and push a `v<version>` tag — CI builds, tests and attaches
the installer, zip and checksums to a GitHub Release.

Keep these files next to `DropReel.exe` (both packages include them):

| File | What it's for | Without it |
|---|---|---|
| `libmpv-2.dll` | Playing movies inside DropReel | Movies open in your default player instead |
| `ffprobe.exe` | Durations, codecs, tracks | Movies are added without technical details |
| `ffmpeg.exe` | Thumbnails and re-encoding | No frame posters, no re-encoding |
| `av*.dll`, `sw*.dll` | FFmpeg libraries used by `ffmpeg.exe`/`ffprobe.exe` | Neither tool starts |

You can also point DropReel at your own `ffmpeg`/`ffprobe` in **Settings**, or have them on `PATH`.

## Using DropReel

| Action | How |
|---|---|
| Add movies | Drag files/folders onto the window, or **Add movies** / **Add folder** |
| Play | Click a poster (or double-click a row in list view) |
| Details, tags, re-encode | Hover a poster → ⓘ, or right-click it |
| Play/pause · seek ±10 s · volume · mute | `Space` · `←` `→` · `↑` `↓` · `M` |
| Fullscreen | `F` or double-click the video; `Esc` leaves fullscreen, then returns to the library |
| Search | `Ctrl+F` |

The installer also adds DropReel to Explorer's **Open with** menu for video files; opening a
movie that way plays it (and adds it to the library if it's new).

Data lives in `%APPDATA%\DropReel`: `library.db` (SQLite), `settings.toml`, `cache\posters`
and `logs`.

### Troubleshooting

- **Black picture but sound plays** — turn off *Settings → Player → High-quality video
  scaling*. On software OpenGL (VMs, remote desktop) simple scaling is used automatically.
- **mpv diagnostics** — set `DROPREEL_MPV_LOG=C:\path\mpv.log` before starting DropReel to get
  mpv's own log; `DROPREEL_MPV_OPTS="hwdec=no,scale=bilinear"` passes extra mpv options.
- **Re-encode didn't fit under the limit** — GPU encoders aim close to the target but can
  overshoot slightly; choose *x265 (CPU)* in Settings for the most accurate size.
- Logs are in `%APPDATA%\DropReel\logs`. Set `DROPREEL_LOG=debug` for more detail.

## Building from source

Requirements: [Rust](https://rustup.rs) (stable, MSVC toolchain), PowerShell, and 7-Zip or
the `tar.exe` that ships with Windows 10+.

```powershell
git clone https://github.com/Alister-o7/DropReel
cd DropReel
.\packaging\package.ps1            # download libmpv + ffmpeg, build, and package
```

The script's stages can also run separately: `-Stage deps` (download the third-party binaries
into `packaging\deps`), `-Stage build` (`cargo build --release`) and `-Stage package`
(assemble `dist\DropReel\`, zip it, and build the installer if
[Inno Setup 6](https://jrsoftware.org/isinfo.php) is installed).

For development, `cargo run` works once `libmpv-2.dll`, `ffmpeg.exe` and `ffprobe.exe` are
next to the built exe or on `PATH` (e.g. `$env:PATH = "$PWD\packaging\deps;$env:PATH"`).
`cargo test` runs the unit tests plus integration tests that drive a real ffmpeg; those skip
themselves when ffmpeg isn't found. Set `DROPREEL_HOME` to use a separate data folder while
testing.

### Where the bundled binaries come from

- **libmpv-2.dll** — the latest `mpv-dev-x86_64-*.7z` from
  [shinchiro/mpv-winbuild-cmake](https://github.com/shinchiro/mpv-winbuild-cmake/releases)
  (override with `-MpvDevUrl`).
- **ffmpeg.exe / ffprobe.exe** and their DLLs — `ffmpeg-master-latest-win64-gpl-shared.zip`
  from [BtbN/FFmpeg-Builds](https://github.com/BtbN/FFmpeg-Builds/releases) (override with
  `-FfmpegUrl`; a static build works too). The GPL build includes x264, x265 and the
  NVENC/AMF/QSV encoders; the shared variant keeps the package about half the size.

The exact URLs used are written to `licenses\*.source.txt` in each package.

## Licenses

DropReel's own code is under the [MIT License](LICENSE).

The packages bundle third-party software under its own licenses — see
[THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md):

- **mpv** (`libmpv-2.dll`) — GPLv2+ for these builds (mpv itself can be LGPL; the shinchiro
  builds include GPL parts). DropReel loads it at run time, so a package containing it is
  distributed under the GPL; DropReel's MIT license is GPL-compatible and its source is this
  repository.
- **FFmpeg** (`ffmpeg.exe`, `ffprobe.exe` and their DLLs) — GPL (BtbN "gpl-shared" build). Run
  as separate programs.
- **Inter** font — SIL Open Font License 1.1. **Phosphor Icons** — MIT.
- Rust crates — mostly MIT/Apache-2.0; the full list is generated into `licenses\rust-crates.txt`.
