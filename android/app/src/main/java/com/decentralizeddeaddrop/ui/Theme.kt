package com.decentralizeddeaddrop.ui

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color

// Colours from docs/design/app-2026-11-26-a.html: cool greys with a blue accent.
private val Light = lightColorScheme(
    primary = Color(0xFF264A9A),
    onPrimary = Color(0xFFFFFFFF),
    primaryContainer = Color(0xFFDBE4F8),
    onPrimaryContainer = Color(0xFF264A9A),
    background = Color(0xFFF5F7FA),
    onBackground = Color(0xFF141A24),
    surface = Color(0xFFF5F7FA),
    onSurface = Color(0xFF141A24),
    surfaceVariant = Color(0xFFE9EDF4),
    onSurfaceVariant = Color(0xFF586373),
    surfaceContainer = Color(0xFFE9EDF4),
    surfaceContainerHigh = Color(0xFFFFFFFF),
    surfaceContainerHighest = Color(0xFFFFFFFF),
    outline = Color(0xFF8A93A1),
    outlineVariant = Color(0xFFD3D9E3),
    error = Color(0xFFB3261E),
    onError = Color(0xFFFFFFFF),
)

private val Dark = darkColorScheme(
    primary = Color(0xFF9DB8FF),
    onPrimary = Color(0xFF0E1A3A),
    primaryContainer = Color(0xFF24345C),
    onPrimaryContainer = Color(0xFF9DB8FF),
    background = Color(0xFF11151C),
    onBackground = Color(0xFFE6EAF1),
    surface = Color(0xFF11151C),
    onSurface = Color(0xFFE6EAF1),
    surfaceVariant = Color(0xFF222A36),
    onSurfaceVariant = Color(0xFF98A3B4),
    surfaceContainer = Color(0xFF222A36),
    surfaceContainerHigh = Color(0xFF1A202A),
    surfaceContainerHighest = Color(0xFF1A202A),
    outline = Color(0xFF6B7483),
    outlineVariant = Color(0xFF2F3846),
    error = Color(0xFFFF8A80),
    onError = Color(0xFF3A0A06),
)

/** Colours with a meaning of their own, separate from the accent. */
data class Signals(val ok: Color, val waiting: Color, val off: Color)

val LocalSignals = staticCompositionLocalOf { Signals(Color(0xFF1B7245), Color(0xFF8F5B00), Color(0xFF8A93A1)) }

@Composable
fun DeadDropTheme(content: @Composable () -> Unit) {
    val dark = isSystemInDarkTheme()
    val signals = if (dark) {
        Signals(Color(0xFF6FD3A0), Color(0xFFF0B95B), Color(0xFF6B7483))
    } else {
        Signals(Color(0xFF1B7245), Color(0xFF8F5B00), Color(0xFF8A93A1))
    }
    CompositionLocalProvider(LocalSignals provides signals) {
        MaterialTheme(colorScheme = if (dark) Dark else Light, content = content)
    }
}
