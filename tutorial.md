# Build button from source

This guide shows how to build button on your own Windows PC, either to run it while you work on it or to make your own installer.

## What you need

| Tool | Version | How to check |
|---|---|---|
| Windows | 10 (22H2) or 11, 64-bit | run `winver` |
| Microsoft C++ Build Tools | "Desktop development with C++" (MSVC and a Windows SDK) | Visual Studio Installer |
| WebView2 Runtime | already part of Windows 11 and updated Windows 10 | Settings > Apps > Installed apps |
| Rust | 1.90 or newer, MSVC toolchain | `rustc -V` |
| Node.js | 24 | `node -v` |
| Git | any recent version | `git --version` |

How to install them:

1. **C++ Build Tools:** download the [Microsoft C++ Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/), run it and tick **Desktop development with C++**. If you already have Visual Studio, add that workload instead.
2. **Rust:** in PowerShell run `winget install --id Rustlang.Rustup`, open a new PowerShell window, then run `rustup default stable-msvc`.
3. **Node.js:** install version 24 from [nodejs.org](https://nodejs.org/).
4. **Git:** install it from [git-scm.com](https://git-scm.com/).

The [Tauri prerequisites page](https://v2.tauri.app/start/prerequisites/) has more detail if something is missing.

## Get the code

```powershell
git clone https://github.com/chirag-gamer/button.git
cd button\app
npm ci
```

## Run it

```powershell
npm run tauri dev
```

The first run takes a while because Rust compiles everything once. When it's ready, the button icon appears in the system tray. On Windows 11 it may be hidden under the `^` arrow next to the clock.

Changes to the UI show up right away. Changes to the Rust code rebuild and restart the app.

## Make an installer

```powershell
npm run tauri build
```

The installer ends up in `app\src-tauri\target\release\bundle\nsis\`. Your build isn't code-signed, same as the official one, so Windows may show the SmartScreen warning described in the [README](README.md#windows-protected-your-pc).

Your build doesn't need any keys. Only the official release adds the update signature, and that happens on GitHub Actions.

## Run the checks

Before you share a change, run the same checks the project uses. From the repo folder:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\check.ps1
```

This runs formatting, lint and all tests. It must pass.

## Where button keeps its files

Settings, logs and the journal (the list of changes to undo) are kept under `%LOCALAPPDATA%\io.github.chirag-gamer.button\`. Don't delete `journal.json` while Recording Mode is on. That file is how button knows what to put back.
