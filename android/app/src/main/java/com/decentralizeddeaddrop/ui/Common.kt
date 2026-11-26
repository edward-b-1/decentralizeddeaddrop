package com.decentralizeddeaddrop.ui

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.decentralizeddeaddrop.engine.EngineException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch
import java.time.Instant
import java.time.LocalDate
import java.time.ZoneId
import java.time.format.DateTimeFormatter

/** What went wrong, in words for the person holding the phone. */
fun explain(failure: Throwable): String = when (failure) {
    is EngineException.Unreachable -> "Cannot reach the server, or what answered is not the server with that fingerprint."
    is EngineException.AwaitingApproval -> "The person who runs the server has not approved this phone yet."
    is EngineException.Refused -> when (failure.message) {
        "not accepted" -> "The server did not accept that. The invite may have been used already or have run out."
        "invite not accepted" -> "That invite was not accepted. It may have been used already or have run out."
        else -> "The server refused: ${failure.message}."
    }
    is EngineException -> failure.message ?: "Something went wrong."
    else -> failure.message ?: "Something went wrong."
}

/** Runs one piece of work at a time off the main thread, and shows what went wrong if it fails. */
class Runner(private val scope: CoroutineScope) {
    var busy by mutableStateOf(false)
        private set
    var problem by mutableStateOf<String?>(null)

    fun run(then: () -> Unit = {}, work: suspend () -> Unit) {
        if (busy) return
        busy = true
        scope.launch {
            try {
                work()
                then()
            } catch (failure: Exception) {
                problem = explain(failure)
            } finally {
                busy = false
            }
        }
    }
}

@Composable
fun rememberRunner(): Runner {
    val scope = rememberCoroutineScope()
    val runner = remember { Runner(scope) }
    runner.problem?.let { problem ->
        AlertDialog(
            onDismissRequest = { runner.problem = null },
            confirmButton = { TextButton(onClick = { runner.problem = null }) { Text("OK") } },
            text = { Text(problem) },
        )
    }
    return runner
}

/** A screen with a back arrow, a title and an optional line under it. */
@Composable
fun Page(
    title: String,
    subtitle: String? = null,
    onBack: () -> Unit,
    action: @Composable () -> Unit = {},
    footer: @Composable () -> Unit = {},
    scrolls: Boolean = true,
    content: @Composable ColumnScope.() -> Unit,
) {
    Column(Modifier.fillMaxSize()) {
        Row(
            Modifier.fillMaxWidth().background(MaterialTheme.colorScheme.surfaceVariant).padding(horizontal = 4.dp, vertical = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back") }
            Column(Modifier.weight(1f)) {
                Text(title, fontSize = 18.sp, fontWeight = FontWeight.Medium, maxLines = 1)
                if (subtitle != null) Text(subtitle, fontSize = 12.5.sp, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1)
            }
            action()
        }
        val body = Modifier.weight(1f).fillMaxWidth().padding(horizontal = 16.dp)
        Column(if (scrolls) body.verticalScroll(rememberScrollState()) else body, content = content)
        footer()
    }
}

@Composable
fun Footer(content: @Composable RowScope.() -> Unit) {
    Column {
        HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
        Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 10.dp), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            content()
        }
    }
}

@Composable
fun Heading(text: String) {
    Text(
        text.uppercase(),
        Modifier.padding(top = 18.dp, bottom = 4.dp),
        fontSize = 12.5.sp,
        fontWeight = FontWeight.Medium,
        letterSpacing = 0.5.sp,
        color = MaterialTheme.colorScheme.primary,
    )
}

@Composable
fun Note(text: String, modifier: Modifier = Modifier) {
    Text(text, modifier.padding(vertical = 6.dp), fontSize = 13.sp, lineHeight = 19.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
}

/** One row of a settings-style list: a title, an optional line under it, and something at the end. */
@Composable
fun Item(
    title: String,
    detail: String? = null,
    value: String? = null,
    danger: Boolean = false,
    highlight: Boolean = false,
    onClick: (() -> Unit)? = null,
    end: @Composable () -> Unit = {},
) {
    val shape = RoundedCornerShape(12.dp)
    var row = Modifier.fillMaxWidth()
    if (highlight) row = row.padding(vertical = 4.dp).clip(shape).background(MaterialTheme.colorScheme.primaryContainer)
    if (onClick != null) row = row.clickable(onClick = onClick)
    Column {
        Row(row.padding(horizontal = if (highlight) 8.dp else 0.dp, vertical = 11.dp), verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text(title, fontSize = 15.sp, color = if (danger) MaterialTheme.colorScheme.error else Color.Unspecified)
                if (detail != null) Text(detail, fontSize = 13.sp, lineHeight = 18.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            if (value != null) Text(value, Modifier.padding(start = 12.dp), fontSize = 14.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
            end()
            if (onClick != null && !danger) {
                Icon(Icons.AutoMirrored.Filled.KeyboardArrowRight, contentDescription = null, tint = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
        if (!highlight) HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
    }
}

@Composable
fun Badge(count: Int) {
    Box(
        Modifier.padding(start = 8.dp).clip(RoundedCornerShape(10.dp)).background(MaterialTheme.colorScheme.primary).padding(horizontal = 7.dp, vertical = 1.dp),
    ) {
        Text(count.toString(), fontSize = 12.sp, fontWeight = FontWeight.Medium, color = MaterialTheme.colorScheme.onPrimary)
    }
}

@Composable
fun Light(color: Color) {
    Box(Modifier.size(10.dp).clip(CircleShape).background(color))
}

/** A fingerprint in its groups of four, laid out four groups to a line. */
@Composable
fun Fingerprint(fingerprint: String) {
    val groups = fingerprint.split("-")
    Column(
        Modifier.fillMaxWidth().padding(vertical = 12.dp).clip(RoundedCornerShape(12.dp))
            .background(MaterialTheme.colorScheme.surfaceContainerHigh).padding(12.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        groups.chunked(4).forEach { line ->
            Row(Modifier.fillMaxWidth()) {
                line.forEach { group ->
                    Text(group, Modifier.weight(1f), fontFamily = FontFamily.Monospace, fontSize = 17.sp)
                }
                repeat(4 - line.size) { Spacer(Modifier.weight(1f)) }
            }
        }
    }
}

/** Asks before doing something that cannot be undone. */
@Composable
fun Confirm(question: String, detail: String, yes: String, onYes: () -> Unit, onNo: () -> Unit) {
    AlertDialog(
        onDismissRequest = onNo,
        title = { Text(question, fontSize = 18.sp) },
        text = { Text(detail) },
        confirmButton = { TextButton(onClick = onYes) { Text(yes, color = MaterialTheme.colorScheme.error) } },
        dismissButton = { TextButton(onClick = onNo) { Text("Cancel") } },
    )
}

/** Shows an invite link, to copy or pass to another app. */
@Composable
fun LinkDialog(title: String, note: String, link: String, onClose: () -> Unit) {
    val context = LocalContext.current
    AlertDialog(
        onDismissRequest = onClose,
        title = { Text(title, fontSize = 18.sp) },
        text = {
            Column {
                Text(note, fontSize = 13.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
                Spacer(Modifier.size(10.dp))
                SelectionContainer { Text(link, fontFamily = FontFamily.Monospace, fontSize = 12.sp, lineHeight = 17.sp) }
            }
        },
        confirmButton = {
            TextButton(onClick = {
                val send = Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_TEXT, link)
                context.startActivity(Intent.createChooser(send, null))
            }) { Text("Share") }
        },
        dismissButton = {
            Row {
                TextButton(onClick = {
                    val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                    clipboard.setPrimaryClip(ClipData.newPlainText("Dead Drop invite", link))
                }) { Text("Copy") }
                Spacer(Modifier.width(4.dp))
                TextButton(onClick = onClose) { Text("Close") }
            }
        },
    )
}

@Composable
fun Primary(text: String, modifier: Modifier = Modifier, enabled: Boolean = true, onClick: () -> Unit) {
    Button(onClick = onClick, modifier = modifier, enabled = enabled) { Text(text) }
}

@Composable
fun Secondary(text: String, modifier: Modifier = Modifier, enabled: Boolean = true, onClick: () -> Unit) {
    OutlinedButton(onClick = onClick, modifier = modifier, enabled = enabled) { Text(text) }
}

/** 18:42 for today, Tue for this week, 12 Nov otherwise. */
fun shortTime(seconds: Long): String {
    if (seconds == 0L) return ""
    val zone = ZoneId.systemDefault()
    val moment = Instant.ofEpochSecond(seconds).atZone(zone)
    val today = LocalDate.now(zone)
    val pattern = when {
        moment.toLocalDate() == today -> "HH:mm"
        moment.toLocalDate().isAfter(today.minusDays(6)) -> "EEE"
        else -> "d MMM"
    }
    return moment.format(DateTimeFormatter.ofPattern(pattern))
}

/** 1 hour, 24 hours, 3 days. */
fun ageText(seconds: Long): String = when {
    seconds % 86_400 == 0L && seconds >= 2 * 86_400 -> "${seconds / 86_400} days"
    seconds % 3600 == 0L -> if (seconds == 3600L) "1 hour" else "${seconds / 3600} hours"
    seconds % 60 == 0L -> if (seconds == 60L) "1 minute" else "${seconds / 60} minutes"
    else -> "$seconds seconds"
}
