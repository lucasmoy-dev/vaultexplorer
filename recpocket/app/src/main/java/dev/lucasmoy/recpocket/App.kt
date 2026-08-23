package dev.lucasmoy.recpocket

import android.app.Application

/**
 * Nothing to set up at startup -- the recorder is built when a recording
 * starts, and the settings are read from `SharedPreferences` wherever they
 * are needed. Declared because the manifest names it, so there is one place
 * to put process-wide setup if this ever grows any.
 */
class App : Application()
