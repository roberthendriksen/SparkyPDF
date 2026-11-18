<img src="assets/rtx-spark-laptop-pdf-icon.png" width="96" alt="Sparky PDF icon">

# Sparky PDF

A small, reliable PDF reader for Windows.

Sparky PDF is a single program of about 1 MB. It draws PDFs with the WebView2
engine that is already part of Windows 11, so there is nothing else to install
and Windows keeps the PDF engine up to date. Around that engine it adds a plain
window, a list of recent files, and help for files that will not open.

## What it does

- **Reads PDFs**: search, text selection and copy, zoom, rotate, table of
  contents, print and save.
- **Explains files that will not open.** If a PDF was cut off by an interrupted
  download or copy, it says so instead of showing a generic error. It also
  recognises files that only carry a `.pdf` name, such as a saved sign-in page.
- **Recovers pages from cut-off scans.** Scanned documents store each page as a
  picture near the start of the file. Sparky PDF lifts out the pages that are
  complete, and the visible part of a page that is not, so they can be read,
  printed or saved as a new PDF.
- **Stays a reader.** The window only ever loads the file you opened. Web links
  inside a PDF are handed to your default browser.

## Status

This is an early version, built and used on Windows 11 on ARM64.

Tested by hand: opening healthy and damaged PDFs, the recovery view, dropping
files on the window, links inside a PDF, printing, full screen, closing a
file, and registering as a PDF app.
The x64 program has only been run under emulation on an ARM64 PC, not on an
x64 machine.

## Requirements

- Windows 10 or 11
- The WebView2 Runtime, which Windows 11 includes

## Getting it

Download from the
[latest release](https://github.com/roberthendriksen/SparkyPDF/releases/latest):

| File | For |
| --- | --- |
| `SparkyPDF-Setup.exe` | Most people. Installer for any 64-bit Windows 10 or 11 PC, ARM or not. Installs for your account only, adds a Start menu entry, and can be removed again from Windows Settings. |
| `SparkyPDF-arm64.exe` | Portable program for Windows on ARM |
| `SparkyPDF-x64.exe` | Portable program for ordinary 64-bit PCs |

A portable program is the whole of Sparky PDF in a single file: put it anywhere
and start it. `SHA256SUMS.txt` lists the checksums.

The downloads are not code-signed, so Windows may show "Windows protected your
PC" the first time. Choose **More info**, then **Run anyway**.

That release page is the only official source of Sparky PDF. [SECURITY.md](SECURITY.md)
explains how to check a download and how to report a security problem.

### Building it yourself

You need [Rust](https://rustup.rs) with the MSVC toolchain and the Visual
Studio Build Tools:

```
cargo build --release
```

The program is then at `target\release\SparkyPDF.exe`.

`scripts\build-release.ps1` builds everything for a release into `dist\`: the
ARM64 and x64 programs, the checksums and, when
[Inno Setup](https://jrsoftware.org/isinfo.php) 6.3 or newer is installed, the
installer described by `installer\SparkyPDF.iss`.

## Using it

Start `SparkyPDF.exe` and open a file, or pass a file on the command line:

```
SparkyPDF.exe "C:\path\to\document.pdf"
```

| Key | Action |
| --- | --- |
| Ctrl+O | Open a file |
| Ctrl+W | Close the file |
| Ctrl+F | Find in the document |
| Ctrl+P | Print |
| F11 | Full screen (Esc leaves it) |

### Making it the default PDF app

Choose **File > Use Sparky PDF for All PDF Files...**, or run:

```
SparkyPDF.exe --register
```

This adds Sparky PDF to Windows' list of PDF apps for your account and opens
Windows Settings, where you pick it for `.pdf`. Windows reserves that last
choice for you; no program can make it on your behalf. The registration points
at the program's current location, so run it again after moving the file.

To remove the registration:

```
SparkyPDF.exe --unregister
```

`--register-quiet` registers without opening Settings; installers use it.

Only your own account is touched (`HKEY_CURRENT_USER`); no administrator rights
are needed.

### Damaged files

A PDF keeps its index, the list of pages and where to find them, at the very
end of the file. When a download stops early, that index is missing and no PDF
engine can open what is left. Sparky PDF checks for this before showing a file.
If the index is gone it scans the file for page pictures stored as JPEG and
shows those instead, together with an explanation.

**File > Recover Pages from Damaged PDF** runs the same scan on a file the
check did not reject. Text and drawings that are not stored as pictures cannot
be recovered this way. The real fix for a cut-off file is always to download or
copy it again.

## What it stores

Everything is kept in `%LOCALAPPDATA%\SparkyPDF`: the list of recent files and
the WebView2 profile folder. Nothing is sent anywhere.

## Building and layout

| Path | Purpose |
| --- | --- |
| `src/main.rs` | Window, menu, shortcuts, navigation rules, Windows registration |
| `src/pdfcheck.rs` | File health check and page recovery |
| `src/pages.rs` | The start screen and the report pages |
| `build.rs` | Embeds the icon and manifest into the program |
| `assets/make-icon.ps1` | Rebuilds `assets/icon.ico` from the icon artwork |
| `scripts/build-release.ps1` | Builds the release files into `dist\` |
| `installer/SparkyPDF.iss` | Inno Setup script for the installer |
| `.github/workflows/release.yml` | Builds, tests and publishes the downloads when a version tag is pushed |

To publish a new version: change `version` in `Cargo.toml`, commit, then push a
matching tag such as `v1.0.1`. GitHub builds the programs and the installer,
test-installs it, and attaches everything to a new release.

Run the tests with `cargo test`. Debug builds accept
`SPARKYPDF_TEST_CMD=<command>` (for example `print` or `recover`) to run one
menu command a few seconds after start, which allows testing without sending
keystrokes to the desktop. With `SPARKYPDF_TEST_LOG=<file>` set, a debug build
writes down the links it would open instead of starting the browser.

Sparky PDF is written in Rust on top of
[tao](https://github.com/tauri-apps/tao),
[wry](https://github.com/tauri-apps/wry),
[muda](https://github.com/tauri-apps/muda) and
[rfd](https://github.com/PolyMeilex/rfd).

## License

[MIT](LICENSE)
