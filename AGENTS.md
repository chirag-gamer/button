# button: free, open-source Windows 10/11 tray app that gets your screen recording-ready in one click and safely puts everything back

> Status: **DRAFT v0.2** (2026-10-25). Awaiting owner approval (kickoff §6.2). Section layout set by the owner ([D-028](decisions.md)). This file is committed to the public repo as it is ([D-053](decisions.md)).

## Source of Truth & Definitions

- **Local-only docs:** plan.md, prd.md, spec.md, decisions.md, docs/ and research/ live only on the owner's PC (`D:\button`) and are git-ignored ([D-053](decisions.md)). If you work from a public clone and they're missing, ask the owner for them. Public files: README.md, tutorial.md, this file, LICENSE and the code.
- **Canonical homes** (kickoff §3.2): why, phases, tasks and status (N.M) in [plan.md](plan.md) · what (FR-#) in [prd.md](prd.md) · how in [spec.md](spec.md) · reasons (D-###) in [decisions.md](decisions.md) · agent rules in this file · build steps in [tutorial.md](tutorial.md). Cite IDs. Never copy facts between docs.
- **Docs folder:** `docs/` (start at [docs/README.md](docs/README.md)). Guides link to spec.md instead of restating it. `research/` is reference only.
- **Key terms:** Switch, Recording Mode, Session, Journal, Restore, User-reverted, Experimental, Probe, Front app, Shelf, Onboarding. Definitions: [docs/glossary.md](docs/glossary.md).
- **Evidence tags:** DOC / UNDOC / KNOW / SPIKE (spec.md header). Technical unknowns U-1 to U-9 live in [prd.md §9](prd.md#9-open-questions-and-technical-unknowns), each tied to a task. Anything new and undecided is **UNRESOLVED**: add it there and ask.
- **Precedence:** owner's instruction > kickoff doc > this file > other docs. On a conflict, pick the safer, more verifiable, more explicit option and log a D-### (kickoff §1.9). If two docs disagree: stop, fix the non-canonical one, log it.

## Architecture

- Tauri **2.12.x** desktop app in `app/`: Rust core in `app/src-tauri/src/`, vanilla TypeScript + Vite UI in `app/src/` (spec §2 to §4).
- Only `switches/*.rs` changes Windows settings or windows. `session.rs` writes each change to `journal.json` **before** applying it, and restores in reverse order. The UI reaches the core only through IPC commands in `commands.rs` (spec §6.1).
- Triggers: tray (left = dropdown, middle = toggle Recording Mode, right = menu), global hotkey **Alt+R** by default (changeable, shown in onboarding), and the command line `button.exe start|stop|toggle|switch <id> on|off|toggle|restore` (spec §6.3). There is no `button://` URL scheme ([D-045](decisions.md)). Don't add one.
- Data: `settings.json` + `journal.json` in `%LOCALAPPDATA%`. No database, no server. The only network call is the update check.
- Experimental switches (DND, Night light, Bluetooth headphones) rely on undocumented APIs and must stay behind a startup probe and an "Experimental" label (spec §7.6).

## Safety

- **No push, tag, release or publish** until the full suite is green, the diff has been reviewed (no secrets, build artifacts, planning docs or unrelated files) and the owner has said "yes". Never force-push. Never rewrite published history.
- **Never:** commit secrets (updater private key and password); commit the local-only docs; disable, skip or weaken tests (unless the owner approves and it's logged as a D-###); delete user data or journal files outside spec §7 code paths; require admin rights; add network calls; add code signing or a URL scheme without a new decision.
- **Your own machine counts:** before changing any real Windows setting during development or testing, tell the owner what will change and how to undo it. Prefer read-only probes.
- **Releases and migrations:** when a file format changes, bump `schemaVersion` / `journalVersion`, add a migration and a fixture test, and keep the old journal readers. Never install an update during an active session. List format changes in the release notes. Release builds come only from GitHub Actions ([D-044](decisions.md), [docs/deployment.md](docs/deployment.md)).
- **Ask first:** new dependencies, version upgrades (check the release notes, log a D-###), CI/workflow edits, capability/permission changes, new use of undocumented APIs, edits to this file or the kickoff doc (propose, get approval, log).

## Work / Workflow

- **Repo:** `https://github.com/chirag-gamer/button` (public, created in task 1.1, [D-057](decisions.md)). Local working copy: `D:\button` on DESKTOP-SE6HKMO (Windows PowerShell 5.1 through the Desktop Commander MCP).
- **Platform:** Windows only (10 22H2 + 11, x64). Don't add macOS or Linux code paths or `cfg` branches. Windows 10 isn't tested: I don't have Windows 10, so it remains untested. However, it should work ([D-041](decisions.md)).
- **Start of work:** read kickoff §0 to §3 and this file, then pick the next unchecked task in plan.md whose dependencies are done. Do one task at a time, report in the kickoff §9 STATUS shape, and wait for "y".
- **Access:** read the whole repo; write only files the current task needs; run local build, test and lint commands; network only for package registries and doc/version checks. Secrets: none.
- **Commands** (available after task 1.5):

  ```powershell
  cd D:\button\app; npm run tauri dev                                        # run the app
  cargo test --manifest-path app/src-tauri/Cargo.toml <module>::             # Rust: only the module(s) you edited
  npx --prefix app vitest related --run <edited files>                       # TS: only the files you edited
  powershell -ExecutionPolicy Bypass -File scripts\check.ps1                 # full suite: phase end + before any push
  ```

- **Tests:** after each task, run **only the tests for the files you edited**. At the end of a phase and before any push, run the full suite ([D-029](decisions.md), [D-056](decisions.md)).
- **Every report must list:** tests run + results; **new tests needed** (add them as a task in plan.md if not written now); **human verification steps** (exact clicks or commands, expected result, how to undo), especially for switches.
- **Commits:** Conventional Commits, one per task: `git commit -m "<type>: ..." -m "- ..." -m "Implements N.M (FR-#)"`. Push only at phase end with the owner's OK ([D-056](decisions.md)). Docs are updated in the same task (plan.md sync rules). Decisions are logged right away.
- **Done means:** the cited FR acceptance criteria are met, tests for the edited files are green, the docs are in sync, the task is ticked in plan.md as `- [x] N.M: ... — **DONE <date>**: ...`, and the STATUS report has been sent.
- **Stop and ask when:** a task contradicts spec.md or prd.md, an API behaves differently from spec.md, a test fails (kickoff §8.4), or you hit something UNRESOLVED.
- **Roles:** the owner approves every gate. The AI assistant drafts, codes and runs checks, and never approves its own work. There's a single agent, so handoff = the STATUS report + an up-to-date plan.md.
