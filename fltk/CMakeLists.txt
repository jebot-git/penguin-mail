cmake_minimum_required(VERSION 3.21)
project(penguin-mail-fltk VERSION 1.0.10 LANGUAGES C CXX)
find_package(FLTK 1.4.5 EXACT CONFIG REQUIRED)
find_package(PkgConfig REQUIRED)
pkg_check_modules(FONTCONFIG REQUIRED IMPORTED_TARGET fontconfig)
find_program(CARGO_EXECUTABLE cargo REQUIRED)
find_program(PENGUIN_STRIP_EXECUTABLE NAMES strip llvm-strip REQUIRED)
set(ENGINE_DIR "${CMAKE_CURRENT_SOURCE_DIR}/..")
# Re-run Cargo when compiled-in OAuth configuration changes. Store only a digest.
string(SHA256 PENGUIN_OAUTH_BUILD_KEY "$ENV{PENGUIN_MAIL_GOOGLE_CLIENT_ID}|$ENV{PENGUIN_MAIL_GOOGLE_CLIENT_SECRET}|$ENV{PENGUIN_MAIL_MICROSOFT_CLIENT_ID}")
file(CONFIGURE OUTPUT "${CMAKE_CURRENT_BINARY_DIR}/oauth-client-build.stamp" CONTENT "@PENGUIN_OAUTH_BUILD_KEY@\n" @ONLY)
file(GLOB_RECURSE ENGINE_SOURCES CONFIGURE_DEPENDS "${ENGINE_DIR}/*/src/*.rs" "${ENGINE_DIR}/*/Cargo.toml" "${CMAKE_CURRENT_SOURCE_DIR}/engine/src/*.rs" "${CMAKE_CURRENT_SOURCE_DIR}/engine/Cargo.toml")
add_custom_command(OUTPUT "${CMAKE_CURRENT_BINARY_DIR}/penguin-mail-engine"
  COMMAND "${CARGO_EXECUTABLE}" build --manifest-path "${ENGINE_DIR}/Cargo.toml" --locked -p penguin-mail-engine
  COMMAND "${CMAKE_COMMAND}" -E copy_if_different "${ENGINE_DIR}/target/debug/penguin-mail-engine" "${CMAKE_CURRENT_BINARY_DIR}/penguin-mail-engine"
  COMMAND "${PENGUIN_STRIP_EXECUTABLE}" --strip-debug "${CMAKE_CURRENT_BINARY_DIR}/penguin-mail-engine"
  DEPENDS "${CMAKE_CURRENT_BINARY_DIR}/oauth-client-build.stamp" ${ENGINE_SOURCES} "${ENGINE_DIR}/Cargo.lock" "${ENGINE_DIR}/Cargo.toml"
  USES_TERMINAL)
add_custom_target(penguin-mail-core ALL DEPENDS "${CMAKE_CURRENT_BINARY_DIR}/penguin-mail-engine")
add_executable(penguin-mail native/main.cpp)
add_dependencies(penguin-mail penguin-mail-core)
target_compile_features(penguin-mail PRIVATE cxx_std_17)
target_compile_options(penguin-mail PRIVATE -Wall -Wextra -Wpedantic)
target_include_directories(penguin-mail PRIVATE native)
target_link_libraries(penguin-mail PRIVATE fltk::fltk-shared fltk::gl-shared fltk::images-shared PkgConfig::FONTCONFIG)
include(native/desktop-menu.cmake)
de_xr_desktop_menu(penguin-mail)
include(GNUInstallDirs)
install(TARGETS penguin-mail RUNTIME DESTINATION ${CMAKE_INSTALL_BINDIR})
install(PROGRAMS "${CMAKE_CURRENT_BINARY_DIR}/penguin-mail-engine" DESTINATION ${CMAKE_INSTALL_BINDIR})
install(FILES "${CMAKE_CURRENT_SOURCE_DIR}/io.github.c9dev.PenguinMail.desktop" DESTINATION ${CMAKE_INSTALL_DATADIR}/applications)
install(DIRECTORY "${ENGINE_DIR}/app/data/icons/" DESTINATION ${CMAKE_INSTALL_DATADIR}/icons/hicolor)

install(DIRECTORY "${ENGINE_DIR}/target/locale/" DESTINATION ${CMAKE_INSTALL_DATADIR}/locale OPTIONAL)
