package com.decentralizeddeaddrop.ui

import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** How many lines go in one piece of the list, so that only what is on screen is laid out. */
private const val LINES_PER_PIECE = 40

/**
 * Settings, Licences: this app's own licence, then every library built into it with its
 * licence text. The text is the file `tools/licences.py` writes into the app's assets.
 */
@Composable
fun LicencesScreen(back: () -> Unit) {
    val context = LocalContext.current
    val pieces by produceState<List<String>?>(null) {
        value = withContext(Dispatchers.IO) {
            runCatching {
                context.assets.open("licences.txt").bufferedReader().useLines { lines ->
                    lines.chunked(LINES_PER_PIECE).map { it.joinToString("\n") }.toList()
                }
            }.getOrDefault(emptyList())
        }
    }
    Page(title = "Licences", onBack = back, scrolls = false) {
        val found = pieces
        when {
            found == null -> Note("Loading…")
            found.isEmpty() -> Note("The licence text is missing from this build. It is in the source code, in LICENSE.md.")
            else -> LazyColumn(Modifier.fillMaxSize()) {
                items(found) { piece ->
                    Text(
                        piece,
                        Modifier.padding(vertical = 2.dp),
                        fontFamily = FontFamily.Monospace,
                        fontSize = 11.sp,
                        lineHeight = 15.sp,
                        color = MaterialTheme.colorScheme.onSurface,
                    )
                }
            }
        }
    }
}
