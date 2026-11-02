#pragma once

#include "rust/cxx.h"

// Application-level Qt calls not wrapped by cxx-qt-lib. TinCan needs a QApplication (not just
// a QGuiApplication) because desktop tray integrations, such as Plasma's, build QMenus.
int run_application(rust::Str qml_url);
void quit_application();
