# Penguin Mail for FLTK and DomeD

This experimental native frontend is based on upstream main at
`0facdf2ee4a7d07eb1a0eed0a22d32822ac1c10e` (2026-10-22, version 1.0.10).
The original GTK frontend and its source documentation remain in `app/` and
`README_UPSTREAM.md`.

## Build and launch

Use Rust 1.98 or newer, CMake, Ninja, a C++17 compiler, the suite's patched
FLTK 1.4.5, Fontconfig, GIO, Wayland client headers, X11 and wayland-scanner.
No GTK, libadwaita or WebKit development packages are needed for the native
binaries.

POP3 uses the platform TLS library with TLS 1.2 as its minimum and normal
certificate and hostname verification. Linux builds also need OpenSSL headers.
This supports providers whose TLS 1.2 servers require RSA cipher suites.

From this repository:

```sh
python3 fltk/build.py --prefix ../_toolkit
LD_LIBRARY_PATH=../_toolkit/lib build-fltk/penguin-mail --demo
LD_LIBRARY_PATH=../_toolkit/lib build-fltk/penguin-mail --calendar 2028-02-29
```

The suite runner also provides `./run penguin-mail`. `--tray` starts in the
tray when a StatusNotifier host is present. Without one the window stays
available. `--compose [ADDRESS]` and `mailto:ADDRESS` open the composer.
`--calendar-today` and `--calendar YYYY-MM-DD` select a local calendar date;
invalid dates fail before launching an engine. A second launch activates the
existing instance through D-Bus. Demo and real stores have separate bus names.

The build helper can find this workspace's existing private Rust toolchain
when the system compiler is too old. A fresh machine should put Rust 1.98+
on PATH. The runtime copy of the engine has debug sections removed; Cargo keeps its
original executable in `target/release` for release builds and `target/debug`
for Debug builds. Binutils or
LLVM strip is required. No tools, libraries or applications are installed by the
build helper.
`cmake --install build-fltk --prefix <prefix>` installs both binaries, native
desktop entry, icons and compiled translations. The frontend finds its engine
beside itself; `PENGUIN_MAIL_ENGINE` overrides that location.

## Architecture and stored data

`native/` draws actual FLTK controls with the suite's OpenGL widget driver.
`engine/` is a display-independent Rust process using the original store and
sync crates. Commands and replies are framed as JSON lines on inherited private
pipes, without a listening socket. Provider calls stay off the UI thread.
Request generations discard old list, conversation and calendar replies.
Mail changes go through `MailActions`, drafts through `AccountSync`, sending
through `Outbox`, and calendar changes through `Calendar` and `CalendarCopy`.
The frontend does not implement a separate IMAP, Gmail or SMTP stack.

The GTK and native applications share the sync lock; close one before opening
the other on the same store.

Real accounts use upstream's `config.toml`, `mailrs.db`, sync lock and desktop
keyring. `MAILRS_DATA_DIR` and `MAILRS_CONFIG` retain their upstream meaning.
`--demo` uses temporary SQLite storage, a temporary GnuPG keyring and upstream
in-memory providers; it cannot change real accounts or personal keys. Explicitly
imported demo keys are discarded when the preview closes. Native UI settings live in
`$XDG_CONFIG_HOME/penguin-mail/fltk.json`. Read requests can run concurrently;
mutations are serialized. Pipe close shuts down sync and the child process.

## Native desktop integration

- Native menu bars collapse while a DomeD universal-menu host and window association
  are available. Keyboard shortcuts remain active; local bars return if the host
  disappears, and other desktop sessions keep their local menus.
- The real FLTK menus export through DBusMenu, KDE's Wayland AppMenu association
  and X11 appmenu properties. Dialog composers export their own edit/send menus.
- DomeD's palette, accent, bitmap font family, style and native strike apply live,
  including text readers, editors and native FLTK dialogs.
- Fontconfig export changes reload the selected bitmap face and clear text
  textures, including fonts added while the application is running. With an
  updated DomeD, `dome-compositor --refresh-session-fonts` repairs older session
  font exports without logging out.
- Standard Ctrl shortcuts and editing commands stay in the app; Super shortcuts
  remain compositor-owned. Message and calendar lists also accept cursor keys.
- StatusNotifierItem shares the actual command menu and unread title. Closing the
  main window keeps its event loop and engine alive when a tray host is available.
  Losing the tray host restores the window.
- Mail and calendar notifications go directly to DomeD's session-bus
  `org.freedesktop.Notifications` server, with the app's desktop identity.
  Mail uses private count-only text; clicking opens its conversation or inbox.
  Calendar reminders share upstream recurrence and snooze policy, work from the
  tray, open the event's date, and offer Join and Snooze 5 Minutes. Delivery is
  recorded only after the server accepts it; failed deliveries retry. Disabling
  Event Reminders in Preferences consumes due reminders without a later backlog.
  Reminder times follow `dome/clock.json`. The LauncherEntry unread badge also
  uses desktop D-Bus.
- DomeD's clock calls `penguin-mail --calendar YYYY-MM-DD`; the running instance
  changes date without starting another sync engine.

## Implemented workflows

Unified and per-account inboxes; server labels; mail search; message or thread
listing; read/unread, flag, archive, trash, labels, reminders and undo; plain-text
reading with upstream HTML-to-text conversion; attachment saving; composition,
reply/forward, draft saving and reopening with attachments, Send Later, outgoing
retry/delete, mbox export; Google/Microsoft browser sign-in, IMAP/SMTP, POP3,
JMAP token/password and CalDAV account forms; monthly calendar, agenda, event
creation/editing/deletion; contact search and creation; native preferences.

Google sign-in uses upstream's configured client or a Google Cloud **Desktop app**
OAuth client JSON selected in Add Account. Only `installed.client_id` and
`installed.client_secret` are read; JSON endpoint URLs cannot redirect tokens.
After successful consent, upstream stores the account client and refresh token in
its desktop keyring. The frontend does not keep a plaintext OAuth configuration.
The build helper also reads upstream's optional `packaging/secrets.env`, and its
engine build is invalidated when the OAuth build values change. A client is not
included in this checkout; live Google consent needs a client and the person's
browser sign-in. Microsoft still requires its configured client ID. POP3 setup keeps server mail.
The core's provider capabilities and permission checks still apply to each action.
A file export refuses to overwrite an existing file. Recurring event changes use
the occurrence id supplied by the upstream calendar model.

## OpenPGP

The reader automatically verifies PGP/MIME and inline signatures and opens
PGP/MIME or inline encrypted mail through the person's GnuPG and pinentry.
It displays the same signature, sender-identity and trust verdicts as GTK. Only
the bytes actually verified or decrypted become the reader body and attachment
list; additional unsigned parts cannot appear under a valid signature. Opened
attachments stay in memory until the person saves them, and plaintext is never
written back to the server or body cache.

The composer offers **Sign (OpenPGP)** and **Encrypt (OpenPGP)**, checks all To/Cc/Bcc
recipients and the sender's Sent copy, and reports missing keys and identity trust.
Bcc uses hidden recipients. Missing keys refuse the send rather than downgrade it.
An encrypted draft is saved encrypted to its writer alone and restores its sign /
encrypt choices when reopened. Replies quoting decrypted mail start encrypted;
sending them in the clear requires an explicit confirmation.

**File → OpenPGP Keys…** checks local keys and imports a picked public or secret key
file. Key generation, revocation, export and trust editing remain available in
the person's usual GnuPG tools. No message read downloads keys. Crypto calls run
on blocking workers and are serialized so pinentry does not block the UI or race.
In demo mode the keyring is temporary, independently of the person's `GNUPGHOME`.

## Parity still to port

The retained GTK interface has features this native frontend does not yet expose:
rich HTML and rich-text editing; integrated S/MIME verification,
decryption, signing, encryption and certificate management; the optional assistant; full account/server/rule
preferences; complete label management; undo-send UI; calendar invitation,
recurrence, import, day/week views and recurrence-scope controls; drag/drop,
attachments in forwarded mail, and upstream's complete accessibility interfaces.
S/MIME messages are identified and can be exported; the native frontend does
not claim to verify or decrypt S/MIME. Do not use the native event editor to change
the recurrence rule of a series: it currently edits individual occurrences only.

FLTK does not supply GTK's AT-SPI tree. Text labels and keyboard access exist,
but the original accessibility gate does not certify this frontend. The upstream
GTK target remains available with `cargo run -p mailrs` where its development
libraries are installed.

## Validation

From this repository, with the built frontend and engine:

```sh
cargo test --workspace --exclude mailrs
cargo clippy --workspace --exclude mailrs --all-targets -- -D warnings
python3 fltk/tests/engine.py build-fltk/penguin-mail-engine
python3 fltk/tests/crypto.py build-fltk/penguin-mail-engine
python3 fltk/tests/oauth.py build-fltk/penguin-mail-engine
uv run --no-project --with python-xlib --with pillow python fltk/tests/ui.py
uv run --no-project --with python-xlib --with pillow --with dbus-next python fltk/tests/menu.py
DOME_TEST_BIN=/path/to/dome-compositor uv run --no-project --with python-xlib --with pillow python fltk/tests/appearance.py
DOME_TEST_BIN=/path/to/dome-compositor uv run --no-project --with python-xlib --with pillow python fltk/tests/clock.py
DOME_TEST_BIN=/path/to/dome-compositor uv run --no-project --with python-xlib --with pillow --with dbus-next python fltk/tests/notifications.py
```

The UI scripts expect the patched FLTK runtime in `../_toolkit/lib`.
They create ignored `test-artifacts/` logs and screenshots. They require Xvfb,
D-Bus, `gdbus` and GnuPG; the clock and notification checks also require a built
DomeD executable. Set `DOME_TEST_BIN` when it is outside this workspace.

The engine test uses the real upstream action/outbox/calendar paths on demo
providers. UI tests use private Xvfb/D-Bus sessions and a private nested DomeD;
they exercise the exported menu, native widgets, date activation, clock settings,
Wayland identity and tray restoration. GnuPG tests generate private fixture keys with a noninteractive pinentry. OAuth
checks verify client selection, PKCE and callback handling without contacting
Google, granting consent or storing a token. Logs and screenshots are in `test-artifacts/`. Physical XR, live provider authentication and live mail sending
have not been tested. Full upstream workspace tests and its AT-SPI script also
require GTK/Pango/WebKit development packages; use the separate native core test
results where those packages are unavailable.
