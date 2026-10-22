# Penguin Mail for FLTK and DomeD

This fork adds a native FLTK mail, calendar and contacts interface to
[c9dev/Penguin Mail](https://github.com/c9dev/penguin-mail), using its Rust
mail engines and SQLite store. It targets Linux and integrates with
[DomeD](https://github.com/jebot-git/DomeD).

The original GTK frontend remains in `app/`. Its documentation is preserved
in [README_UPSTREAM.md](README_UPSTREAM.md). This fork is based on upstream
main at `0facdf2ee4a7d07eb1a0eed0a22d32822ac1c10e`, version 1.0.10.

## Native interface

- Unified and per-account mail, search, message actions, composition, drafts,
  attachments, Send Later and outbox management.
- Calendar month and agenda, event editing, contacts and calendar date links.
- DomeD universal menus, appearance, keyboard shortcuts and tray integration.
  Local menu bars collapse while DomeD hosts their commands and return when
  the host disappears.
- Mail and calendar notifications through DomeD's notification server.
  Clicks open the conversation or event date; reminders offer Join and a
  persistent five-minute snooze.
- OpenPGP verification, decryption, signing, encryption, encrypted drafts and
  key-file import through GnuPG.
- Google Desktop OAuth client JSON selection and the upstream account/provider
  adapters. Live Gmail consent still requires your own configured client.

See [the native frontend documentation](fltk/README.md) for implemented
workflows, configuration, cryptography, dependencies and remaining gaps.

## Build and preview

Install Rust 1.98 or newer, CMake, Ninja, a C++17 compiler, Fontconfig, GIO,
Wayland client headers, X11 and wayland-scanner. The frontend requires the
suite's patched FLTK 1.4.5 installed at the prefix you supply. The native
binaries do not require GTK, libadwaita or WebKit development packages.

```sh
python3 fltk/build.py --prefix /path/to/patched-fltk
LD_LIBRARY_PATH=/path/to/patched-fltk/lib build-fltk/penguin-mail --demo
```

Demo mode uses temporary mail storage, in-memory providers and a temporary
GnuPG keyring. It does not connect to real mail accounts.

Omit `--demo` to use the upstream account store. Close the GTK application
before opening the native interface on the same store.

## Validation and current limits

The current native build passed 4,097 Rust tests and 45 UI checks, including
private DomeD notification, universal-menu, clock and tray tests. Native
Clippy and translation checks also passed. Reproduction commands are in
[fltk/README.md](fltk/README.md#validation).

Full GTK workspace and accessibility checks remain blocked in the development
environment by missing GTK/Pango development libraries. The FLTK frontend does
not yet provide GTK's complete accessibility interfaces, rich HTML editing,
S/MIME processing or every upstream calendar/account feature. Live provider
sign-in, outbound mail and physical XR have not been tested.

## License

[GPL-3.0-or-later](LICENSE), retaining upstream authorship and history.
