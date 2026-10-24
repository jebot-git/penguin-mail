// SPDX-License-Identifier: GPL-3.0-or-later
#pragma once
#include <gio/gio.h>
#include <filesystem>
#include <stdexcept>
#include <string>
namespace login_startup {
inline std::filesystem::path path() {
  const char *config = getenv("XDG_CONFIG_HOME"), *home = getenv("HOME");
  if (config && *config) return std::filesystem::path(config) / "autostart/io.github.c9dev.PenguinMail.desktop";
  if (home && *home) return std::filesystem::path(home) / ".config/autostart/io.github.c9dev.PenguinMail.desktop";
  throw std::runtime_error("No configuration directory");
}
inline bool enabled() {
  auto file = g_key_file_new();
  bool result = g_key_file_load_from_file(file, path().c_str(), G_KEY_FILE_NONE, nullptr) &&
      !g_key_file_get_boolean(file, "Desktop Entry", "Hidden", nullptr);
  if (g_key_file_has_key(file, "Desktop Entry", "X-GNOME-Autostart-enabled", nullptr))
    result = result && g_key_file_get_boolean(file, "Desktop Entry", "X-GNOME-Autostart-enabled", nullptr);
  g_key_file_unref(file);
  return result;
}
inline std::string command(const std::string &exe) {
  std::string result = "\"";
  for (char c : exe) {
    if (c == '%') result += '%';
    if (c == '\\' || c == '\"' || c == '$' || c == '`') result += '\\';
    result += c;
  }
  return result + "\" --tray";
}
inline void apply(bool start, const std::string &exe = std::filesystem::canonical("/proc/self/exe").string()) {
  auto target = path();
  std::filesystem::create_directories(target.parent_path());
  auto file = g_key_file_new();
  g_key_file_load_from_file(file, target.c_str(), G_KEY_FILE_KEEP_COMMENTS, nullptr);
  g_key_file_set_string(file, "Desktop Entry", "Type", "Application");
  g_key_file_set_string(file, "Desktop Entry", "Name", "Penguin Mail");
  g_key_file_set_string(file, "Desktop Entry", "Exec", command(exe).c_str());
  g_key_file_set_string(file, "Desktop Entry", "TryExec", exe.c_str());
  g_key_file_set_boolean(file, "Desktop Entry", "Terminal", false);
  g_key_file_set_boolean(file, "Desktop Entry", "Hidden", !start);
  g_key_file_set_boolean(file, "Desktop Entry", "X-GNOME-Autostart-enabled", start);
  gsize length = 0;
  auto text = g_key_file_to_data(file, &length, nullptr);
  g_key_file_unref(file);
  GError *error = nullptr;
  bool ok = g_file_set_contents_full(target.c_str(), text, length,
      static_cast<GFileSetContentsFlags>(G_FILE_SET_CONTENTS_CONSISTENT | G_FILE_SET_CONTENTS_DURABLE), 0600, &error);
  g_free(text);
  if (!ok) {
    std::string message = error ? error->message : "Cannot save login startup";
    g_clear_error(&error);
    throw std::runtime_error(message);
  }
}
} // namespace login_startup
