// SPDX-License-Identifier: GPL-3.0-or-later
// Included inside MailWindow so the optional dialog uses its normal engine,
// callback ownership and universal-menu helpers.
void misfinDialog() {
  if (isDemo) {
    fl_message("%s", _("Demo mode cannot use Misfin identities"));
    return;
  }
  struct Form {
    dx::GpuWindow *window;
    Fl_Choice *identity;
    Fl_Input *to, *subject, *inbox;
    Fl_Text_Editor *editor;
    Fl_Text_Display *reader;
    Fl_Hold_Browser *history;
    Fl_Box *status;
    Fl_Button *send, *fetch, *import;
    std::shared_ptr<Fl_Text_Buffer> body = std::make_shared<Fl_Text_Buffer>();
    std::shared_ptr<Fl_Text_Buffer> viewed = std::make_shared<Fl_Text_Buffer>();
    Json identities = Json::array(), messages = Json::array();
    std::string chosen;
    bool busy = false;
    void wait(bool value) {
      busy = value;
      Fl_Widget *controls[] = {identity, to, subject, editor, send, fetch, import, inbox};
      for (Fl_Widget *w : controls) {
        if (value)
          w->deactivate();
        else
          w->activate();
      }
    }
  };
  auto f = std::make_shared<Form>();
  f->window = dialog(1000, 800, _("Misfin Messaging"));
  auto bar = new DesktopMenuBar(0, 0, 1000, 26);
  f->import = new Fl_Button(20, 42, 230, 32, _("Import Misfin Identity…"));
  f->identity = new Fl_Choice(390, 42, 580, 32, _("Identity"));
  f->to = new Fl_Input(180, 90, 790, 30, _("Misfin Recipient"));
  f->to->tooltip(_("One mailbox@hostname address; Misfin(C), port 1958 by default"));
  f->subject = new Fl_Input(180, 132, 790, 30, _("Subject"));
  f->editor = new Fl_Text_Editor(20, 198, 950, 180, _("Gemtext Message"));
  f->editor->align(FL_ALIGN_TOP_LEFT);
  f->editor->buffer(f->body.get());
  f->editor->wrap_mode(Fl_Text_Display::WRAP_AT_BOUNDS, 0);
  f->send = new Fl_Button(20, 392, 145, 32, _("Send Misfin"));
  auto importMail = new Fl_Button(180, 392, 210, 32, _("Import Gemmail Archive…"));
  auto refresh = new Fl_Button(405, 392, 200, 32, _("Refresh Local History"));
  f->inbox = new Fl_Input(180, 444, 600, 30, _("Gemini Mailbox URL"));
  f->inbox->tooltip(_("Optional server-provided gemini:// URL for a Misfin(C) gemmail or "
                      "gembox archive"));
  f->fetch = new Fl_Button(800, 444, 170, 30, _("Fetch Mailbox"));
  f->history = new Fl_Hold_Browser(20, 520, 330, 210, _("Local Misfin History"));
  f->history->align(FL_ALIGN_TOP_LEFT);
  f->history->format_char(0);
  f->reader = new Fl_Text_Display(370, 520, 600, 210, _("Gemmail Text"));
  f->reader->align(FL_ALIGN_TOP_LEFT);
  f->reader->buffer(f->viewed.get());
  f->reader->wrap_mode(Fl_Text_Display::WRAP_AT_BOUNDS, 0);
  f->status = new Fl_Box(20, 750, 950, 28);
  f->status->align(FL_ALIGN_LEFT | FL_ALIGN_INSIDE);
  f->status->copy_label(
      _("Misfin(C) uses certificate identities. Attachments and SMTP are not used."));
  auto load = std::make_shared<std::function<void()>>();
  *load = [this, f] {
    engine->request({{"op", "misfin-state"}}, [this, f](const Json &reply) {
      if (!f->window->shown())
        return;
      if (!accepted(reply)) {
        fl_alert("%s", str(reply, "error").c_str());
        return;
      }
      f->identities = reply["result"]["identities"];
      f->identity->clear();
      int selected = 0;
      for (size_t i = 0; i < f->identities.size(); ++i) {
        addChoice(f->identity, str(f->identities[i], "address"));
        if (str(f->identities[i], "id") == f->chosen)
          selected = int(i);
      }
      if (!f->identities.empty()) {
        f->identity->value(selected);
        f->chosen = str(f->identities[selected], "id");
      }
      f->messages = Json::array();
      f->history->clear();
      f->viewed->text("");
      const auto &messages = reply["result"]["messages"];
      for (auto it = messages.rbegin(); it != messages.rend(); ++it) {
        if (str(*it, "identity") != f->chosen)
          continue;
        f->messages.push_back(*it);
        std::string label =
            (str(*it, "direction") == "sent" ? _("Sent: ") : _("Received: ")) +
            str(*it, "subject");
        auto clipped = g_utf8_substring(label.c_str(), 0, 120);
        label = clipped;
        g_free(clipped);
        for (char &c : label)
          if ((unsigned char)c < 32)
            c = ' ';
        f->history->add(label.c_str());
      }
    });
  };
  bind(f->identity, [f, load] {
    int n = f->identity->value();
    if (n >= 0 && n < int(f->identities.size()))
      f->chosen = str(f->identities[n], "id");
    (*load)();
  });
  bind(f->history, [f] {
    int n = f->history->value() - 1;
    if (n < 0 || n >= int(f->messages.size()))
      return;
    const auto &m = f->messages[n];
    auto text = str(m, "from") + "\n" + str(m, "to") + "\n" + str(m, "timestamp") +
                "\n\n" + str(m, "body");
    f->viewed->text(text.c_str());
  });
  auto identity = [this, f, load] {
    if (f->busy)
      return;
    Fl_Native_File_Chooser picker;
    picker.type(Fl_Native_File_Chooser::BROWSE_FILE);
    picker.title(_("Misfin Identity Certificate (PEM)"));
    if (picker.show())
      return;
    std::string certificate = picker.filename();
    picker.title(_("Misfin Private Key (PEM)"));
    if (picker.show())
      return;
    f->wait(true);
    engine->request({{"op", "misfin-import-identity"},
                     {"certificate", certificate},
                     {"key", picker.filename()}},
                    [this, f, load](const Json &reply) {
                      if (!f->window->shown())
                        return;
                      f->wait(false);
                      if (!accepted(reply)) {
                        fl_alert("%s", str(reply, "error").c_str());
                        return;
                      }
                      f->chosen = str(reply["result"], "identity");
                      (*load)();
                    });
  };
  auto deliver = [this, f, load](bool fetch) {
    if (f->busy)
      return;
    if (f->chosen.empty()) {
      fl_alert("%s", _("Choose a Misfin identity first"));
      return;
    }
    Json request = {{"op", fetch ? "misfin-fetch" : "misfin-send"},
                    {"identity", f->chosen},
                    {"destination", fetch ? f->inbox->value() : f->to->value()}};
    if (!fetch) {
      char *body = f->body->text();
      request["body"] = body;
      free(body);
      request["subject"] = f->subject->value();
    }
    Json probe = {{"op", "misfin-probe"},
                  {"identity", f->chosen},
                  {"destination", request["destination"]},
                  {"gemini", fetch}};
    f->wait(true);
    f->status->copy_label(_("Checking server certificate…"));
    engine->request(probe, [this, f, load, request](const Json &reply) mutable {
      if (!f->window->shown())
        return;
      if (!accepted(reply)) {
        f->wait(false);
        fl_alert("%s", str(reply, "error").c_str());
        return;
      }
      const auto &peer = reply["result"];
      if (!peer.value("trusted", false)) {
        const char *warning = peer["previous"].is_null()
                                  ? _("This server is not trusted yet.")
                                  : _("The server certificate has changed.");
        if (fl_choice(_("%s\n%s\nSHA-256: %s\n\nTrust this fingerprint and continue?"),
                      _("Cancel"), _("Trust and Continue"), nullptr, warning,
                      str(peer, "peer").c_str(), str(peer, "fingerprint").c_str()) != 1) {
          f->wait(false);
          f->status->copy_label(_("Canceled"));
          return;
        }
      }
      request["fingerprint"] = peer["fingerprint"];
      f->status->copy_label(_("Contacting Misfin server…"));
      engine->request(request, [this, f, load, request](const Json &answer) {
        if (!f->window->shown())
          return;
        f->wait(false);
        if (!accepted(answer)) {
          fl_alert("%s", str(answer, "error").c_str());
          return;
        }
        if (answer["result"].contains("warning") &&
            answer["result"]["warning"].is_string())
          fl_alert("%s", str(answer["result"], "warning").c_str());
        f->status->copy_label(request["op"] == "misfin-send"
                                  ? _("Misfin message accepted by the server")
                                  : _("Misfin mailbox fetched"));
        (*load)();
      });
    });
  };
  bind(f->import, identity);
  bind(f->send, [deliver] { deliver(false); });
  bind(f->fetch, [deliver] { deliver(true); });
  bind(refresh, [load] { (*load)(); });
  auto importArchive = [this, f, load] {
    if (f->busy)
      return;
    if (f->chosen.empty()) {
      fl_alert("%s", _("Choose a Misfin identity first"));
      return;
    }
    Fl_Native_File_Chooser picker;
    picker.type(Fl_Native_File_Chooser::BROWSE_FILE);
    picker.title(_("Misfin(C) Gemmail or Gembox Archive"));
    if (picker.show())
      return;
    f->wait(true);
    engine->request({{"op", "misfin-import-mail"},
                     {"identity", f->chosen},
                     {"path", picker.filename()}},
                    [this, f, load](const Json &reply) {
                      if (!f->window->shown())
                        return;
                      f->wait(false);
                      if (!accepted(reply)) {
                        fl_alert("%s", str(reply, "error").c_str());
                        return;
                      }
                      (*load)();
                    });
  };
  bind(importMail, importArchive);
  addMenu(*bar, _("File/Send Misfin"), FL_CTRL | FL_Enter, [deliver] { deliver(false); });
  addMenu(*bar, _("File/Import Misfin Identity…"), 0, identity);
  addMenu(*bar, _("File/Fetch Mailbox"), 0, [deliver] { deliver(true); });
  addMenu(*bar, _("File/Import Gemmail Archive…"), 0, importArchive);
  addMenu(*bar, _("File/Close"), FL_CTRL | 'w', [f] { f->window->hide(); });
  addMenu(*bar, _("Edit/Cut"), FL_CTRL | 'x', [this] { edit('x'); });
  addMenu(*bar, _("Edit/Copy"), FL_CTRL | 'c', [this] { edit('c'); });
  addMenu(*bar, _("Edit/Paste"), FL_CTRL | 'v', [this] { edit('v'); });
  addMenu(*bar, _("Edit/Select All"), FL_CTRL | 'a', [this] { edit('a'); });
  f->window->resizable(f->reader);
  f->window->size_range(900, 650);
  showDialog(f->window);
  dialogMenus.push_back(std::make_unique<DesktopMenu>(*f->window, *bar));
  (*load)();
}
