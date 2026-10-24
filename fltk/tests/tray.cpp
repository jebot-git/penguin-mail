// SPDX-License-Identifier: GPL-3.0-or-later
#include "autostart.h"
#include "appearance.h"
#include "gpu-window.h"
#include "status-icon.h"
#include <iostream>
#include <thread>
#include <unistd.h>
static StatusIcon *icon;
static dx::GpuWindow *window;
static void input(int fd, void *) {
  char value[32] = {};
  auto count = read(fd, value, sizeof(value) - 1);
  if (count <= 0 || value[0] == 'q') window->hide();
  else icon->setUnread(atoi(value));
}
int main(int argc, char **argv) try {
  if (argc != 2) return 2;
  setenv("XDG_CONFIG_HOME", argv[1], 1);
  const std::string exe = "/tmp/Penguin 日本語 % \" \\ $ ` Mail";
  login_startup::apply(true, exe);
  if (!login_startup::enabled()) throw std::runtime_error("Login startup was not enabled");
  auto file = g_key_file_new();
  g_key_file_load_from_file(file, login_startup::path().c_str(), G_KEY_FILE_NONE, nullptr);
  auto exec = g_key_file_get_string(file, "Desktop Entry", "Exec", nullptr);
  if (exec != login_startup::command(exe)) throw std::runtime_error("Desktop command did not round-trip");
  g_free(exec);
  g_key_file_set_boolean(file, "Desktop Entry", "Hidden", true);
  g_key_file_save_to_file(file, login_startup::path().c_str(), nullptr);
  if (login_startup::enabled()) throw std::runtime_error("Hidden login entry was enabled");
  g_key_file_unref(file);
  login_startup::apply(false, exe);
  if (login_startup::enabled()) throw std::runtime_error("Login startup was not disabled");
  dx::init();
  dx::GpuWindow app(600, 300, "Tray test");window = &app;
  DesktopMenuBar bar(0, 0, 600, 25);
  bar.add("Slow", 0, [](Fl_Widget *, void *) {
    std::this_thread::sleep_for(std::chrono::milliseconds(500));
  });
  app.end();
  DesktopMenu menu(app, bar);
  StatusIcon tray(menu);icon = &tray;
  tray.activate = [] { icon->setUnread(1); };
  Fl::add_fd(STDIN_FILENO, FL_READ, input);
  app.show();
  std::cout << menu.serviceName() << " " << menu.objectPath() << std::endl;
  Fl::run();
  Fl::remove_fd(STDIN_FILENO);
  return 0;
} catch (const std::exception &e) { std::cerr << e.what() << '\n';return 1; }
