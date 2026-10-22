// SPDX-License-Identifier: GPL-3.0-or-later
#pragma once
#include "appearance.h"
#include <cerrno>
#include <fcntl.h>
#include <functional>
#include <libintl.h>
#include <map>
#include <signal.h>
#include <sys/wait.h>
// The pipe endpoints are inherited only by this process and its child.
class MailEngine {
  int input = -1, output = -1;
  pid_t child = -1;
  int nextId = 1;
  std::string received, outgoing;
  std::map<int, std::function<void(const Json &)>> pending;
  static void readable(int, void *p) { static_cast<MailEngine *>(p)->read(); }
  static void writable(int, void *p) { static_cast<MailEngine *>(p)->write(); }
  void fail(const std::string &reason) {
    if (input >= 0) {
      Fl::remove_fd(input);
      close(input);
      input = -1;
    }
    if (output >= 0) {
      Fl::remove_fd(output);
      close(output);
      output = -1;
    }
    auto callbacks = std::move(pending);
    for (auto &item : callbacks)
      item.second(Json{{"ok", false}, {"error", reason}});
    if (event)
      event(Json{{"event", "fatal"}, {"error", reason}});
  }
  void write() {
    while (output >= 0 && !outgoing.empty()) {
      auto n = ::write(output, outgoing.data(), outgoing.size());
      if (n > 0)
        outgoing.erase(0, n);
      else if (n < 0 && errno == EINTR)
        continue;
      else if (n < 0 && (errno == EAGAIN || errno == EWOULDBLOCK))
        break;
      else {
        fail(gettext("Mail engine disconnected"));
        return;
      }
    }
    if (output >= 0 && outgoing.empty())
      Fl::remove_fd(output, FL_WRITE);
  }
  void read() {
    char bytes[16384];
    while (input >= 0) {
      auto n = ::read(input, bytes, sizeof(bytes));
      if (n > 0) {
        received.append(bytes, n);
        if (received.size() > 64 * 1024 * 1024) {
          fail(gettext("Mail engine response exceeds limit"));
          return;
        }
      } else if (n < 0 && errno == EINTR)
        continue;
      else if (n < 0 && (errno == EAGAIN || errno == EWOULDBLOCK))
        break;
      else {
        fail(gettext("Mail engine stopped"));
        return;
      }
    }
    size_t newline;
    while ((newline = received.find('\n')) != std::string::npos) {
      auto line = received.substr(0, newline);
      received.erase(0, newline + 1);
      try {
        auto reply = Json::parse(line);
        if (reply.contains("event")) {
          if (event)
            event(reply);
        } else if (reply.contains("id") && reply["id"].is_number_integer()) {
          int id = reply["id"].get<int>();
          auto found = pending.find(id);
          if (found != pending.end()) {
            auto callback = std::move(found->second);
            pending.erase(found);
            callback(reply);
          }
        }
      } catch (const Json::exception &e) {
        fail(std::string("Invalid engine reply: ") + e.what());
        return;
      }
    }
  }

public:
  std::function<void(const Json &)> event;
  MailEngine(const std::string &binary, bool demo) {
    int to[2], from[2];
    if (pipe2(to, O_CLOEXEC))
      throw std::runtime_error(gettext("Cannot create engine pipe"));
    if (pipe2(from, O_CLOEXEC)) {
      close(to[0]);
      close(to[1]);
      throw std::runtime_error(gettext("Cannot create engine pipe"));
    }
    posix_spawn_file_actions_t actions;
    posix_spawn_file_actions_init(&actions);
    posix_spawn_file_actions_adddup2(&actions, to[0], STDIN_FILENO);
    posix_spawn_file_actions_adddup2(&actions, from[1], STDOUT_FILENO);
    std::vector<std::string> args{binary};
    if (demo)
      args.emplace_back("--demo");
    std::vector<char *> argv;
    for (auto &s : args)
      argv.push_back(s.data());
    argv.push_back(nullptr);
    posix_spawnattr_t attributes;
    posix_spawnattr_init(&attributes);
    sigset_t mask, defaults;
    sigemptyset(&mask); sigemptyset(&defaults); sigaddset(&defaults, SIGTERM);
    posix_spawnattr_setsigmask(&attributes, &mask);
    posix_spawnattr_setsigdefault(&attributes, &defaults);
    posix_spawnattr_setflags(&attributes, POSIX_SPAWN_SETSIGMASK | POSIX_SPAWN_SETSIGDEF);
    int error = posix_spawn(&child, binary.c_str(), &actions, &attributes,
                            argv.data(), environ);
    posix_spawnattr_destroy(&attributes);
    posix_spawn_file_actions_destroy(&actions);
    close(to[0]);
    close(from[1]);
    if (error) {
      close(to[1]);
      close(from[0]);
      throw std::runtime_error(std::string("Cannot launch mail engine: ") +
                               strerror(error));
    }
    input = from[0];
    output = to[1];
    fcntl(input, F_SETFL, fcntl(input, F_GETFL) | O_NONBLOCK);
    fcntl(output, F_SETFL, fcntl(output, F_GETFL) | O_NONBLOCK);
    Fl::add_fd(input, FL_READ, readable, this);
  }
  ~MailEngine() {
    if (input >= 0) {
      Fl::remove_fd(input);
      close(input);
    }
    if (output >= 0) {
      Fl::remove_fd(output);
      close(output);
    }
    if (child > 0) {
      // Let the worker stop sync, kill its demo agent and discard its temporary
      // keyring after EOF. Bound teardown even if a provider or pinentry hangs.
      auto finished = [this] {
        pid_t result = waitpid(child, nullptr, WNOHANG);
        return result == child || (result < 0 && errno == ECHILD);
      };
      bool stopped = finished();
      for (int n = 0; !stopped && n < 40; ++n) {
        usleep(50000); stopped = finished();
      }
      if (!stopped) {
        kill(child, SIGTERM);
        for (int n = 0; !stopped && n < 10; ++n) {
          usleep(50000); stopped = finished();
        }
      }
      if (!stopped) {
        kill(child, SIGKILL);
        while (waitpid(child, nullptr, 0) < 0 && errno == EINTR) {}
      }
    }
  }
  void request(Json request, std::function<void(const Json &)> callback = {}) {
    if (!callback)
      callback = [](const Json &) {};
    if (output < 0) {
      callback(
          Json{{"ok", false}, {"error", gettext("Mail engine unavailable")}});
      return;
    }
    if (outgoing.size() > 64 * 1024 * 1024 || pending.size() >= 128) {
      callback(
          Json{{"ok", false}, {"error", gettext("Too many pending requests")}});
      return;
    }
    int id = nextId++;
    request["id"] = id;
    pending[id] = std::move(callback);
    outgoing += request.dump() + "\n";
    Fl::add_fd(output, FL_WRITE, writable, this);
    write();
  }
};
