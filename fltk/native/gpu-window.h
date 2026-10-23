// SPDX-License-Identifier: MIT
#pragma once
#include "appearance.h"
#include "suite-icons.h"
#include <FL/Fl_Gl_Window.H>
#include <FL/Fl_SVG_Image.H>
#include <FL/gl.h>
#include <FL/gl_draw.H>
namespace dx {
// FLTK's native GL widget driver draws shapes on the GPU and caches text as
// textures. Its text cache rasterizes through the patched native bitmap path.
// Wayland EGL submits DMA-BUF directly; CPU readback is confined to QA
// captures.
class GpuWindow : public Fl_Gl_Window {
  std::string iconTint;
  void refreshIcon() {
    unsigned char r, g, b;
    Fl::get_color(FL_FOREGROUND_COLOR, r, g, b);
    char color[8];
    snprintf(color, sizeof(color), "#%02x%02x%02x", r, g, b);
    if (iconTint == color)
      return;
    iconTint = color;
    auto svg = dx_icon::svg(dx_icon::role(label() ? label() : "app"), iconTint);
    Fl_SVG_Image image(nullptr, svg.c_str());
    image.resize(48, 48);
    image.normalize();
    if (!image.fail())
      icon(&image);
  }

public:
  GpuWindow(int W, int H, const char *label = nullptr)
      : Fl_Gl_Window(W, H, label) {
    mode(FL_RGB | FL_ALPHA | FL_DOUBLE);
    begin();
    Fl::draw_GL_text_with_textures(1);
    gl_texture_pile_height(512);
  }
  GpuWindow(int X, int Y, int W, int H, const char *label = nullptr)
      : Fl_Gl_Window(X, Y, W, H, label) {
    mode(FL_RGB | FL_ALPHA | FL_DOUBLE);
    begin();
    Fl::draw_GL_text_with_textures(1);
    gl_texture_pile_height(512);
  }
  void draw() override {
    // A hidden window can lose its last shared GL context. Cached string
    // texture names then refer to destroyed objects after tray restoration.
    if (!context_valid()) gl_texture_reset();
    refreshIcon();
    static bool reported = false;
    if (!reported && getenv("DE_XR_GL_TRACE")) {
      fprintf(stderr, "FLTK GPU widgets: %s\n",
              (const char *)glGetString(GL_RENDERER));
      reported = true;
    }
    static std::string fontRevision;
    auto revision = std::to_string(dx::font_revision);
    if (fontRevision != revision) {
      gl_texture_reset();
      gl_texture_pile_height(512);
      fontRevision = revision;
    }
    glViewport(0, 0, pixel_w(), pixel_h());
    glDisable(GL_SCISSOR_TEST);
    glDisable(GL_DEPTH_TEST);
    unsigned char r, g, b;
    Fl::get_color(FL_BACKGROUND_COLOR, r, g, b);
    glClearColor(r / 255.f, g / 255.f, b / 255.f, 1);
    glClear(GL_COLOR_BUFFER_BIT);
    Fl_Gl_Window::draw();
  }
};
} // namespace dx
