# button

A free, open-source Windows app that gets your screen ready to record in one click, and puts everything back when you're done.

> **Status:** in development. There's no release yet, so the download link below starts working once v1.0.0 is out.

## What it does

Before you record a tutorial or share your screen in a call, you usually tidy up first. You hide the desktop icons, minimize the other windows, silence notifications and make sure the PC won't fall asleep. Afterwards you have to undo all of it, and it's easy to forget something.

button does this for you. Click its icon in the system tray, or press **Alt+R**, to turn on Recording Mode. Do it again and button puts back exactly what it changed, and nothing else. If your PC crashes or restarts in the middle, button puts things back the next time it starts.

## Features

**Recording Mode** (one click, you choose what's included):

- Do Not Disturb, so notifications don't pop up (experimental)
- Hide desktop icons
- Hide other apps (minimizes everything except the app you're showing)
- Hide the taskbar (turns on auto-hide)
- Hide tray icons (Windows 11 only)
- Backdrop: a colour, gradient or picture behind your app, on all monitors or just one
- Keep the PC awake
- Mute Windows alert sounds

**Quick switches** in the tray dropdown:

- Keep awake, for as long as you want
- Mute microphone
- Lock keyboard (press Esc to unlock)
- Dark mode
- Night light (experimental)
- Start the screen saver
- Connect or disconnect Bluetooth headphones, with battery level where Windows shows it (experimental)

**Also:**

- Alt+R turns Recording Mode on and off. You can change it, and every other hotkey, in Settings.
- Turn any switch off automatically in 15 minutes, 1 hour, 4 hours or tomorrow morning.
- Optional 3, 2, 1 countdown and an optional time limit.
- A short welcome screen the first time you start it.
- Command line for scripts, shortcuts, Task Scheduler or a Stream Deck: `button.exe toggle`, `button.exe switch microphone on`.
- No account, no ads, no tracking. The only thing button connects to is GitHub, to check for updates, and you can turn that off.

"Experimental" means the feature relies on parts of Windows that Microsoft doesn't document. button checks at startup whether they work on your PC and turns the feature off if they don't.

## Download

**[Download button for Windows (button-setup.exe)](https://github.com/chirag-gamer/button/releases/latest/download/button-setup.exe)**

For Windows 11 and Windows 10 (version 22H2), 64-bit. All releases are on the [Releases page](https://github.com/chirag-gamer/button/releases). The installer doesn't need admin rights. It installs just for your user account.

### "Windows protected your PC"

button isn't code-signed. Signing certificates cost money and this is a free project. So the first time you run the installer, Windows SmartScreen may show a blue box saying "Windows protected your PC". Click **More info**, then **Run anyway**.

Every installer is built by GitHub Actions straight from the code in this repo, so you can see exactly what goes into it. If you'd rather not run it, you can build button yourself.

### Windows 10

I don't have Windows 10, so it remains untested. However, it should work. If something doesn't, please open an issue. A few features need Windows 11 and are greyed out on Windows 10.

## How it puts things back

Before button changes a setting, it writes down the old value in a small file on your PC. Restoring reads that file backwards and puts each setting back. If you changed something back yourself during the session, button leaves it alone. If you uninstall button while Recording Mode is on, it restores everything first.

## Build it yourself

See [tutorial.md](tutorial.md).

## License

MIT. See [LICENSE](LICENSE).
