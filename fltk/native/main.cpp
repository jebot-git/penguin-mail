// SPDX-License-Identifier: GPL-3.0-or-later
#include "desktop-menu.h"
#include "engine.h"
#include "gpu-window.h"
#include "status-icon.h"
#include "autostart.h"
#include "notifications.h"
#include <FL/Fl_Button.H>
#include <FL/Fl_Check_Button.H>
#include <FL/Fl_Hold_Browser.H>
#include <FL/Fl_Input.H>
#include <FL/Fl_Int_Input.H>
#include <FL/Fl_Menu_Bar.H>
#include <FL/Fl_Native_File_Chooser.H>
#include <FL/Fl_Secret_Input.H>
#include <FL/Fl_Table_Row.H>
#include <FL/Fl_Text_Editor.H>
#include <FL/Fl_Tile.H>
#include <array>
#include <chrono>
#include <cmath>
#include <ctime>
#include <iomanip>
#include <libintl.h>
#include <limits>
#include <set>
#include <sstream>
#define _(s) gettext(s)
static std::string str(const Json &j, const char *k,
                       const std::string &fallback = "") {
  auto i = j.find(k);
  return i != j.end() && i->is_string() ? i->get<std::string>() : fallback;
}
static int64_t nowMs() {
  return std::chrono::duration_cast<std::chrono::milliseconds>(
             std::chrono::system_clock::now().time_since_epoch())
      .count();
}
static std::string dateText(int64_t millis,
                            const char *format = "%Y-%m-%d %H:%M") {
  time_t t = millis / 1000;
  tm local{};
  localtime_r(&t, &local);
  char out[128];
  strftime(out, sizeof(out), format, &local);
  return out;
}
static constexpr int64_t InvalidDate = std::numeric_limits<int64_t>::min();
static int64_t parseDate(const std::string &s,
                         const char *format = "%Y-%m-%d %H:%M") {
  tm t{};
  t.tm_isdst = -1;
  char *end = strptime(s.c_str(), format, &t);
  if (!end || *end)
    return InvalidDate;
  errno = 0;
  time_t value = mktime(&t);
  if ((value == -1 && errno == EOVERFLOW) ||
      dateText(value * 1000, format) != s)
    return InvalidDate;
  return value * 1000;
}
static std::string displayText(const std::string &text) {
  std::string result;
  result.reserve(text.size());
  for (size_t i = 0; i < text.size(); ++i) {
    if (text[i] == '\r') {
      if (i + 1 == text.size() || text[i + 1] != '\n')
        result += '\n';
    } else
      result += text[i];
  }
  return result;
}
static std::string addressList(const Json &items) {
  std::string result;
  if (!items.is_array())
    return result;
  for (const auto &item : items) {
    if (!result.empty())
      result += ", ";
    result += str(item, "email");
  }
  return result;
}
// Treat addresses and server errors as text, rather than FLTK symbol markup.
static std::string literalLabel(const std::string &text) {
  std::string result;
  for (char c : text) {
    result += c;
    if (c == '@') result += '@';
  }
  return result;
}
static void copyLabel(Fl_Widget *widget, const std::string &text) {
  widget->copy_label(literalLabel(text).c_str());
}
static void addChoice(Fl_Choice *choice, const std::string &text) {
  // replace() copies the literal label without parsing / or | as menu structure.
  std::string placeholder = "Penguin Mail row " + std::to_string(choice->size());
  while (choice->find_item(placeholder.c_str())) placeholder += "x";
  int index = choice->add(placeholder.c_str());
  std::string label = literalLabel(text);
  std::string escaped;
  for (char c : label) { escaped += c; if (c == '&') escaped += '&'; }
  choice->replace(index, escaped.c_str());
}
struct CommandButton : Fl_Button {
  std::function<void()> action;
  CommandButton(int x, int y, int w, int h, const char *label,
                std::function<void()> fn)
      : Fl_Button(x, y, w, h, label), action(std::move(fn)) {
    callback([](Fl_Widget *w, void *) {
      auto b = static_cast<CommandButton *>(w);
      if (b->action)
        b->action();
    });
  }
};
class ReaderPane : public Fl_Group {
  void layout() {
    if (!reader)
      return;
    const int available = std::max(1, w() - 24);
    std::array<int, 3> widths{};
    int height = 30;
    auto previousFont = fl_font();
    int previousSize = fl_size();
    for (size_t i = 0; i < actions.size(); ++i) {
      auto button = actions[i];
      fl_font(button->labelfont(), button->labelsize());
      widths[i] = std::min(available,
                          std::max(80, int(std::ceil(fl_width(button->label()))) + 24));
      int textWidth = std::max(1, widths[i] - 24), textHeight = 0;
      fl_measure(button->label(), textWidth, textHeight, 0);
      height = std::max(height, textHeight + 12);
    }
    fl_font(previousFont, previousSize);
    int left = x() + 12, nextX = left, nextY = y() + 10;
    // Pane resizing must not shrink actions below their translated labels.
    for (size_t i = 0; i < actions.size(); ++i) {
      if (nextX > left && nextX + widths[i] > left + available) {
        nextX = left;
        nextY += height + 8;
      }
      actions[i]->resize(nextX, nextY, widths[i], height);
      nextX += widths[i] + 10;
    }
    int readerY = nextY + height + 14;
    reader->resize(left, readerY, available,
                   std::max(40, y() + h() - 110 - readerY));
  }
  void draw() override {
    // Live appearance changes can change font metrics without resizing a pane.
    layout();
    Fl_Group::draw();
  }

public:
  std::array<CommandButton *, 3> actions{};
  Fl_Text_Display *reader = nullptr;
  using Fl_Group::Fl_Group;
  void resize(int x, int y, int w, int h) override {
    Fl_Group::resize(x, y, w, h);
    layout();
  }
};
class MessageList : public Fl_Table_Row {
  int activeRow = -1;
  void draw_cell(TableContext context, int row, int, int x, int y, int w,
                 int h) override {
    if (context != CONTEXT_CELL || row < 0 || row >= int(items.size()))
      return;
    if (y < this->y() || y + h > this->y() + this->h())
      return;
    const auto &item = items[row];
    bool selected = row_selected(row);
    fl_push_clip(std::max(x, tix), std::max(y, tiy),
                 std::max(0, std::min(x + w, tix + tiw) - std::max(x, tix)),
                 std::max(0, std::min(y + h, tiy + tih) - std::max(y, tiy)));
    fl_color(selected ? FL_SELECTION_COLOR : FL_BACKGROUND2_COLOR);
    fl_rectf(x, y, w, h);
    auto foreground = selected
                          ? fl_contrast(FL_FOREGROUND_COLOR, FL_SELECTION_COLOR)
                          : FL_FOREGROUND_COLOR;
    fl_color(foreground);
    fl_font(dx::font, dx::size);
    std::string sender =
        (item.value("unread", false) ? "● " : "") + str(item, "from");
    if (item.value("starred", false))
      sender = "* " + sender;
    fl_draw(sender.c_str(), x + 10, y + 4, w - 100, 20,
            FL_ALIGN_LEFT | FL_ALIGN_INSIDE | FL_ALIGN_CLIP);
    auto date = dateText(item.value("last_message_at", int64_t(0)), "%d %b");
    fl_draw(date.c_str(), x + w - 85, y + 4, 75, 20,
            FL_ALIGN_RIGHT | FL_ALIGN_INSIDE);
    auto subject = str(item, "subject");
    fl_draw(subject.c_str(), x + 10, y + 26, w - 20, 20,
            FL_ALIGN_LEFT | FL_ALIGN_INSIDE | FL_ALIGN_CLIP);
    fl_color(selected ? foreground : fl_inactive(foreground));
    auto snippet = str(item, "snippet");
    fl_draw(snippet.c_str(), x + 10, y + 48, w - 20, 18,
            FL_ALIGN_LEFT | FL_ALIGN_INSIDE | FL_ALIGN_CLIP);
    fl_color(fl_color_average(FL_FOREGROUND_COLOR, FL_BACKGROUND2_COLOR, .12f));
    fl_line(x, y + h - 1, x + w, y + h - 1);
    fl_pop_clip();
  }

public:
  Json items = Json::array();
  std::function<void(int)> chosen;
  MessageList(int x, int y, int w, int h) : Fl_Table_Row(x, y, w, h) {
    cols(1);
    rows(0);
    col_header(0);
    row_header(0);
    col_resize(0);
    row_resize(0);
    row_height_all(72);
    type(SELECT_MULTI);
    when(FL_WHEN_RELEASE);
    callback([](Fl_Widget *w, void *) {
      auto s = static_cast<MessageList *>(w);
      int row = s->callback_row();
      if (s->callback_context() == CONTEXT_CELL && row >= 0 && s->chosen)
        s->chosen(row);
    });
    end();
  }
  void resize(int x, int y, int w, int h) override {
    Fl_Table_Row::resize(x, y, w, h);
    col_width(0, std::max(30, w - 18));
  }
  void set(Json value, const std::string &key = "") {
    std::set<std::string> selected;
    for (int i = 0; i < int(items.size()); ++i)
      if (row_selected(i))
        selected.insert(items[i]["account_id"].dump() + ":" +
                        str(items[i], "id") + ":" +
                        items[i].value("message_id", Json()).dump());
    items = std::move(value);
    rows(items.size());
    row_height_all(72);
    col_width(0, std::max(30, w() - 18));
    for (int i = 0; i < int(items.size()); ++i)
      select_row(i,
                 selected.count(items[i]["account_id"].dump() + ":" +
                                str(items[i], "id") + ":" +
                                items[i].value("message_id", Json()).dump()) ||
                         (selected.empty() && str(items[i], "id") == key)
                     ? 1
                     : 0);
    redraw();
  }
  int handle(int event) override {
    if (event == FL_KEYDOWN && !Fl::event_state(FL_CTRL | FL_ALT | FL_META)) {
      int key = Fl::event_key(), row = activeRow;
      if (key == FL_Down)
        row = std::min(int(items.size()) - 1, row + 1);
      else if (key == FL_Up)
        row = std::max(0, row - 1);
      else if (key == FL_Home)
        row = 0;
      else if (key == FL_End)
        row = int(items.size()) - 1;
      else if (key == FL_Page_Down)
        row = std::min(int(items.size()) - 1, row + 8);
      else if (key == FL_Page_Up)
        row = std::max(0, row - 8);
      else if (key != FL_Enter)
        return Fl_Table_Row::handle(event);
      if (row >= 0 && row < int(items.size())) {
        if (!Fl::event_state(FL_SHIFT))
          for (int i = 0; i < int(items.size()); ++i)
            select_row(i, 0);
        select_row(row, 1);
        activeRow = row;
        row_position(row);
        redraw();
        if (chosen)
          chosen(row);
      }
      return 1;
    }
    if (event == FL_PUSH)
      take_focus();
    int result = Fl_Table_Row::handle(event);
    if (event == FL_RELEASE && callback_row() >= 0)
      activeRow = callback_row();
    return result;
  }
  Json targets() {
    Json out = Json::array();
    for (int i = 0; i < int(items.size()); ++i)
      if (row_selected(i)) {
        Json target = {{"account", items[i].value("account_id", 0)},
                       {"thread", str(items[i], "id")}};
        if (items[i].contains("message_id") &&
            items[i]["message_id"].is_string())
          target["message"] = items[i]["message_id"];
        out.push_back(target);
      }
    return out;
  }
};
class MonthGrid : public Fl_Widget {
public:
  int64_t first = 0, selected = 0;
  Json events = Json::array();
  std::function<void(int64_t)> chosen;
  MonthGrid(int x, int y, int w, int h) : Fl_Widget(x, y, w, h) {
    set(nowMs());
  }
  void set(int64_t date) {
    selected = date;
    time_t t = date / 1000;
    tm m{};
    localtime_r(&t, &m);
    m.tm_mday = 1;
    m.tm_hour = 0;
    m.tm_min = 0;
    m.tm_sec = 0;
    m.tm_isdst = -1;
    first = mktime(&m) * 1000;
    redraw();
  }
  int64_t day(int cell) const {
    time_t t = first / 1000;
    tm m{};
    localtime_r(&t, &m);
    m.tm_mday += cell - (m.tm_wday + 6) % 7;
    m.tm_isdst = -1;
    return mktime(&m) * 1000;
  }
  void move(int months) {
    time_t t = first / 1000;
    tm m{};
    localtime_r(&t, &m);
    m.tm_mon += months;
    m.tm_isdst = -1;
    set(mktime(&m) * 1000);
  }
  void draw() override {
    fl_push_clip(x(), y(), w(), h());
    fl_color(FL_BACKGROUND2_COLOR);
    fl_rectf(x(), y(), w(), h());
    const char *names[] = {_("Mon"), _("Tue"), _("Wed"), _("Thu"),
                           _("Fri"), _("Sat"), _("Sun")};
    fl_font(dx::font, dx::size);
    int cw = w() / 7, rh = (h() - 28) / 6;
    for (int c = 0; c < 7; ++c) {
      fl_color(FL_FOREGROUND_COLOR);
      fl_draw(names[c], x() + c * cw, y(), cw, 28, FL_ALIGN_CENTER);
    }
    for (int cell = 0; cell < 42; ++cell) {
      auto date = day(cell);
      int xx = x() + cell % 7 * cw, yy = y() + 28 + cell / 7 * rh;
      bool active =
          dateText(date, "%Y-%m-%d") == dateText(selected, "%Y-%m-%d");
      fl_color(active ? FL_SELECTION_COLOR : FL_BACKGROUND2_COLOR);
      fl_rectf(xx, yy, cw, rh);
      fl_color(active ? fl_contrast(FL_FOREGROUND_COLOR, FL_SELECTION_COLOR)
                      : FL_FOREGROUND_COLOR);
      auto label = dateText(date, "%d");
      if (dateText(date, "%Y-%m-%d") == dateText(nowMs(), "%Y-%m-%d"))
        label = "[" + label + "]";
      fl_draw(label.c_str(), xx + 5, yy + 2, cw - 10, 20,
              FL_ALIGN_LEFT | FL_ALIGN_INSIDE);
      int n = 0;
      for (const auto &e : events) {
        auto when = e["occurrence"].value("start", int64_t(0));
        if (dateText(date, "%Y-%m-%d") != dateText(when, "%Y-%m-%d"))
          continue;
        if (++n > std::max(0, (rh - 24) / 20))
          break;
        auto title = str(e["occurrence"]["event"], "title");
        fl_draw(title.c_str(), xx + 5, yy + 22 + (n - 1) * 20, cw - 10, 20,
                FL_ALIGN_LEFT | FL_ALIGN_INSIDE | FL_ALIGN_CLIP);
      }
      fl_color(
          fl_color_average(FL_FOREGROUND_COLOR, FL_BACKGROUND2_COLOR, .2f));
      fl_rect(xx, yy, cw, rh);
    }
    fl_pop_clip();
  }
  int handle(int event) override {
    if (event == FL_PUSH) {
      take_focus();
      return 1;
    }
    if (event == FL_RELEASE) {
      int cw = w() / 7, rh = (h() - 28) / 6;
      int c = (Fl::event_x() - x()) / cw, r = (Fl::event_y() - y() - 28) / rh;
      if (c >= 0 && c < 7 && r >= 0 && r < 6) {
        selected = day(r * 7 + c);
        redraw();
        if (chosen)
          chosen(selected);
      }
      return 1;
    }
    if (event == FL_KEYDOWN &&
        (Fl::event_key() == FL_Left || Fl::event_key() == FL_Right ||
         Fl::event_key() == FL_Up || Fl::event_key() == FL_Down)) {
      int delta = Fl::event_key() == FL_Left    ? -1
                  : Fl::event_key() == FL_Right ? 1
                  : Fl::event_key() == FL_Up    ? -7
                                                : 7;
      time_t t = selected / 1000;
      tm m{};
      localtime_r(&t, &m);
      m.tm_mday += delta;
      m.tm_isdst = -1;
      set(mktime(&m) * 1000);
      if (chosen)
        chosen(selected);
      return 1;
    }
    return Fl_Widget::handle(event);
  }
};
class MailWindow : public dx::GpuWindow {
  struct Binding {
    std::function<void()> fn;
  };
  std::vector<std::unique_ptr<Binding>> bindings;
  std::vector<std::unique_ptr<dx::GpuWindow>> dialogs;
  std::vector<std::unique_ptr<DesktopMenu>> dialogMenus;
  std::vector<std::unique_ptr<Fl_Text_Buffer>> buffers;
  DesktopMenuBar *menu;
  Fl_Choice *accountChoice, *space;
  Fl_Widget *toolbar[4]{};
  Fl_Button *saveAttach = nullptr;
  Fl_Input *search;
  Fl_Box *status;
  Fl_Group *mailPage, *calendarPage, *contactsPage;
  Fl_Tile *tile;
  Fl_Hold_Browser *sidebar, *attachments, *agenda, *contactsList;
  MessageList *list;
  Fl_Text_Display *reader;
  Fl_Text_Buffer readerBuffer;
  MonthGrid *month;
  Fl_Box *monthLabel;
  Fl_Input *contactSearch;
  std::unique_ptr<DesktopMenu> desktop;
  std::unique_ptr<StatusIcon> tray;
  std::unique_ptr<DesktopNotifications> desktopNotifications;
  Json pendingMailTarget, pendingCalendarTarget;
  Json accounts = Json::array(), boxes = Json::array(), opened = Json::array(),
       fileParts = Json::array(), calendarEvents = Json::array(),
       calendars = Json::array(), contactItems = Json::array();
  Json currentBox = {{"mailbox", "inbox"}}, currentTarget;
  std::string currentKey, smokePath, accountError;
  uint64_t listGeneration = 0, threadGeneration = 0, calendarGeneration = 0;
  bool ready = false, closing = false, refreshPending = false,
       accountsPending = false, needRefresh = false, hideRequested = false,
       hideHandled = false;
  bool notifications = true, eventReminders = true, hideToTray = true, threading = true;
  int64_t selectedAccount = 0;
  int unread = -1, shownSpace = 0, loadLimit = 500;
  std::unique_ptr<MailEngine> engine;
  void bind(Fl_Widget *widget, std::function<void()> fn) {
    auto b = std::make_unique<Binding>();
    b->fn = std::move(fn);
    widget->callback(
        [](Fl_Widget *, void *p) {
          auto b = static_cast<Binding *>(p);
          if (b->fn)
            b->fn();
        },
        b.get());
    bindings.push_back(std::move(b));
  }
  void addMenu(Fl_Menu_Bar &bar, const char *path, int key,
               std::function<void()> fn) {
    auto b = std::make_unique<Binding>();
    b->fn = std::move(fn);
    bar.add(
        path, key,
        [](Fl_Widget *, void *p) { static_cast<Binding *>(p)->fn(); }, b.get());
    bindings.push_back(std::move(b));
  }
  void say(const std::string &s) { copyLabel(status, s.c_str()); }
  bool accepted(const Json &reply) {
    if (reply.value("ok", false))
      return true;
    say(str(reply, "error", _("The request failed.")));
    return false;
  }
  void send(Json r, std::function<void(const Json &)> fn = {}) {
    if (!ready) {
      say(_("The mail engine is starting."));
      return;
    }
    engine->request(std::move(r), [this, fn](const Json &reply) {
      if (accepted(reply) && fn)
        fn(reply["result"]);
    });
  }
  std::filesystem::path prefsPath() const {
    const char *p = getenv("XDG_CONFIG_HOME");
    return std::filesystem::path(
               p ? p
                 : (std::string(getenv("HOME") ? getenv("HOME") : ".") +
                    "/.config")) /
           (isDemo ? "penguin-mail-demo" : "penguin-mail") / "fltk.json";
  }
  void savePrefs() {
    if (isDemo)
      return;
    try {
      auto path = prefsPath();
      std::filesystem::create_directories(path.parent_path());
      Json j = {{"notifications", notifications},
                {"event_reminders", eventReminders},
                {"hide_to_tray", hideToTray},
                {"threading", threading}};
      std::ofstream f(path);
      f << j.dump(2);
      std::filesystem::permissions(path,
                                   std::filesystem::perms::owner_read |
                                       std::filesystem::perms::owner_write);
    } catch (const std::exception &e) {
      say(e.what());
    }
  }
  void edit(int key) {
    auto focus = Fl::focus();
    if (auto input = dynamic_cast<Fl_Input_ *>(focus)) {
      if (key == 'c')
        input->copy(1);
      else if (key == 'x') {
        input->copy(1);
        input->cut();
      } else if (key == 'v')
        Fl::paste(*input, 1);
      else if (key == 'a')
        input->insert_position(input->size(), 0);
      else if (key == 'z')
        input->undo();
    } else if (auto editor = dynamic_cast<Fl_Text_Editor *>(focus)) {
      if (key == 'c')
        Fl_Text_Editor::kf_copy(0, editor);
      else if (key == 'x')
        Fl_Text_Editor::kf_cut(0, editor);
      else if (key == 'v')
        Fl_Text_Editor::kf_paste(0, editor);
      else if (key == 'a')
        Fl_Text_Editor::kf_select_all(0, editor);
      else if (key == 'z')
        Fl_Text_Editor::kf_undo(0, editor);
    } else if (auto display = dynamic_cast<Fl_Text_Display *>(focus)) {
      if (key == 'a')
        display->buffer()->select(0, display->buffer()->length());
      if (key == 'c') {
        char *text = display->buffer()->selection_text();
        if (text) {
          Fl::copy(text, strlen(text), 1);
          free(text);
        }
      }
    } else if (key == 'z')
      actionUndo();
  }
  void actionUndo() {
    send({{"op", "undo"}}, [this](const Json &) {
      say(_("Undone"));
      reload();
    });
  }
  void action(const std::string &command) {
    auto targets = list->targets();
    if (targets.empty() && !currentTarget.is_null())
      targets.push_back(currentTarget);
    if (targets.empty()) {
      say(_("Select a conversation first."));
      return;
    }
    Json r = {{"op", "action"}, {"action", command}, {"targets", targets}};
    if (command == "label") {
      const char *name = fl_input("%s", "", _("Label name"));
      if (!name || !*name)
        return;
      r["name"] = name;
    }
    if (command == "remind") {
      const char *at = fl_input("%s", dateText(nowMs() + 86400000).c_str(),
                                _("Remind at (YYYY-MM-DD HH:MM)"));
      if (!at)
        return;
      auto time = parseDate(at);
      if (time <= nowMs()) {
        fl_alert("%s", _("Choose a future date and time."));
        return;
      }
      r["at"] = time;
    }
    say(_("Updating mail…"));
    send(r, [this](const Json &) {
      say(_("Mail updated. Use Undo to reverse the action."));
      reload();
    });
  }
  void rebuildBoxes() {
    std::string key = currentBox.dump();
    boxes = Json::array();
    sidebar->clear();
    auto add = [&](const char *label, const char *kind) {
      boxes.push_back({{"mailbox", kind}});
      sidebar->add(label);
    };
    add(_("Inbox"), "inbox");
    add(_("Flagged"), "flagged");
    add(_("Sent"), "sent");
    add(_("Drafts"), "drafts");
    add(_("Archive"), "archive");
    add(_("Junk"), "junk");
    add(_("Trash"), "trash");
    add(_("All Mail"), "all");
    add(_("Send Later"), "scheduled");
    add(_("Outbox"), "outbox");
    add(_("Remind Me"), "reminders");
    add(_("Follow Up"), "follow-up");
    add(_("Muted"), "muted");
    for (const auto &item : accounts)
      if (item["account"].value("id", 0) == selectedAccount)
        for (const auto &label : item["labels"])
          if (str(label, "kind") == "User" || str(label, "kind") == "user" ||
              str(label, "kind") == "Tag") {
            auto name = str(label, "name");
            boxes.push_back({{"mailbox", "label"},
                             {"label", str(label, "id")},
                             {"name", name}});
            sidebar->add(name.c_str());
          }
    for (int i = 0; i < int(boxes.size()); ++i)
      if (boxes[i].dump() == key)
        sidebar->value(i + 1);
  }
  void reloadAccounts() {
    if (!ready || accountsPending)
      return;
    accountsPending = true;
    engine->request({{"op", "accounts"}}, [this](const Json &reply) {
      accountsPending = false;
      if (!accepted(reply))
        return;
      const auto &r = reply["result"];
      auto next = r["items"];
      if (next != accounts) {
        accounts = next;
        accountChoice->clear();
        accountChoice->add(_("All Accounts"));
        int selected = 0;
        for (int i = 0; i < int(accounts.size()); ++i) {
          auto a = accounts[i]["account"];
          std::string title = str(a, "email");
          if (!accounts[i].value("connected", false))
            title += " (" + str(a, "state") + ")";
          addChoice(accountChoice, title);
          if (a.value("id", 0) == selectedAccount)
            selected = i + 1;
        }
        accountChoice->value(selected);
        rebuildBoxes();
      }
      int count = r.value("unread", 0);
      if (unread != count) {
        unread = count;
        updateTitle();
        dockBadge(count);
      }
      if (pendingCompose && !accounts.empty()) {
        pendingCompose = false;
        composer("new", pendingRecipient);
        pendingRecipient.clear();
      }
    });
  }
  void reload() {
    if (!ready)
      return;
    needRefresh = false;
    reloadAccounts();
    if (shownSpace == 1) {
      reloadCalendar();
      return;
    }
    if (shownSpace == 2) {
      reloadContacts();
      return;
    }
    if (refreshPending) {
      needRefresh = true;
      return;
    }
    refreshPending = true;
    auto generation = listGeneration;
    Json r = currentBox;
    r["op"] = "list";
    r["account"] = selectedAccount;
    r["threading"] = threading;
    r["limit"] = loadLimit;
    engine->request(r, [this, generation](const Json &reply) {
      refreshPending = false;
      if (generation != listGeneration) {
        reload();
        return;
      }
      if (!accepted(reply))
        return;
      const auto &result = reply["result"];
      list->set(result["rows"], currentKey);
      bool found = false;
      for (const auto &item : list->items)
        if (str(item, "id") == currentKey)
          found = true;
      if (!found) {
        currentKey.clear();
        currentTarget = nullptr;
        opened = Json::array();
        readerBuffer.text("");
        attachments->clear();
        ++threadGeneration;
      }
      if (!pendingMailTarget.is_null()) {
        auto target = pendingMailTarget;
        pendingMailTarget = nullptr;
        openItem({{"account_id",target["account"]},{"id",target["thread"]},
                  {"message_id",target.value("message",Json())}});
      }
      if (needRefresh)
        reload();
      else
        say(std::to_string(list->items.size()) + " " + _("conversations") +
            (result.value("more", false)
                 ? " · " + std::string(_("Load More is available in View."))
                 : ""));
      if (!smokePath.empty()) {
        if (!list->items.empty()) {
          list->select_row(0, 1);
          open(0);
        } else
          finishSmoke();
      }
    });
  }
  void finishSmoke() {
    if (smokePath.empty())
      return;
    auto path = smokePath;
    smokePath.clear();
    Fl::add_timeout(
        .5,
        [](void *p) {
          auto s = static_cast<MailWindow *>(p);
          s->make_current();
          Fl::check();
          bool ok = dx::screenshot(*s, s->smokeOutput.c_str());
          printf("{\"toolkit\":\"FLTK\",\"messages\":%zu,\"demo\":%s}\n",
                 s->list->items.size(), s->isDemo ? "true" : "false");
          s->exitCode = ok ? 0 : 1;
          s->quit();
        },
        this);
    smokeOutput = path;
  }
  void open(int row) {
    if (row < 0 || row >= int(list->items.size()))
      return;
    openItem(list->items[row]);
  }
  void openItem(const Json &item) {
    if (currentKey == str(item, "id") && !currentTarget.is_null() &&
        currentTarget["account"] == item["account_id"])
      return;
    currentKey = str(item, "id");
    currentTarget = {{"account", item["account_id"]}, {"thread", currentKey}};
    if (item.contains("message_id") && item["message_id"].is_string())
      currentTarget["message"] = item["message_id"];
    auto generation = ++threadGeneration;
    readerBuffer.text(_("Loading conversation…"));
    attachments->clear();
    opened = Json::array();
    fileParts = Json::array();
    send({{"op", "thread"},
          {"account", item["account_id"]},
          {"thread", currentKey}},
         [this, generation](const Json &r) {
           if (generation != threadGeneration)
             return;
           opened = r["messages"];
           std::string text;
           for (const auto &part : opened) {
             const auto &meta = part["meta"];
             if (!text.empty())
               text += "\n\n────────────────────────────────\n\n";
             text += str(meta, "subject") + "\n";
             if (meta["from"].is_object())
               text += str(meta["from"], "email") + "\n";
             text += dateText(meta.value("date", int64_t(0))) + "\n" +
                     addressList(meta["to"]) + "\n\n";
             if (part.contains("mark")) {
               const auto &mark = part["mark"];
               text += "[OpenPGP] " + str(mark, "title") + "\n";
               auto detail = str(mark, "detail");
               if (!detail.empty()) text += detail + "\n";
               text += "\n";
             }
             text += str(part, "text");
             size_t fileIndex = 0;
             for (const auto &file : part["body"]["attachments"]) {
               Json f = file;
               f["message"] = meta["id"];
               if (part.contains("files") && fileIndex < part["files"].size())
                 f["opened_data"] = part["files"][fileIndex];
               ++fileIndex;
               fileParts.push_back(f);
               attachments->add(str(file, "filename", _("Attachment")).c_str());
             }
             if (!part["body"]["protection"].is_null() && !part.contains("mark"))
               text += "\n" + std::string(_("S/MIME message. Export the original to inspect its protection."));
           }
           readerBuffer.text(displayText(text).c_str());
           reader->scroll(0, 0);
           say(_("Conversation loaded"));
           finishSmoke();
         });
  }
  void saveAttachment() {
    int index = attachments->value() - 1;
    if (index < 0 || index >= int(fileParts.size()) || currentTarget.is_null())
      return;
    auto f = fileParts[index];
    Fl_Native_File_Chooser chooser;
    chooser.type(Fl_Native_File_Chooser::BROWSE_SAVE_FILE);
    chooser.title(_("Save Attachment"));
    chooser.preset_file(str(f, "filename", _("Attachment")).c_str());
    if (chooser.show())
      return;
    Json request = {{"op", "attachment"},
          {"account", currentTarget["account"]},
          {"message", f["message"]},
          {"attachment", str(f, "attachment_id", str(f, "part_id"))},
          {"path", chooser.filename()}};
    if (f.contains("opened_data")) request["opened_data"] = f["opened_data"];
    send(request, [this](const Json &) { say(_("Attachment saved")); });
  }
  void editDraft() {
    if (opened.empty() || currentTarget.is_null())
      return;
    send({{"op", "draft-open"},
          {"account", currentTarget["account"]},
          {"thread", currentKey},
          {"message", opened.back()["meta"]["id"]}},
         [this](const Json &r) { composer("draft", "", r); });
  }
  void queuedAction(bool remove) {
    if (currentKey.rfind("outbox:", 0) != 0) {
      say(_("Select an outgoing message first."));
      return;
    }
    if (remove && fl_choice("%s", _("Cancel"), _("Delete"), nullptr,
                            _("Delete this queued message?")) != 1)
      return;
    send({{"op", remove ? "outbox-delete" : "outbox-send"},
          {"queue", std::stoll(currentKey.substr(7))}},
         [this](const Json &) { reload(); });
  }
  void exportThread() {
    if (currentTarget.is_null())
      return;
    Fl_Native_File_Chooser chooser;
    chooser.type(Fl_Native_File_Chooser::BROWSE_SAVE_FILE);
    chooser.title(_("Export Conversation"));
    chooser.preset_file("conversation.mbox");
    if (chooser.show())
      return;
    send({{"op", "export"},
          {"account", currentTarget["account"]},
          {"thread", currentTarget["thread"]},
          {"path", chooser.filename()}},
         [this](const Json &) { say(_("Conversation exported")); });
  }
  void dockBadge(int count) {
    auto bus = g_bus_get_sync(G_BUS_TYPE_SESSION, nullptr, nullptr);
    if (!bus)
      return;
    GVariantBuilder props;
    g_variant_builder_init(&props, G_VARIANT_TYPE("a{sv}"));
    g_variant_builder_add(&props, "{sv}", "count", g_variant_new_int64(count));
    g_variant_builder_add(&props, "{sv}", "count-visible",
                          g_variant_new_boolean(count > 0));
    g_dbus_connection_emit_signal(
        bus, nullptr, "/io/github/c9dev/PenguinMail",
        "com.canonical.Unity.LauncherEntry", "Update",
        g_variant_new("(sa{sv})",
                      "application://io.github.c9dev.PenguinMail.desktop",
                      &props),
        nullptr);
    g_object_unref(bus);
  }
  void notificationAccount(int64_t account) {
    selectedAccount = account;
    accountChoice->value(0);
    for (int i = 0; i < int(accounts.size()); ++i)
      if (accounts[i]["account"].value("id", int64_t(0)) == account)
        accountChoice->value(i + 1);
    rebuildBoxes();
  }
  void notificationChosen(const Json &notice, const std::string &action) {
    if (str(notice, "kind") == "calendar") {
      if (action == "default") {
        notificationAccount(notice["target"].value("account_id", int64_t(0)));
        ++calendarGeneration;
        pendingCalendarTarget = notice["target"];
        openCalendar(str(notice, "date"));
      } else {
        send({{"op","notification-action"},{"target",notice["target"]},{"action",action}},
          [this](const Json &reply) {
            auto url = str(reply,"url");
            if (url.rfind("https://",0) == 0) dx::spawn({"xdg-open",url});
            else say(_("Reminder snoozed"));
          });
      }
    } else if (action == "default") {
      activate();
      notificationAccount(notice.value("account", int64_t(0)));
      currentBox = {{"mailbox","inbox"}};
      search->value("");
      ++listGeneration;
      pendingMailTarget = notice.value("target",Json());
      rebuildBoxes();
      changeSpace(0);
    }
  }
  void notificationSettings() {
    if (!ready) return;
    send({{"op","notification-settings"},{"reminders",eventReminders && !isDemo}});
    if (!notifications) desktopNotifications->withdrawKind("mail");
    if (!eventReminders) desktopNotifications->withdrawKind("calendar");
  }
  dx::GpuWindow *dialog(int width, int height, const char *title) {
    Fl_Group::current(nullptr);
    auto window = std::make_unique<dx::GpuWindow>(width, height, title);
    window->xclass("io.github.c9dev.PenguinMail");
    window->callback([](Fl_Widget *w, void *) { w->hide(); });
    auto ptr = window.get();
    dialogs.push_back(std::move(window));
    return ptr;
  }
  void showDialog(dx::GpuWindow *window) {
    window->end();
    dx::style(window);
    window->show();
  }
  void composer(const std::string &mode = "new",
                const std::string &recipient = "", Json saved = Json()) {
    struct Form {
      dx::GpuWindow *window;
      Fl_Choice *from;
      Fl_Input *to, *cc, *bcc, *subject;
      Fl_Text_Editor *editor;
      Fl_Hold_Browser *files;
      Fl_Box *status;
      DesktopMenuBar *menu;
      Fl_Button *send, *save;
      Fl_Check_Button *sign, *encrypt;
      Fl_Box *keys;
      Json senders;
      uint64_t keyGeneration = 0;
      bool quotedEncrypted = false;
      std::shared_ptr<Fl_Text_Buffer> body = std::make_shared<Fl_Text_Buffer>();
      std::vector<std::string> paths;
      Json base, kept = Json::array();
      std::string draftId;
      bool busy = false, closeAfterSave = false;
      int menuHeight = 26;
    };
    if (accounts.empty()) {
      say(_("Add an account before composing mail."));
      return;
    }
    auto f = std::make_shared<Form>();
    f->senders = accounts;
    f->window = dialog(840, 800, _("Compose Message"));
    f->menu = new DesktopMenuBar(0, 0, 840, 26);
    f->send = new CommandButton(14, 40, 105, 32, _("Send"), [] {});
    f->save = new CommandButton(130, 40, 140, 32, _("Save Draft"), [] {});
    f->from = new Fl_Choice(100, 90, 720, 28, _("From"));
    int selected = 0;
    for (int i = 0; i < int(accounts.size()); ++i) {
      addChoice(f->from, str(accounts[i]["account"], "email"));
      if (accounts[i]["account"].value("id", 0) == selectedAccount)
        selected = i;
    }
    f->from->value(selected);
    f->sign = new Fl_Check_Button(285, 40, 150, 32, _("Sign (OpenPGP)"));
    f->encrypt = new Fl_Check_Button(440, 40, 170, 32, _("Encrypt (OpenPGP)"));
    f->sign->deactivate(); f->encrypt->deactivate();
    f->to = new Fl_Input(100, 130, 720, 28, _("To"));
    f->cc = new Fl_Input(100, 168, 720, 28, _("Cc"));
    f->bcc = new Fl_Input(100, 206, 720, 28, _("Bcc"));
    f->subject = new Fl_Input(100, 244, 720, 28, _("Subject"));
    f->editor = new Fl_Text_Editor(14, 290, 806, 294, _("Message"));
    f->editor->align(FL_ALIGN_TOP_LEFT);
    f->editor->buffer(f->body.get());
    f->editor->wrap_mode(Fl_Text_Display::WRAP_AT_BOUNDS, 0);
    f->files = new Fl_Hold_Browser(14, 600, 600, 65, _("Attachments"));
    f->files->align(FL_ALIGN_TOP_LEFT);
    f->files->format_char(0);
    new CommandButton(630, 600, 190, 28, _("Attach File…"), [f] {
      Fl_Native_File_Chooser c;
      c.title(_("Attach File"));
      c.type(Fl_Native_File_Chooser::BROWSE_FILE);
      if (!c.show()) {
        f->paths.push_back(c.filename());
        f->files->add(std::filesystem::path(c.filename()).filename().c_str());
      }
    });
    new CommandButton(630, 636, 190, 28, _("Remove Attachment"), [f] {
      int row = f->files->value() - 1;
      if (row >= 0 && row < int(f->kept.size() + f->paths.size())) {
        if (row < int(f->kept.size()))
          f->kept.erase(row);
        else
          f->paths.erase(f->paths.begin() + row - int(f->kept.size()));
        f->files->remove(row + 1);
      }
    });
    f->keys = new Fl_Box(14, 680, 806, 46, _("Checking local OpenPGP keys…"));
    f->keys->align(FL_ALIGN_LEFT | FL_ALIGN_INSIDE | FL_ALIGN_WRAP);
    f->status = new Fl_Box(14, 740, 806, 45);
    f->status->align(FL_ALIGN_LEFT | FL_ALIGN_INSIDE | FL_ALIGN_WRAP);
    f->to->value(recipient.c_str());
    if (!saved.is_null()) {
      f->base = saved;
      f->draftId = str(saved, "draft_id");
      f->sign->value(saved.value("sign", false));
      f->encrypt->value(saved.value("encrypt", false));
      f->kept = saved.value("kept_files", Json::array());
      f->to->value(str(saved, "to").c_str());
      f->cc->value(str(saved, "cc").c_str());
      f->bcc->value(str(saved, "bcc").c_str());
      f->subject->value(str(saved, "subject").c_str());
      f->body->text(displayText(str(saved, "text")).c_str());
      for (const auto &file : f->kept)
        f->files->add(str(file, "name").c_str());
      for (int i = 0; i < int(accounts.size()); ++i)
        if (accounts[i]["account"]["id"] == saved["account"])
          f->from->value(i);
    }
    if (mode != "new" && mode != "draft" && !opened.empty()) {
      const auto &part = opened.back();
      const auto &meta = part["meta"];
      f->quotedEncrypted = part.value("sealed", false);
      f->encrypt->value(f->quotedEncrypted);
      auto subject = str(meta, "subject");
      if (mode == "forward")
        f->subject->value(("Fwd: " + subject).c_str());
      else {
        f->subject->value(
            (subject.rfind("Re:", 0) == 0 ? subject : "Re: " + subject)
                .c_str());
        f->to->value(str(meta["from"], "email").c_str());
        if (mode == "reply-all")
          f->cc->value(
              (addressList(meta["to"]) + ", " + addressList(meta["cc"]))
                  .c_str());
        f->base["thread"] = currentKey;
        f->base["in_reply_to"] = meta["rfc822_msgid"];
      }
      std::istringstream lines(displayText(str(part, "text")));
      std::string text = "\n\n", line;
      while (std::getline(lines, line))
        text += "> " + line + "\n";
      f->body->text(text.c_str());
      for (int i = 0; i < int(accounts.size()); ++i)
        if (accounts[i]["account"].value("id", 0) ==
            currentTarget.value("account", 0))
          f->from->value(i);
    }
    auto submit = [this, f](bool draft, bool later) {
      if (f->busy)
        return;
      Json r = f->base;
      r["op"] = "compose";
      r["draft"] = draft;
      r["account"] = f->senders.at(f->from->value())["account"]["id"];
      r["sign"] = bool(f->sign->value());
      r["encrypt"] = bool(f->encrypt->value());
      if (!draft && f->quotedEncrypted && !f->encrypt->value() &&
          fl_choice("%s", _("Cancel"), _("Send Unencrypted"), nullptr,
            _("This message quotes decrypted mail. Send that text without encryption?")) != 1)
        return;
      r["to"] = f->to->value();
      r["cc"] = f->cc->value();
      r["bcc"] = f->bcc->value();
      r["subject"] = f->subject->value();
      char *text = f->body->text();
      r["text"] = text ? text : "";
      free(text);
      r["files"] = f->paths;
      r["kept_files"] = f->kept;
      if (!f->draftId.empty())
        r["draft_id"] = f->draftId;
      if (later) {
        const char *at = fl_input("%s", dateText(nowMs() + 3600000).c_str(),
                                  _("Send at (YYYY-MM-DD HH:MM)"));
        if (!at)
          return;
        auto time = parseDate(at);
        if (time <= nowMs()) {
          fl_alert("%s", _("Choose a future date and time."));
          return;
        }
        r["send_at"] = time;
      }
      if (!draft && !strlen(f->subject->value()) &&
          fl_choice("%s", _("Cancel"), _("Send"), nullptr,
                    _("Send without a subject?")) != 1)
        return;
      f->busy = true;
      f->send->deactivate();
      f->save->deactivate();
      copyLabel(f->status, _("Working…"));
      engine->request(r, [this, f, draft](const Json &reply) {
        f->busy = false;
        f->send->activate();
        f->save->activate();
        if (!reply.value("ok", false)) {
          copyLabel(f->status, str(reply, "error").c_str());
          return;
        }
        if (draft) {
          f->draftId = str(reply["result"], "draft_id");
          copyLabel(f->status, _("Draft saved"));
          if (f->closeAfterSave)
            f->window->hide();
        } else {
          f->window->hide();
          say(reply["result"].value("queued", false)
                  ? _("Message queued in Outbox or Send Later.")
                  : _("Message sent"));
        }
        reload();
      });
    };
    auto checkKeys = [this, f] {
      if (!f->window->shown() && f->keyGeneration) return;
      auto generation = ++f->keyGeneration;
      Json request = {{"op", "crypto-keys"},
        {"from", str(f->senders.at(f->from->value())["account"], "email")},
        {"to", f->to->value()}, {"cc", f->cc->value()}, {"bcc", f->bcc->value()}};
      engine->request(request, [f, generation](const Json &reply) {
        if (generation != f->keyGeneration) return;
        if (!reply.value("ok", false)) {
          copyLabel(f->keys, str(reply, "error").c_str());
          return;
        }
        const auto &r = reply["result"];
        f->sign->activate(); f->encrypt->activate();
        std::string info = r.value("can_sign", false)
          ? _("A secret key is available for signing.") : _("No secret signing key for this sender.");
        info += " ";
        info += r.value("can_encrypt", false)
          ? _("All recipients and your Sent copy have encryption keys.")
          : str(r, "cannot_encrypt");
        for (const auto &item : r["keys"]) {
          if (!item["key"].is_null()) {
            auto trust = str(item["key"], "trust");
            if (trust != "Full" && trust != "Ultimate") {
              info += " "; info += _("Key identity is not fully trusted:");
              info += " " + str(item, "address");
            }
          }
        }
        copyLabel(f->keys, info.c_str());
      });
    };
    for (auto input : {f->to, f->cc, f->bcc}) {
      input->when(FL_WHEN_RELEASE);
      bind(input, checkKeys);
    }
    bind(f->from, checkKeys);
    new CommandButton(625, 40, 195, 32, _("Check Keys"), checkKeys);
    bind(f->send, [submit] { submit(false, false); });
    bind(f->save, [submit] { submit(true, false); });
    addMenu(*f->menu, _("File/Send"), FL_CTRL | FL_Enter,
            [submit] { submit(false, false); });
    addMenu(*f->menu, _("File/Send Later…"), 0,
            [submit] { submit(false, true); });
    addMenu(*f->menu, _("File/Save Draft"), FL_CTRL | 's',
            [submit] { submit(true, false); });
    addMenu(*f->menu, _("File/Close"), FL_CTRL | 'w',
            [f] { f->window->do_callback(); });
    for (auto key : {'z', 'x', 'c', 'v', 'a'}) {
      const char *path = key == 'z'   ? _("Edit/Undo")
                         : key == 'x' ? _("Edit/Cut")
                         : key == 'c' ? _("Edit/Copy")
                         : key == 'v' ? _("Edit/Paste")
                                      : _("Edit/Select All");
      addMenu(*f->menu, path, FL_CTRL | key, [this, key] { edit(key); });
    }
    bind(f->window, [f, submit] {
      if (f->busy) {
        copyLabel(f->status, _("Wait for the current request to finish."));
        return;
      }
      int choice = fl_choice(
          "%s", _("Cancel"), _("Discard"), _("Save Draft"),
          _("Close this message? Save a draft to keep your changes."));
      if (choice == 1)
        f->window->hide();
      else if (choice == 2) {
        f->closeAfterSave = true;
        submit(true, false);
      }
    });
    f->window->resizable(f->editor);
    f->window->size_range(700, 650);
    f->menu->layoutChanged = [f] {
      int delta = f->menu->h() - f->menuHeight;
      f->menuHeight = f->menu->h();
      int bottom = f->editor->y() + f->editor->h();
      for (int i = 0; i < f->window->children(); ++i) {
        auto widget = f->window->child(i);
        if (widget != f->menu && widget != f->editor && widget->y() < bottom)
          widget->position(widget->x(), widget->y() + delta);
      }
      f->editor->resize(f->editor->x(), f->editor->y() + delta,
                        f->editor->w(), f->editor->h() - delta);
      f->window->init_sizes();
    };
    showDialog(f->window);
    checkKeys();
    dialogMenus.push_back(std::make_unique<DesktopMenu>(*f->window, *f->menu));
    f->to->take_focus();
  }
  void browserSignIn(dx::GpuWindow *window, const std::string &provider) {
    engine->request({{"op", "oauth-status"}}, [this, window, provider](const Json &status) {
      if (!accepted(status)) return;
      Json request = {{"op", "add-browser"}, {"provider", provider}};
      if (!status["result"].value(provider, false)) {
        if (provider != "google") {
          fl_alert("%s", _("This build has no Microsoft sign-in client.")); return;
        }
        int choice = fl_choice("%s", _("Cancel"), _("Choose Client JSON…"), _("Setup Instructions"),
          _("This build has no Google sign-in client.\nChoose the Desktop app OAuth client JSON from your Google Cloud project.\nAfter sign-in, the account client and refresh token will be saved\nin your desktop keyring."));
        if (choice == 2) {
          dx::spawn({"xdg-open", "https://github.com/c9dev/penguin-mail/blob/main/docs/setup.md#your-own-google-client"}); return;
        }
        if (choice != 1) return;
        Fl_Native_File_Chooser picker;
        picker.type(Fl_Native_File_Chooser::BROWSE_FILE); picker.title(_("Google Desktop OAuth Client"));
        picker.filter("JSON\t*.json");
        if (picker.show()) return;
        request["client_file"] = picker.filename();
      }
      say(_("Complete sign-in in your browser."));
      engine->request(request, [this, window](const Json &reply) {
        if (!accepted(reply)) { fl_alert("%s", str(reply, "error").c_str()); return; }
        window->hide(); reload();
      });
    });
  }
  #ifdef PENGUIN_ENABLE_MISFIN
  #include "misfin.h"
  #endif
  void accountDialog() {
    if (isDemo) {
      fl_message("%s", _("Demo mode uses temporary sample accounts. Start "
                         "without --demo to add an account."));
      return;
    }
    auto win = dialog(760, 650, _("Add Account"));
    new CommandButton(20, 20, 220, 34, _("Google…"), [this, win] {browserSignIn(win, "google");});
    new CommandButton(255, 20, 220, 34, _("Microsoft…"), [this, win] {browserSignIn(win, "microsoft");});
    auto kind = new Fl_Choice(500, 20, 230, 34);
    kind->tooltip(_("Account Type"));
    kind->add("IMAP|POP3|JMAP|CalDAV");
    kind->value(0);
    auto address = new Fl_Input(180, 80, 550, 28, _("Email Address"));
    auto username = new Fl_Input(180, 120, 550, 28, _("User Name"));
    auto password = new Fl_Secret_Input(180, 160, 550, 28, _("Password"));
    auto ih = new Fl_Input(180, 225, 410, 28, _("IMAP Server"));
    auto ip = new Fl_Int_Input(610, 225, 120, 28, _("Port"));
    ip->value("993");
    auto itls = new Fl_Choice(180, 265, 550, 28, _("IMAP Security"));
    itls->add("TLS|STARTTLS");
    itls->value(0);
    auto sh = new Fl_Input(180, 320, 410, 28, _("SMTP Server"));
    auto sp = new Fl_Int_Input(610, 320, 120, 28, _("Port"));
    sp->value("465");
    auto stls = new Fl_Choice(180, 360, 550, 28, _("SMTP Security"));
    stls->add("TLS|STARTTLS");
    stls->value(0);
    auto su = new Fl_Input(180, 400, 550, 28, _("SMTP User Name"));
    auto pw = new Fl_Secret_Input(180, 440, 550, 28, _("SMTP Password"));
    auto info = new Fl_Box(
        20, 480, 710, 85,
        _("Leave user names and the SMTP password empty to use the incoming "
          "account's details. Passwords are stored in your desktop keyring."));
    info->align(FL_ALIGN_LEFT | FL_ALIGN_INSIDE | FL_ALIGN_WRAP);
    new CommandButton(
        20, 585, 200, 34, _("Find Servers"),
        [this, address, ih, ip, itls, sh, sp, stls, info] {
          send({{"op", "discover"}, {"address", address->value()}},
               [address, ih, ip, itls, sh, sp, stls, info](const Json &r) {
                 const auto &c = r["candidates"];
                 if (c.empty()) {
                   copyLabel(info, _(
                       "No servers found. Enter the server details manually."));
                   return;
                 }
                 const auto &v = c[0];
                 if (v.value("confirm", false) &&
                     fl_choice("%s", _("Cancel"), _("Use Servers"), nullptr,
                               _("These servers were discovered outside your "
                                 "email domain. Use them?")) != 1)
                   return;
                 if (v["imap"].is_object()) {
                   ih->value(str(v["imap"], "host").c_str());
                   ip->value(
                       std::to_string(v["imap"].value("port", 993)).c_str());
                   itls->value(str(v["imap"], "security") == "tls" ? 0 : 1);
                 }
                 sh->value(str(v["smtp"], "host").c_str());
                 sp->value(
                     std::to_string(v["smtp"].value("port", 465)).c_str());
                 stls->value(str(v["smtp"], "security") == "tls" ? 0 : 1);
                 copyLabel(info, address->value());
               });
        });
    bind(kind, [kind, ih, ip, itls, sh, sp, stls, su, pw] {
      int type = kind->value();
      copyLabel(ih, type == 1   ? _("POP3 Server")
                     : type >= 2 ? _("Server URL")
                                 : _("IMAP Server"));
      for (Fl_Widget *field : std::vector<Fl_Widget *>{sh, sp, stls, su, pw}) {
        if (type >= 2)
          field->hide();
        else
          field->show();
      }
      if (type >= 2)
        ip->hide();
      else {
        ip->show();
        ip->value(type == 1 ? "995" : "993");
      }
      itls->clear();
      if (type == 2) {
        copyLabel(itls, _("Authentication"));
        itls->add(_("API Token"));
        itls->add(_("Password"));
        itls->show();
      } else {
        copyLabel(itls, _("Security"));
        itls->add("TLS|STARTTLS");
        if (type == 3)
          itls->hide();
        else
          itls->show();
      }
      itls->value(0);
      ih->window()->redraw();
    });
    auto connect =
        new CommandButton(530, 585, 200, 34, _("Add Account"), [] {});
    bind(connect, [this, win, kind, address, username, password, ih, ip, itls,
                   sh, sp, stls, su, pw, info, connect] {
      int incoming = atoi(ip->value()), outgoing = atoi(sp->value());
      if (incoming < 1 || incoming > 65535 || outgoing < 1 ||
          outgoing > 65535) {
        copyLabel(info, _("Enter ports between 1 and 65535."));
        return;
      }
      Json r = {{"op", "add-imap"},
                {"address", address->value()},
                {"username", username->value()},
                {"password", password->value()},
                {"smtp_username", su->value()},
                {"smtp_password", pw->value()},
                {"provider_name", "IMAP"},
                {"imap",
                 {{"host", ih->value()},
                  {"port", incoming},
                  {"security", itls->value() ? "start_tls" : "tls"},
                  {"user_name", "address"}}},
                {"smtp",
                 {{"host", sh->value()},
                  {"port", outgoing},
                  {"security", stls->value() ? "start_tls" : "tls"},
                  {"user_name", "address"}}}};
      if (kind->value() == 1)
        r["op"] = "add-pop3";
      if (kind->value() >= 2) {
        r["op"] = kind->value() == 2 ? "add-jmap" : "add-caldav";
        r["url"] = ih->value();
        r["bearer"] = itls->value() == 0;
      }
      connect->deactivate();
      copyLabel(info, _("Checking the incoming and outgoing servers…"));
      engine->request(
          r, [this, win, password, pw, connect, info](const Json &reply) {
            connect->activate();
            if (!reply.value("ok", false)) {
              copyLabel(info, str(reply, "error").c_str());
              return;
            }
            password->value("");
            pw->value("");
            win->hide();
            reload();
          });
    });
    showDialog(win);
  }
  void openPgp() {
    auto win = dialog(760, 490, _("OpenPGP Keys"));
    auto info = new Fl_Box(20, 20, 720, 70,
      isDemo ? _("Demo mode uses a temporary GnuPG keyring. Imported keys are discarded when the preview closes.") :
      _("OpenPGP uses your GnuPG keyring and pinentry. Import a public or secret key file, or manage keys with your usual GnuPG tools. Keys are never fetched automatically."));
    info->align(FL_ALIGN_LEFT | FL_ALIGN_INSIDE | FL_ALIGN_WRAP);
    auto addresses = new Fl_Input(135, 115, 590, 28, _("Addresses"));
    if (selectedAccount) for (const auto &a : accounts)
      if (a["account"].value("id", 0) == selectedAccount)
        addresses->value(str(a["account"], "email").c_str());
    buffers.push_back(std::make_unique<Fl_Text_Buffer>());
    auto buffer = buffers.back().get();
    auto view = new Fl_Text_Display(20, 165, 720, 245);
    view->buffer(buffer); view->wrap_mode(Fl_Text_Display::WRAP_AT_BOUNDS, 0);
    auto inspect = [this, addresses, buffer] {
      engine->request({{"op", "crypto-keys"}, {"to", addresses->value()}},
        [buffer](const Json &reply) {
          if (!reply.value("ok", false)) {buffer->text(str(reply, "error").c_str()); return;}
          std::string text;
          for (const auto &item : reply["result"]["keys"]) {
            text += str(item, "address") + "\n";
            const auto &key = item["key"];
            text += key.is_null() ? std::string(_("No encryption key")) :
              str(key, "user_id") + "\n" + str(key, "fingerprint") + "\n" + str(key, "trust");
            text += "\n\n";
          }
          buffer->text(text.c_str());
        });
    };
    new CommandButton(20, 435, 200, 34, _("Check Local Keys"), inspect);
    new CommandButton(240, 435, 220, 34, _("Import Key File…"), [this, buffer] {
      Fl_Native_File_Chooser picker; picker.type(Fl_Native_File_Chooser::BROWSE_FILE);
      picker.title(_("Import OpenPGP Key"));
      if (picker.show()) return;
      auto path = std::string(picker.filename());
      if (fl_choice("%s", _("Cancel"), _("Import"), nullptr,
          _("Import this file into your GnuPG keyring?")) != 1) return;
      buffer->text(_("Importing key…"));
      engine->request({{"op", "crypto-import"}, {"path", path}}, [buffer](const Json &reply) {
        if (!reply.value("ok", false)) { buffer->text(str(reply, "error").c_str()); return; }
        std::string text = _("Imported keys:"); text += "\n";
        for (const auto &key : reply["result"]["keys"])
          text += str(key, "name") + " <" + str(key, "address") + ">\n" + str(key, "fingerprint") + "\n";
        buffer->text(text.c_str());
      });
    });
    new CommandButton(560, 435, 180, 34, _("Close"), [win] {win->hide();});
    showDialog(win);
    send({{"op", "crypto-status"}}, [buffer](const Json &r) {
      buffer->text(r.value("available", false) ?
        (_("GnuPG version:") + std::string(" ") + str(r, "version")).c_str() :
        _("This computer has no gpg. Install GnuPG to read or send OpenPGP mail."));
    });
  }
  void preferences() {
    auto win = dialog(620, 440, _("Preferences"));
    auto startup = new Fl_Check_Button(20, 20, 580, 30, _("Start Penguin Mail at login"));
    startup->value(login_startup::enabled());
    if (isDemo) startup->deactivate();
    auto hide = new Fl_Check_Button(
        20, 60, 580, 30, _("Keep syncing in the tray when the window closes"));
    hide->value(hideToTray);
    auto notices =
        new Fl_Check_Button(20, 100, 580, 30, _("Show new mail notifications"));
    notices->value(notifications);
    auto reminders = new Fl_Check_Button(20, 140, 580, 30, _("Event Reminders"));
    reminders->value(eventReminders);
    auto grouped = new Fl_Check_Button(20, 180, 580, 30,
                                       _("Group messages into conversations"));
    grouped->value(threading);
    auto poll =
        new Fl_Int_Input(250, 225, 340, 28, _("Sync Interval (seconds)"));
    poll->value("30");
    auto days =
        new Fl_Int_Input(250, 265, 340, 28, _("Local Mail Window (days)"));
    days->value("30");
    auto info = new Fl_Box(
        20, 305, 580, 60,
        _("Colors, accent, font and bitmap strike follow DomeD. Sync interval "
          "and mail window changes apply after restart."));
    info->align(FL_ALIGN_LEFT | FL_ALIGN_INSIDE | FL_ALIGN_WRAP);
    new CommandButton(420, 380, 170, 34, _("Save"),
                      [this, win, startup, hide, notices, reminders, grouped, poll, days] {
                        if (!isDemo) {
                          try { login_startup::apply(startup->value()); }
                          catch (const std::exception &e) {
                            fl_alert("%s\n%s", _("Could not update login startup"), e.what());
                            return;
                          }
                        }
                        hideToTray = hide->value();
                        notifications = notices->value();
                        eventReminders = reminders->value();
                        notificationSettings();
                        threading = grouped->value();
                        savePrefs();
                        send({{"op", "settings-save"},
                              {"poll_seconds", atoi(poll->value())},
                              {"window_days", atoi(days->value())}},
                             [this, win](const Json &) {
                               win->hide();
                               ++listGeneration;
                               reload();
                             });
                      });
    send({{"op", "settings"}}, [poll, days](const Json &r) {
      auto s = r["sync"];
      if (s.contains("poll_seconds"))
        poll->value(std::to_string(s.value("poll_seconds", 30)).c_str());
      if (s.contains("window_days"))
        days->value(std::to_string(s.value("window_days", 30)).c_str());
    });
    showDialog(win);
  }
  void reloadCalendar() {
    auto generation = ++calendarGeneration;
    copyLabel(monthLabel, dateText(month->first, "%B %Y").c_str());
    updateTitle();
    send({{"op", "calendar"},
          {"account", selectedAccount},
          {"from", month->day(0)},
          {"to", month->day(41) + 2 * 86400000}},
         [this, generation](const Json &r) {
           if (generation != calendarGeneration)
             return;
           calendarEvents = r["events"];
           calendars = r["calendars"];
           month->events = calendarEvents;
           month->redraw();
           updateAgenda();
           say(_("Calendar loaded"));
         });
  }
  void updateAgenda() {
    agenda->clear();
    for (const auto &e : calendarEvents) {
      auto when = e["occurrence"].value("start", int64_t(0));
      auto label =
          dateText(when) + "  " + str(e["occurrence"]["event"], "title");
      agenda->add(label.c_str());
    }
    for (int i = 0; i < int(calendarEvents.size()); ++i)
      if (dateText(calendarEvents[i]["occurrence"].value("start", int64_t(0)),
                   "%Y-%m-%d") == dateText(month->selected, "%Y-%m-%d")) {
        agenda->value(i + 1);
        break;
      }
    if (!pendingCalendarTarget.is_null()) {
      for (int i = 0; i < int(calendarEvents.size()); ++i) {
        const auto &e = calendarEvents[i];
        const auto &t = pendingCalendarTarget;
        if (e["account"] == t["account_id"] && e["occurrence"]["start"] == t["start"] &&
            e["occurrence"]["event"]["id"] == t["event"] &&
            e["occurrence"]["event"]["calendar"] == t["calendar"]) {
          agenda->value(i + 1); agenda->middleline(i + 1); break;
        }
      }
      pendingCalendarTarget = nullptr;
    }
  }
  void eventDialog(bool edit) {
    Json selected;
    int row = agenda->value() - 1;
    if (edit && row >= 0 && row < int(calendarEvents.size()))
      selected = calendarEvents[row];
    else if (edit)
      return;
    auto win = dialog(710, 510, edit ? _("Edit Event") : _("New Event"));
    auto cal = new Fl_Choice(150, 25, 535, 28, _("Calendar"));
    Json writable = Json::array();
    for (const auto &c : calendars) {
      auto a = str(c["calendar"], "access");
      if (a == "Owner" || a == "Writer") {
        writable.push_back(c);
        addChoice(cal, str(c["calendar"], "name"));
      }
    }
    if (writable.empty()) {
      win->end();
      say(_("No writable calendar is available."));
      return;
    }
    cal->value(0);
    auto title = new Fl_Input(150, 70, 535, 28, _("Title"));
    auto start = new Fl_Input(150, 115, 535, 28, _("Start"));
    auto end = new Fl_Input(150, 160, 535, 28, _("End"));
    auto place = new Fl_Input(150, 205, 535, 28, _("Location"));
    auto desc = new Fl_Input(150, 250, 535, 100, _("Notes"));
    desc->type(FL_MULTILINE_INPUT);
    auto info = new Fl_Box(20, 365, 665, 55,
                           _("Enter local dates as YYYY-MM-DD HH:MM. Changes "
                             "are sent to the calendar provider."));
    info->align(FL_ALIGN_LEFT | FL_ALIGN_INSIDE | FL_ALIGN_WRAP);
    if (edit) {
      const auto &o = selected["occurrence"];
      const auto &e = o["event"];
      title->value(str(e, "title").c_str());
      place->value(str(e, "place").c_str());
      desc->value(str(e, "description").c_str());
      start->value(dateText(o.value("start", int64_t(0))).c_str());
      end->value(dateText(o.value("end", int64_t(0))).c_str());
      for (int i = 0; i < int(writable.size()); ++i)
        if (writable[i]["account"] == selected["account"] &&
            writable[i]["calendar"]["id"] == e["calendar"])
          cal->value(i);
      cal->deactivate();
    } else {
      start->value((dateText(month->selected, "%Y-%m-%d") + " 09:00").c_str());
      end->value((dateText(month->selected, "%Y-%m-%d") + " 10:00").c_str());
    }
    new CommandButton(
        510, 450, 175, 34, _("Save Event"),
        [this, win, cal, writable, title, start, end, place, desc, info,
         selected, edit] {
          auto from = parseDate(start->value()), to = parseDate(end->value());
          if (from == InvalidDate || to <= from || !strlen(title->value())) {
            copyLabel(info,
                _("Enter a title and a valid start and end time."));
            return;
          }
          const auto &c = writable[cal->value()];
          send({{"op", "event-save"},
                {"account", c["account"]},
                {"calendar", c["calendar"]["id"]},
                {"event", edit ? str(selected["occurrence"], "id") : ""},
                {"title", title->value()},
                {"start", from},
                {"end", to},
                {"place", place->value()},
                {"description", desc->value()}},
               [this, win](const Json &) {
                 win->hide();
                 reloadCalendar();
               });
        });
    showDialog(win);
  }
  void deleteEvent() {
    int row = agenda->value() - 1;
    if (row < 0 || row >= int(calendarEvents.size()))
      return;
    auto e = calendarEvents[row];
    if (fl_choice("%s", _("Cancel"), _("Delete Event"), nullptr,
                  _("Delete this event and notify its guests?")) != 1)
      return;
    send({{"op", "event-delete"},
          {"account", e["account"]},
          {"event", e["occurrence"]["id"]}},
         [this](const Json &) { reloadCalendar(); });
  }
  void reloadContacts() {
    send({{"op", "contacts"}}, [this](const Json &r) {
      contactItems = r["items"];
      filterContacts();
      say(_("Contacts loaded"));
    });
  }
  void filterContacts() {
    for (int i = 1; i <= contactsList->size(); ++i)
      delete static_cast<std::string *>(contactsList->data(i));
    contactsList->clear();
    std::string filter = contactSearch->value();
    std::transform(filter.begin(), filter.end(), filter.begin(), ::tolower);
    for (const auto &c : contactItems) {
      auto label = str(c, "name") + "  <" + str(c, "email") + ">  " +
                   str(c, "organization");
      auto lower = label;
      std::transform(lower.begin(), lower.end(), lower.begin(), ::tolower);
      if (lower.find(filter) != std::string::npos)
        contactsList->add(label.c_str(), new std::string(str(c, "email")));
    }
  }
  void contactDialog() {
    if (accounts.empty())
      return;
    auto win = dialog(600, 240, _("New Contact"));
    auto name = new Fl_Input(130, 25, 445, 28, _("Name"));
    auto email = new Fl_Input(130, 70, 445, 28, _("Email Address"));
    auto account = new Fl_Choice(130, 115, 445, 28, _("Account"));
    for (const auto &a : accounts)
      addChoice(account, str(a["account"], "email"));
    account->value(0);
    new CommandButton(
        400, 180, 175, 32, _("Save Contact"),
        [this, win, name, email, account] {
          send({{"op", "contact-save"},
                {"account", accounts[account->value()]["account"]["id"]},
                {"name", name->value()},
                {"email", email->value()}},
               [this, win](const Json &) {
                 win->hide();
                 reloadContacts();
               });
        });
    showDialog(win);
  }
  void updateTitle() {
    std::string title = "Penguin Mail";
    if (isDemo)
      title += " (Demo)";
    if (shownSpace == 1)
      title += " - " + std::string(_("Calendar")) + " - " +
               dateText(month->selected, "%Y-%m-%d");
    if (shownSpace == 2)
      title += " - " + std::string(_("Contacts"));
    title += " (" + std::to_string(std::max(0, unread)) + ")";
    copy_label(title.c_str());
    if (tray) {
      tray->setTitle(title);
      tray->setUnread(std::max(0, unread));
    }
  }
  void changeSpace(int value) {
    shownSpace = value;
    updateTitle();
    space->value(value);
    mailPage->hide();
    calendarPage->hide();
    contactsPage->hide();
    (value == 1 ? calendarPage : value == 2 ? contactsPage : mailPage)->show();
    redraw();
    reload();
  }
  void setAvailability() {
    bool selected = !currentTarget.is_null();
    for (const char *path :
         {_("Message/Reply"), _("Message/Reply All"), _("Message/Forward"),
          _("Message/Archive"), _("Message/Move to Trash"),
          _("Message/Mark Read"), _("Message/Mark Unread"), _("Message/Flag"),
          _("Message/Unflag"), _("Message/Label…"), _("Message/Remind Me…"),
          _("File/Export Conversation…")})
      if (auto item = const_cast<Fl_Menu_Item *>(menu->find_item(path))) {
        if (selected)
          item->activate();
        else
          item->deactivate();
      }
    if (tray) {
      if (hideRequested && !hideHandled && tray->available()) {
        hideHandled = true;
        hide();
        send({{"op", "visibility"}, {"open", false}});
      }
      if (!shown() && !tray->available() && !closing) {
        show();
        send({{"op", "visibility"}, {"open", true}});
      }
    }
  }
  static void tick(void *p) {
    auto s = static_cast<MailWindow *>(p);
    if (s->closing)
      return;
    s->setAvailability();
    if (s->needRefresh && s->ready && !s->refreshPending)
      s->reload();
    Fl::repeat_timeout(.25, tick, p);
  }
  static void refreshTick(void *p) {
    auto s = static_cast<MailWindow *>(p);
    if (s->closing)
      return;
    s->reload();
    Fl::repeat_timeout(15, refreshTick, p);
  }

public:
  bool pendingCompose = false;
  std::string pendingRecipient;
  bool isDemo;
  int exitCode = 0;
  std::string smokeOutput;
  MailWindow(const std::string &binary, bool demo, bool trayOnly,
             const std::string &date, const std::string &smoke)
      : dx::GpuWindow(1280, 820, "Penguin Mail"), smokePath(smoke),
        hideRequested(trayOnly), isDemo(demo) {
    xclass("io.github.c9dev.PenguinMail");
    size_range(1000, 650);
    if (!isDemo)
      try {
        std::ifstream f(prefsPath());
        if (f) {
          auto j = Json::parse(f);
          notifications = j.value("notifications", true);
          eventReminders = j.value("event_reminders", true);
          hideToTray = j.value("hide_to_tray", true);
          threading = j.value("threading", true);
        }
      } catch (const std::exception &) {
      }
    menu = new DesktopMenuBar(0, 0, 1280, 26);
    toolbar[0] = new CommandButton(12, 38, 120, 32, _("New Message"),
                                   [this] { composer(); });
    toolbar[1] = new CommandButton(140, 38, 90, 32, _("Refresh"), [this] {
      send({{"op", "refresh"}}, [this](const Json &) { reload(); });
    });
    toolbar[2] = new CommandButton(240, 38, 85, 32, _("Archive"),
                                   [this] { action("archive"); });
    toolbar[3] = new CommandButton(335, 38, 85, 32, _("Trash"),
                                   [this] { action("trash"); });
    accountChoice = new Fl_Choice(440, 38, 280, 32);
    accountChoice->tooltip(_("Account"));
    accountChoice->add(_("All Accounts"));
    accountChoice->value(0);
    space = new Fl_Choice(735, 38, 150, 32);
    space->tooltip(_("View"));
    space->add(_("Mail"));
    space->add(_("Calendar"));
    space->add(_("Contacts"));
    space->value(0);
    search = new Fl_Input(975, 38, 290, 32, _("Search"));
    search->when(FL_WHEN_ENTER_KEY_ALWAYS);
    mailPage = new Fl_Group(0, 84, 1280, 704);
    tile = new Fl_Tile(0, 84, 1280, 704);
    sidebar = new Fl_Hold_Browser(0, 84, 190, 704);
    sidebar->format_char(0);
    sidebar->tooltip(_("Mailboxes"));
    list = new MessageList(190, 84, 425, 704);
    list->tooltip(_("Conversations"));
    auto reading = new ReaderPane(615, 84, 665, 704);
    reading->actions[0] = new CommandButton(627, 94, 95, 30, _("Reply"),
                      [this] { composer("reply"); });
    reading->actions[1] = new CommandButton(732, 94, 110, 30, _("Reply All"),
                      [this] { composer("reply-all"); });
    reading->actions[2] = new CommandButton(852, 94, 105, 30, _("Forward"),
                      [this] { composer("forward"); });
    for (auto button : reading->actions)
      button->align(FL_ALIGN_CENTER | FL_ALIGN_INSIDE | FL_ALIGN_WRAP | FL_ALIGN_CLIP);
    reader = new Fl_Text_Display(627, 138, 641, 540);
    reading->reader = reader;
    reader->buffer(&readerBuffer);
    reader->wrap_mode(Fl_Text_Display::WRAP_AT_BOUNDS, 0);
    reader->tooltip(_("Conversation"));
    attachments = new Fl_Hold_Browser(627, 704, 480, 72, _("Attachments"));
    attachments->align(FL_ALIGN_TOP_LEFT);
    attachments->format_char(0);
    saveAttach = new CommandButton(1118, 704, 150, 30, _("Save Attachment"),
                                   [this] { saveAttachment(); });
    reading->resizable(reader);
    reading->end();
    tile->resizable(reading);
    tile->end();
    mailPage->resizable(tile);
    mailPage->end();
    calendarPage = new Fl_Group(0, 84, 1280, 704);
    new CommandButton(15, 95, 70, 30, "<", [this] {
      month->move(-1);
      reloadCalendar();
    });
    new CommandButton(95, 95, 85, 30, _("Today"), [this] {
      month->set(nowMs());
      reloadCalendar();
    });
    new CommandButton(190, 95, 70, 30, ">", [this] {
      month->move(1);
      reloadCalendar();
    });
    monthLabel = new Fl_Box(275, 95, 400, 30);
    new CommandButton(1000, 95, 120, 30, _("New Event"),
                      [this] { eventDialog(false); });
    new CommandButton(1130, 95, 130, 30, _("Edit Event"),
                      [this] { eventDialog(true); });
    month = new MonthGrid(15, 140, 875, 630);
    month->tooltip(_("Calendar days"));
    agenda = new Fl_Hold_Browser(910, 150, 350, 570, _("Agenda"));
    agenda->align(FL_ALIGN_TOP_LEFT);
    agenda->format_char(0);
    new CommandButton(1100, 735, 160, 30, _("Delete Event"),
                      [this] { deleteEvent(); });
    calendarPage->resizable(month);
    calendarPage->end();
    calendarPage->hide();
    contactsPage = new Fl_Group(0, 84, 1280, 704);
    contactSearch = new Fl_Input(140, 105, 600, 30, _("Find Contacts"));
    contactSearch->when(FL_WHEN_CHANGED);
    new CommandButton(760, 105, 180, 30, _("New Contact"),
                      [this] { contactDialog(); });
    new CommandButton(
        955, 105, 305, 30, _("Write to Selected Contact"), [this] {
          int n = contactsList->value();
          if (n && contactsList->data(n))
            composer("new", *static_cast<std::string *>(contactsList->data(n)));
        });
    contactsList = new Fl_Hold_Browser(15, 155, 1245, 615);
    contactsList->format_char(0);
    contactsList->tooltip(_("Contacts"));
    contactsPage->resizable(contactsList);
    contactsPage->end();
    contactsPage->hide();
    status = new Fl_Box(12, 792, 1256, 24, _("Starting Penguin Mail…"));
    status->align(FL_ALIGN_LEFT | FL_ALIGN_INSIDE | FL_ALIGN_CLIP);
    addMenu(*menu, _("File/New Message"), FL_CTRL | 'n',
            [this] { composer(); });
    addMenu(*menu, _("File/Add Account…"), 0, [this] { accountDialog(); });
#ifdef PENGUIN_ENABLE_MISFIN
    addMenu(*menu, _("File/Misfin Messaging…"), 0, [this] { misfinDialog(); });
#endif
    addMenu(*menu, _("File/Export Conversation…"), FL_CTRL | FL_SHIFT | 's',
            [this] { exportThread(); });
    addMenu(*menu, _("File/OpenPGP Keys…"), 0, [this] { openPgp(); });
    addMenu(*menu, _("File/Preferences…"), FL_CTRL | ',',
            [this] { preferences(); });
    addMenu(*menu, _("File/Show Window"), 0, [this] { activate(); });
    addMenu(*menu, _("File/Close Window"), FL_CTRL | 'w',
            [this] { closeWindow(); });
    addMenu(*menu, _("File/Quit"), FL_CTRL | 'q', [this] { quit(); });
    addMenu(*menu, _("Edit/Undo"), FL_CTRL | 'z', [this] { edit('z'); });
    addMenu(*menu, _("Edit/Cut"), FL_CTRL | 'x', [this] { edit('x'); });
    addMenu(*menu, _("Edit/Copy"), FL_CTRL | 'c', [this] { edit('c'); });
    addMenu(*menu, _("Edit/Paste"), FL_CTRL | 'v', [this] { edit('v'); });
    addMenu(*menu, _("Edit/Select All"), FL_CTRL | 'a', [this] { edit('a'); });
    addMenu(*menu, _("Edit/Search"), FL_CTRL | 'f',
            [this] { search->take_focus(); });
    addMenu(*menu, _("View/Mail"), FL_CTRL | '1', [this] { changeSpace(0); });
    addMenu(*menu, _("View/Calendar"), FL_CTRL | '2',
            [this] { changeSpace(1); });
    addMenu(*menu, _("View/Contacts"), FL_CTRL | '3',
            [this] { changeSpace(2); });
    addMenu(*menu, _("View/Refresh"), FL_F + 5, [this] {
      send({{"op", "refresh"}}, [this](const Json &) { reload(); });
    });
    addMenu(*menu, _("View/Load More"), 0, [this] {
      loadLimit = std::min(2000, loadLimit + 500);
      ++listGeneration;
      reload();
    });
    addMenu(*menu, _("Message/Edit Draft"), 0, [this] { editDraft(); });
    addMenu(*menu, _("Message/Send Queued Message Now"), 0,
            [this] { queuedAction(false); });
    addMenu(*menu, _("Message/Delete Queued Message"), 0,
            [this] { queuedAction(true); });
    addMenu(*menu, _("Message/Reply"), FL_CTRL | 'r',
            [this] { composer("reply"); });
    addMenu(*menu, _("Message/Reply All"), FL_CTRL | FL_SHIFT | 'r',
            [this] { composer("reply-all"); });
    addMenu(*menu, _("Message/Forward"), FL_CTRL | 'l',
            [this] { composer("forward"); });
    addMenu(*menu, _("Message/Archive"), FL_CTRL | 'e',
            [this] { action("archive"); });
    addMenu(*menu, _("Message/Move to Trash"), FL_Delete,
            [this] { action("trash"); });
    addMenu(*menu, _("Message/Mark Read"), FL_CTRL | 'k',
            [this] { action("read"); });
    addMenu(*menu, _("Message/Mark Unread"), FL_CTRL | FL_SHIFT | 'k',
            [this] { action("unread"); });
    addMenu(*menu, _("Message/Flag"), 0, [this] { action("star"); });
    addMenu(*menu, _("Message/Unflag"), 0, [this] { action("unstar"); });
    addMenu(*menu, _("Message/Label…"), 0, [this] { action("label"); });
    addMenu(*menu, _("Message/Remind Me…"), 0, [this] { action("remind"); });
    addMenu(*menu, _("Help/Keyboard Shortcuts"), FL_F + 1, [] {
      fl_message("%s",
                 _("Ctrl+N: New Message\nCtrl+R: Reply\nCtrl+Shift+R: Reply "
                   "All\nCtrl+L: Forward\nCtrl+E: Archive\nDelete: Move to "
                   "Trash\nCtrl+Z: Undo\nCtrl+F: Search\nCtrl+1/2/3: Mail / "
                   "Calendar / Contacts\nF5: Refresh\nCtrl+W: Close "
                   "Window\nCtrl+Q: Quit\n\nComposer: Ctrl+Enter sends, Ctrl+S "
                   "saves a draft.\nDomeD owns Super shortcuts."));
    });
    addMenu(*menu, _("Help/About Penguin Mail"), 0, [] {
      fl_message(
          "%s",
          _("Penguin Mail 1.0.10\nFLTK edition for DomeD\n\nOriginal mail "
            "engines: c9dev/penguin-mail\nLicense: GPL-3.0-or-later"));
    });
    bind(accountChoice, [this] {
      int n = accountChoice->value();
      selectedAccount = n > 0 ? accounts[n - 1]["account"].value("id", 0) : 0;
      rebuildBoxes();
      ++listGeneration;
      reload();
    });
    bind(space, [this] { changeSpace(space->value()); });
    bind(search, [this] {
      std::string query = search->value();
      currentBox = query.empty()
                       ? Json{{"mailbox", "inbox"}}
                       : Json{{"mailbox", "search"}, {"query", query}};
      loadLimit = 500;
      ++listGeneration;
      reload();
    });
    bind(sidebar, [this] {
      int n = sidebar->value() - 1;
      if (n < 0 || n >= int(boxes.size()))
        return;
      currentBox = boxes[n];
      search->value("");
      loadLimit = 500;
      ++listGeneration;
      reload();
    });
    bind(contactSearch, [this] { filterContacts(); });
    list->chosen = [this](int row) { open(row); };
    month->chosen = [this](int64_t date) {
      if (dateText(date, "%Y-%m") != dateText(month->first, "%Y-%m")) {
        month->set(date);
        reloadCalendar();
      } else
        updateAgenda();
    };
    end();
    resizable(mailPage);
    callback([](Fl_Widget *w, void *) {
      static_cast<MailWindow *>(w)->closeWindow();
    });
    menu->layoutChanged = [this] {
      resize(x(), y(), w(), h());
      init_sizes();
    };
    desktop = std::make_unique<DesktopMenu>(*this, *menu);
    tray = std::make_unique<StatusIcon>(*desktop);
    tray->activate = [this] {
      if (shown())
        closeWindow();
      else
        activate();
    };
    dx::style(this);
    show();
    desktopNotifications = std::make_unique<DesktopNotifications>(
      [this](const Json &n, const std::string &action) { notificationChosen(n, action); },
      [this](const Json &n) {
        if (str(n,"kind") == "calendar")
          send({{"op","notification-delivered"},{"key",n["key"]}});
      });
    engine = std::make_unique<MailEngine>(binary, demo);
    engine->event = [this](const Json &e) {
      auto type = str(e, "event");
      if (type == "ready") {
        ready = true;
        notificationSettings();
        reload();
      } else if (type == "changed") {
        needRefresh = true;
      } else if (type == "notification-withdraw") {
        desktopNotifications->withdraw(str(e,"key"));
      } else if (type == "notification") {
        if (!isDemo && (str(e,"kind") == "calendar" ? eventReminders : notifications))
          desktopNotifications->post(e);
        needRefresh = true;
      } else if (type == "authorize") {
        auto url = str(e, "url");
        if (url.rfind("https://", 0) == 0)
          dx::spawn({"xdg-open", url});
      } else if (type == "account-error" || type == "warning" ||
                 type == "fatal") {
        accountError = str(e, "error");
        say(accountError);
        if (type == "fatal")
          ready = false;
      }
    };
    if (!date.empty()) {
      auto value = parseDate(date, "%Y-%m-%d");
      if (value != InvalidDate) {
        month->set(value);
        shownSpace = 1;
        space->value(1);
        mailPage->hide();
        calendarPage->show();
      }
    }
    Fl::add_timeout(.25, tick, this);
    Fl::add_timeout(15, refreshTick, this);
  }
  ~MailWindow() {
    Fl::remove_timeout(tick, this);
    Fl::remove_timeout(refreshTick, this);
    desktopNotifications.reset();
    engine.reset();
    dockBadge(0);
    for (int i = 1; i <= contactsList->size(); ++i)
      delete static_cast<std::string *>(contactsList->data(i));
    for (auto &w : dialogs)
      w->hide();
    dialogMenus.clear();
    dialogs.clear();
    reader->buffer(nullptr);
  }
  bool running() const { return !closing; }
  void activate() {
    show();
    take_focus();
    if (ready)
      send({{"op", "visibility"}, {"open", true}});
  }
  void openCalendar(const std::string &date) {
    auto value = parseDate(date, "%Y-%m-%d");
    if (value == InvalidDate) {
      say(_("Invalid calendar date. Use YYYY-MM-DD."));
      return;
    }
    month->set(value);
    activate();
    changeSpace(1);
  }
  void composeTo(const std::string &to) {
    activate();
    if (!ready || accountsPending || accounts.empty()) {
      pendingCompose = true;
      pendingRecipient = to;
      return;
    }
    composer("new", to);
  }
  void closeWindow() {
    if (hideToTray && tray && tray->available()) {
      hide();
      if (ready)
        send({{"op", "visibility"}, {"open", false}});
    } else
      quit();
  }
  void quit() {
    if (!smokeOutput.empty()) {
      closing = true;
      hide();
      return;
    }
    for (auto &w : dialogs)
      if (w->shown() &&
          std::string(w->label() ? w->label() : "") == _("Compose Message")) {
        w->do_callback();
        if (w->shown())
          return;
      }
    closing = true;
    for (auto &w : dialogs)
      w->hide();
    hide();
  }
  void resize(int x, int y, int w, int h) override {
    dx::GpuWindow::resize(x, y, w, h);
    if (!menu)
      return;
    const int top = menu->universal() ? 0 : 26;
    menu->resize(0, 0, w, top);
    mailPage->resize(0, top + 58, w, h - top - 90);
    if (w < 1180) {
      const int xs[] = {12, 126, 214, 302}, ws[] = {104, 78, 78, 70};
      for (int i = 0; i < 4; ++i)
        toolbar[i]->resize(xs[i], top + 12, ws[i], 32);
      accountChoice->resize(384, top + 12, 190, 32);
      space->resize(586, top + 12, 115, 32);
      search->resize(w - 240, top + 12, 225, 32);
    } else {
      const int xs[] = {12, 140, 240, 335}, ws[] = {120, 90, 85, 85};
      for (int i = 0; i < 4; ++i)
        toolbar[i]->resize(xs[i], top + 12, ws[i], 32);
      accountChoice->resize(440, top + 12, 280, 32);
      space->resize(735, top + 12, 150, 32);
      search->resize(w - 305, top + 12, 290, 32);
    }
    if (saveAttach) {
      saveAttach->resize(w - 162, h - 116, 150, 30);
      attachments->resize(reader->x(), h - 116,
                          std::max(80, w - 174 - reader->x()), 72);
    }
    status->resize(12, h - 28, w - 24, 24);
    calendarPage->resize(0, top + 58, w, h - top - 90);
    contactsPage->resize(0, top + 58, w, h - top - 90);
  }
};
class DesktopActivation {
  GDBusConnection *bus = nullptr;
  GDBusNodeInfo *info = nullptr;
  guint object = 0;
  std::string name;
  static void call(GDBusConnection *, const gchar *, const gchar *,
                   const gchar *, const gchar *method, GVariant *params,
                   GDBusMethodInvocation *invocation, gpointer data) {
    auto self = static_cast<DesktopActivation *>(data);
    if (self->window) {
      if (!strcmp(method, "OpenCalendar")) {
        const char *date;
        g_variant_get(params, "(&s)", &date);
        self->window->openCalendar(date);
      } else if (!strcmp(method, "Compose")) {
        const char *to;
        g_variant_get(params, "(&s)", &to);
        self->window->composeTo(to);
      } else
        self->window->activate();
    }
    g_dbus_method_invocation_return_value(invocation, nullptr);
  }

public:
  MailWindow *window = nullptr;
  bool primary = true;
  explicit DesktopActivation(bool demo)
      : name(demo ? "io.github.c9dev.PenguinMail.FltkDemo"
                  : "io.github.c9dev.PenguinMail.Fltk") {
    bus = g_bus_get_sync(G_BUS_TYPE_SESSION, nullptr, nullptr);
    if (!bus)
      return;
    auto reply = g_dbus_connection_call_sync(
        bus, "org.freedesktop.DBus", "/org/freedesktop/DBus",
        "org.freedesktop.DBus", "RequestName",
        g_variant_new("(su)", name.c_str(), 4u), G_VARIANT_TYPE("(u)"),
        G_DBUS_CALL_FLAGS_NONE, 2000, nullptr, nullptr);
    if (!reply)
      return;
    guint result;
    g_variant_get(reply, "(u)", &result);
    g_variant_unref(reply);
    primary = result == 1 || result == 4;
    if (!primary)
      return;
    info = g_dbus_node_info_new_for_xml(
        "<node><interface name='io.github.c9dev.PenguinMail.Desktop'><method "
        "name='Activate'/><method name='OpenCalendar'><arg name='date' "
        "type='s' direction='in'/></method><method name='Compose'><arg "
        "name='to' type='s' direction='in'/></method></interface></node>",
        nullptr);
    static const GDBusInterfaceVTable vtable = {
        call, nullptr, nullptr, {nullptr}};
    object = g_dbus_connection_register_object(
        bus, "/io/github/c9dev/PenguinMail", info->interfaces[0], &vtable, this,
        nullptr, nullptr);
  }
  bool forward(const std::string &date, const std::string &to) {
    if (!bus)
      return false;
    GError *error = nullptr;
    auto reply = g_dbus_connection_call_sync(
        bus, name.c_str(), "/io/github/c9dev/PenguinMail",
        "io.github.c9dev.PenguinMail.Desktop",
        !date.empty() ? "OpenCalendar"
        : !to.empty() ? "Compose"
                      : "Activate",
        !date.empty() ? g_variant_new("(s)", date.c_str())
        : !to.empty() ? g_variant_new("(s)", to.c_str())
                      : nullptr,
        nullptr, G_DBUS_CALL_FLAGS_NONE, 3000, nullptr, &error);
    if (error) {
      fprintf(stderr, "%s\n", error->message);
      g_error_free(error);
    }
    if (reply) {
      g_variant_unref(reply);
      return true;
    }
    return false;
  }
  ~DesktopActivation() {
    if (bus) {
      if (object)
        g_dbus_connection_unregister_object(bus, object);
      if (primary)
        g_dbus_connection_call_sync(
            bus, "org.freedesktop.DBus", "/org/freedesktop/DBus",
            "org.freedesktop.DBus", "ReleaseName",
            g_variant_new("(s)", name.c_str()), nullptr, G_DBUS_CALL_FLAGS_NONE,
            1000, nullptr, nullptr);
      g_object_unref(bus);
    }
    if (info)
      g_dbus_node_info_unref(info);
  }
};
int main(int argc, char **argv) {
  setlocale(LC_ALL, "");
  auto executable = std::filesystem::canonical("/proc/self/exe");
  auto locale = executable.parent_path() / "../share/locale";
  auto buildLocale = executable.parent_path() / "../target/locale";
  if (std::filesystem::is_directory(buildLocale))
    locale = buildLocale;
  if (const char *override = getenv("PENGUIN_MAIL_LOCALEDIR"))
    locale = override;
  setenv("PENGUIN_MAIL_LOCALEDIR", locale.c_str(), 1);
  bindtextdomain("penguin-mail", locale.c_str());
  bind_textdomain_codeset("penguin-mail", "UTF-8");
  textdomain("penguin-mail");
  signal(SIGPIPE, SIG_IGN);
  bool demo = false, tray = false, compose = false;
  std::string date, smoke, to;
  for (int i = 1; i < argc; ++i) {
    std::string arg = argv[i];
    if (arg == "--version") {
      puts("Penguin Mail 1.0.10 (FLTK 1.4.5, DomeD)");
      return 0;
    }
    if (arg == "--help") {
      puts("Usage: penguin-mail [--demo] [--tray] [--calendar YYYY-MM-DD] "
           "[--compose ADDRESS] [mailto:ADDRESS]\n       penguin-mail "
           "--smoke-test OUTPUT.ppm --demo");
      return 0;
    }
    if (arg == "--demo")
      demo = true;
    else if (arg == "--tray")
      tray = true;
    else if (arg == "--calendar-today")
      date = dateText(nowMs(), "%Y-%m-%d");
    else if (arg == "--calendar" && i + 1 < argc)
      date = argv[++i];
    else if (arg == "--smoke-test" && i + 1 < argc)
      smoke = argv[++i];
    else if (arg == "--compose") {
      compose = true;
      if (i + 1 < argc && std::string(argv[i + 1]).rfind("--", 0) != 0)
        to = argv[++i];
    } else if (arg.rfind("mailto:", 0) == 0) {
      compose = true;
      auto raw = arg.substr(7);
      auto decoded =
          g_uri_unescape_string(raw.substr(0, raw.find('?')).c_str(), nullptr);
      if (decoded) {
        to = decoded;
        g_free(decoded);
      }
    } else {
      fprintf(stderr, "Unknown or incomplete option: %s\n", arg.c_str());
      return 2;
    }
  }
  if (!date.empty() && parseDate(date, "%Y-%m-%d") == InvalidDate) {
    fprintf(stderr, "Invalid calendar date: %s\n", date.c_str());
    return 2;
  }
  DesktopActivation activation(demo);
  if (!activation.primary)
    return activation.forward(date, compose && to.empty() ? " " : to) ? 0 : 1;
  dx::init();
  auto self = std::filesystem::canonical("/proc/self/exe");
  auto binary = self.parent_path() / "penguin-mail-engine";
  if (const char *override = getenv("PENGUIN_MAIL_ENGINE"))
    binary = override;
  try {
    MailWindow window(binary.string(), demo, tray, date, smoke);
    activation.window = &window;
    if (compose)
      window.composeTo(to);
    while (window.running())
      Fl::wait(.05);
    int result = 0;
    activation.window = nullptr;
    return result ? result : window.exitCode;
  } catch (const std::exception &e) {
    fprintf(stderr, "Penguin Mail: %s\n", e.what());
    return 1;
  }
}
