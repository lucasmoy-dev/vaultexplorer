package dev.lucasmoy.ytpocket

import android.os.Build
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.platform.LocalContext

/**
 * Follow the phone's own light/dark setting.
 *
 * `MaterialTheme {}` with no colour scheme is **always light** -- which on a
 * phone set to dark is a white app among dark ones. (The same class of
 * mistake, in CSS, made the sibling app's music player draw white buttons on
 * a dark window.)
 *
 * Android 12+ gets the wallpaper-derived scheme, so the app matches whatever
 * the rest of the system looks like; older versions get Material's own
 * defaults, which are at least the right brightness.
 */
@Composable
fun AppTheme(content: @Composable () -> Unit) {
    val dark = isSystemInDarkTheme()
    val context = LocalContext.current
    val scheme = when {
        Build.VERSION.SDK_INT >= Build.VERSION_CODES.S ->
            if (dark) dynamicDarkColorScheme(context) else dynamicLightColorScheme(context)
        dark -> darkColorScheme()
        else -> lightColorScheme()
    }
    MaterialTheme(colorScheme = scheme, content = content)
}
