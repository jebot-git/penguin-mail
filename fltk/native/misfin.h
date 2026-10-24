// SPDX-License-Identifier: GPL-3.0-or-later
// Misfin uses the normal account list, reader and composer. Only certificate
// trust and account-specific receiving settings need separate controls.
void misfinTransmit(Json request, std::function<void(const Json &)> done) {
  Json probe = {{"op", "misfin-probe"},
                {"account", request.value("account", int64_t(0))},
                {"destination", request.value("op", "") == "compose"
                                    ? request["to"]
                                    : request["destination"]},
                {"gemini", request.value("op", "") == "misfin-fetch"}};
  engine->request(probe, [this, request, done](const Json &reply) mutable {
    if (!reply.value("ok", false)) {
      done(reply);
      return;
    }
    const auto &peer = reply["result"];
    if (!peer.value("trusted", false)) {
      const char *warning = peer["previous"].is_null()
                                ? _("This server is not trusted yet.")
                                : _("The server certificate has changed.");
      if (fl_choice(
              _("%s\n%s\nSHA-256: %s\n\nTrust this fingerprint and continue?"),
              _("Cancel"), _("Trust and Continue"), nullptr, warning,
              str(peer, "peer").c_str(),
              str(peer, "fingerprint").c_str()) != 1) {
        done({{"ok", false}, {"error", _("Canceled")}});
        return;
      }
    }
    request["fingerprint"] = peer["fingerprint"];
    engine->request(request, done);
  });
}
void refreshMailbox() {
  if (selectedAccount >= 0) {
    send({{"op", "refresh"}}, [this](const Json &) { reload(); });
    return;
  }
  for (const auto &item : accounts) {
    const auto &account = item["account"];
    if (account.value("id", int64_t(0)) != selectedAccount)
      continue;
    auto url = str(account, "misfin_inbox_url");
    if (url.empty()) {
      say(_("Set a provider-supplied Gemini mailbox URL in Account Settings, "
            "or import a Gemmail archive."));
      reload();
      return;
    }
    say(_("Fetching Misfin mailbox…"));
    misfinTransmit({{"op", "misfin-fetch"},
                    {"account", selectedAccount},
                    {"destination", url}},
                   [this](const Json &reply) {
                     if (accepted(reply)) {
                       say(_("Misfin mailbox fetched"));
                       reload();
                     }
                   });
    return;
  }
}
void misfinSettings() {
  if (selectedAccount >= 0)
    return;
  const auto account = selectedAccount;
  auto win = dialog(820, 360, _("Account Settings"));
  auto bar = new DesktopMenuBar(0, 0, 820, 26);
  auto url = new Fl_Input(195, 55, 600, 30, _("Gemini Mailbox URL"));
  for (const auto &item : accounts)
    if (item["account"].value("id", int64_t(0)) == account)
      url->value(str(item["account"], "misfin_inbox_url").c_str());
  auto info =
      new Fl_Box(20, 110, 775, 100,
                 _("Misfin(C) receives mail through a provider-supplied Gemini "
                   "archive or a local Gemmail import.\n"
                   "Refresh fetches the configured archive with your "
                   "certificate. There is no background polling.\n"
                   "Creating an identity does not register a server mailbox. "
                   "Copy the public certificate for your provider."));
  info->align(FL_ALIGN_LEFT | FL_ALIGN_INSIDE | FL_ALIGN_WRAP);
  auto copy = [this, account] {
    send({{"op", "misfin-certificate"}, {"account", account}},
         [this](const Json &r) {
           auto cert = str(r, "certificate");
           Fl::copy(cert.c_str(), int(cert.size()), 1);
           say(_("Public identity certificate copied. The private key stays in "
                 "Penguin Mail."));
         });
  };
  auto import = [this, account] {
    Fl_Native_File_Chooser picker;
    picker.type(Fl_Native_File_Chooser::BROWSE_FILE);
    picker.title(_("Misfin(C) Gemmail or Gembox Archive"));
    if (picker.show())
      return;
    send({{"op", "misfin-import-mail"},
          {"account", account},
          {"path", picker.filename()}},
         [this](const Json &) { reload(); });
  };
  new CommandButton(20, 235, 300, 32, _("Copy Identity Certificate"), copy);
  new CommandButton(340, 235, 300, 32, _("Import Gemmail Archive…"), import);
  new CommandButton(655, 300, 140, 32, _("Save"), [this, win, url, account] {
    send({{"op", "misfin-settings"},
          {"account", account},
          {"inbox_url", url->value()}},
         [this, win](const Json &) {
           win->hide();
           reloadAccounts();
           reload();
         });
  });
  addMenu(*bar, _("File/Copy Identity Certificate"), 0, copy);
  addMenu(*bar, _("File/Import Gemmail Archive…"), 0, import);
  addMenu(*bar, _("File/Close"), FL_CTRL | 'w', [win] { win->hide(); });
  for (auto key : {'x', 'c', 'v', 'a'}) {
    const char *path = key == 'x'   ? _("Edit/Cut")
                       : key == 'c' ? _("Edit/Copy")
                       : key == 'v' ? _("Edit/Paste")
                                    : _("Edit/Select All");
    addMenu(*bar, path, FL_CTRL | key, [this, key] { edit(key); });
  }
  showDialog(win);
  dialogMenus.push_back(std::make_unique<DesktopMenu>(*win, *bar));
}
