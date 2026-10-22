// SPDX-License-Identifier: MIT
// One 24-unit, 1.6-unit rounded-stroke vocabulary for desktop and XR chrome.
#pragma once
#include <algorithm>
#include <string>
namespace dx_icon {
inline std::string role(std::string name) {
  std::transform(name.begin(), name.end(), name.begin(), ::tolower);
  if (name.find("penguin") != std::string::npos) return "mail";
  if (name.find("folder") != std::string::npos ||
      name.find("file-manager") != std::string::npos ||
      name.find("fsv") != std::string::npos)
    return "folder";
  if (name.find("image") != std::string::npos ||
      name.find("qimg") != std::string::npos ||
      name.find("photo") != std::string::npos)
    return "image";
  if (name.find("audio") != std::string::npos ||
      name.find("music") != std::string::npos ||
      name.find("cmux") != std::string::npos ||
      name.find("multimedia-player") != std::string::npos)
    return "music";
  if (name.find("video") != std::string::npos ||
      name.find("mpvr") != std::string::npos)
    return "video";
  if (name.find("arc") != std::string::npos ||
      name.find("depth") != std::string::npos)
    return "depth";
  if (name.find("terminal") != std::string::npos ||
      name.find("ghostty") != std::string::npos)
    return "terminal";
  if (name.find("dome") != std::string::npos)
    return "dome";
  if (name.find("lightxr") != std::string::npos)
    return "light";
  if (name.find("clipboard") != std::string::npos)
    return "clipboard";
  if (name.find("notification") != std::string::npos ||
      name.find("bell") != std::string::npos)
    return "bell";
  if (name.find("muted") != std::string::npos ||
      name.find("unavailable") != std::string::npos)
    return "muted";
  if (name.find("volume") != std::string::npos ||
      name.find("speaker") != std::string::npos)
    return "speaker";
  return "app";
}
inline std::string path(const std::string &role) {
  if (role == "mail") return "M3 5h18v15H3z M3 5l9 9 9-9 M10 14h4l-2 2z M3 5l3-3h12l3 3";
  if (role == "folder")
    return "M3 19V5h7l2 3h9v11H3z M3 9h18";
  if (role == "image")
    return "M3 4h18v16H3z M4 17l6-6 4 4 3-3 4 5 M15 8h.1";
  if (role == "music")
    return "M9 17V5l11-2v12 M9 7l11-2 M9 17c0 3-6 4-6 1s6-4 6-1 M20 15c0 3-6 "
           "4-6 1s6-4 6-1";
  if (role == "video")
    return "M3 5h18v14H3z M10 8l5 4-5 4z";
  if (role == "depth")
    return "M3 6h8v12H3z M13 6h8v12h-8z M6 10l2 2-2 2 M18 10l-2 2 2 2";
  if (role == "dome")
    return "M2 18a10 10 0 0 1 20 0 M2 18h20 M12 8v10 M5 11l14 7 M19 11L5 18";
  if (role == "light")
    return "M4 19V5h16v14H4z M12 9v5 M12 16h.01 M7 3v4 M17 3v4";
  if (role == "clipboard")
    return "M8 5H5v16h14V5h-3 M8 3h8v4H8z M8 11h8 M8 15h8";
  if (role == "bell")
    return "M5 17h14l-2-4V9a5 5 0 0 0-10 0v4l-2 4z M10 20h4 M12 2v2";
  if (role == "speaker")
    return "M3 10h4l5-5v14l-5-5H3z M16 8c3 2 3 6 0 8 M19 5c5 4 5 10 0 14";
  if (role == "muted")
    return "M3 10h4l5-5v14l-5-5H3z M16 9l5 6 M21 9l-5 6";
  if (role == "terminal")
    return "M3 4h18v16H3z M6 8l4 4-4 4 M13 16h5";
  if (role == "app")
    return "M3 4h18v16H3z M3 8h18 M7 6h.1 M10 6h.1";
  return "M3 4h18v16H3z M3 8h18 M7 6h.1 M10 6h.1";
}
inline std::string svg(const std::string &name, const std::string &color) {
  return "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"24\" height=\"24\" "
         "viewBox=\"0 0 24 24\"><path fill=\"none\" stroke=\"" +
         color +
         "\" stroke-width=\"1.6\" stroke-linecap=\"round\" "
         "stroke-linejoin=\"round\" d=\"" +
         path(name) + "\"/></svg>";
}
} // namespace dx_icon
