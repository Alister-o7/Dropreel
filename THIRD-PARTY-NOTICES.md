# Third-party components

DropReel's own source code is under the MIT License (see `LICENSE`). The Windows packages
bundle the components below, each under its own license.

## mpv — `libmpv-2.dll`

- Project: https://mpv.io — source: https://github.com/mpv-player/mpv
- Windows build: https://github.com/shinchiro/mpv-winbuild-cmake (the exact download URL is
  recorded in `licenses/libmpv.source.txt`)
- License: mpv is GPLv2-or-later; it can be built as LGPLv2.1-or-later, but these builds
  include GPL components, so treat the DLL as **GPLv2+**.

DropReel loads `libmpv-2.dll` at run time. When you distribute a package that contains it,
the GPL applies to that package as a whole. DropReel's MIT license is GPL-compatible, and its
complete source is available at https://github.com/alister-o7/dropreel, which satisfies the
source requirement for DropReel. Sources for mpv and its dependencies are at the links above.

## FFmpeg — `ffmpeg.exe`, `ffprobe.exe` and the `av*.dll`/`sw*.dll` libraries

- Project: https://ffmpeg.org — source: https://git.ffmpeg.org/ffmpeg.git
- Windows build: https://github.com/BtbN/FFmpeg-Builds ("gpl-shared" variant, which includes x264,
  x265 and the NVENC/AMF/QSV hardware encoders; the exact URL is in
  `licenses/ffmpeg.source.txt`)
- License: **GPL** (the build's license text is included as `licenses/ffmpeg-LICENSE.txt`).

DropReel runs `ffmpeg.exe` and `ffprobe.exe` as separate programs; they are not linked into
DropReel.

## Inter (font)

- © The Inter Project Authors — https://github.com/rsms/inter
- License: SIL Open Font License 1.1 (`licenses/LICENSE-Inter.txt`)
- Subset to remove Private Use Area glyphs (see `assets/fonts/README.md`).

## Phosphor Icons (icon font)

- © Phosphor Icons — https://phosphoricons.com
- License: MIT (`licenses/LICENSE-Phosphor.txt`)
- The DropReel app icon is drawn from Phosphor's "film-reel" icon.

## Rust crates

DropReel is built from many open-source crates (egui/eframe, rusqlite with a bundled SQLite,
reqwest, notify, rayon and others), almost all under MIT and/or Apache-2.0. The packaging
script writes the full list with licenses to `licenses/rust-crates.txt`; `Cargo.lock` pins the
exact versions.
