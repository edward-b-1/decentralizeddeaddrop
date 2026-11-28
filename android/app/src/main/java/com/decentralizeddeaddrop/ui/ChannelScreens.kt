package com.decentralizeddeaddrop.ui

import androidx.compose.runtime.rememberCoroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import androidx.activity.compose.BackHandler
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material.icons.filled.Info
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Switch
import androidx.compose.material3.AlertDialog
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.decentralizeddeaddrop.data.ChannelUi
import com.decentralizeddeaddrop.data.MessageUi
import com.decentralizeddeaddrop.data.Sending
import com.decentralizeddeaddrop.data.ServerUi
import com.decentralizeddeaddrop.data.Servers
import com.decentralizeddeaddrop.engine.JoinRequestInfo
import com.decentralizeddeaddrop.engine.readLink

// ---------------------------------------------------------------- a channel (mockup screen 2)

@Composable
fun ConversationScreen(channel: ChannelUi, server: ServerUi?, back: () -> Unit, go: (Screen) -> Unit) {
    val all by Servers.messages.collectAsStateWithLifecycle()
    val messages = all[channel.key].orEmpty()
    val earlier by Servers.earlier.collectAsStateWithLifecycle()
    val hasEarlier = channel.key in earlier
    val scope = rememberCoroutineScope()
    val runner = rememberRunner()
    var draft by rememberSaveable(channel.key) { mutableStateOf("") }
    val list = rememberLazyListState()

    // While this channel is on screen its messages are read as they arrive.
    DisposableEffect(channel.key) {
        Servers.open = channel.key
        onDispose { Servers.open = null }
    }
    // Follows the newest message. Bringing in earlier ones leaves the view where it was.
    // Also when the foot of the screen changes height, which would otherwise cover it.
    LaunchedEffect(messages.lastOrNull()?.id, channel.unlisted) {
        Servers.markRead(channel.key)
        if (messages.isNotEmpty()) list.animateScrollToItem(messages.lastIndex + if (hasEarlier) 2 else 1)
    }

    val present = channel.members.count { !it.hasLeft }
    Page(
        title = channel.name,
        subtitle = "${server?.label.orEmpty()} · " + if (channel.unlisted) "no longer on the server" else "$present ${if (present == 1) "member" else "members"}",
        onBack = back,
        scrolls = false,
        action = {
            IconButton(onClick = { go(Screen.ChannelDetails(channel.key)) }) { Icon(Icons.Filled.Info, contentDescription = "Channel details") }
        },
        footer = {
            Column {
                HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
                if (channel.unlisted) {
                    Note(
                        "The server no longer shows this channel to this phone, so nothing can be sent or received here. What you already have is kept. Channel details says more.",
                        Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
                    )
                } else Row(Modifier.fillMaxWidth().padding(start = 12.dp, end = 8.dp, top = 6.dp, bottom = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                    OutlinedTextField(
                        value = draft,
                        onValueChange = { draft = it },
                        modifier = Modifier.weight(1f),
                        placeholder = { Text("Message") },
                        shape = RoundedCornerShape(22.dp),
                        maxLines = 4,
                    )
                    val text = draft.trim()
                    IconButton(
                        onClick = {
                            draft = ""
                            runner.run { Servers.send(channel.slot, channel.id, text) }
                        },
                        enabled = text.isNotEmpty() && !runner.busy,
                        modifier = Modifier.padding(start = 6.dp).clip(CircleShape).background(MaterialTheme.colorScheme.primary),
                    ) {
                        Icon(Icons.AutoMirrored.Filled.Send, contentDescription = "Send", tint = MaterialTheme.colorScheme.onPrimary)
                    }
                }
            }
        },
    ) {
        LazyColumn(Modifier.weight(1f).fillMaxWidth(), state = list, verticalArrangement = Arrangement.spacedBy(10.dp)) {
            item {
                Text(
                    "Messages wait on the server for up to ${ageText(channel.maxAge)}, then are wiped.",
                    Modifier.fillMaxWidth().padding(top = 12.dp, start = 24.dp, end = 24.dp),
                    fontSize = 12.5.sp,
                    textAlign = TextAlign.Center,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            if (hasEarlier) {
                item(key = "earlier") {
                    Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) {
                        TextButton(onClick = {
                            scope.launch {
                                // Put the oldest message at the very top first: the list holds on to
                                // whatever is first on screen while earlier ones are added above it.
                                list.scrollToItem(2)
                                Servers.loadEarlier(channel.key)
                            }
                        }) { Text("Show earlier messages") }
                    }
                }
            }
            items(messages, key = { it.id }) { message ->
                // What the person asks to be done with a message of theirs that has not gone.
                // Not through the runner: it does one thing at a time, and drops the rest.
                Bubble(message, channel) { work ->
                    scope.launch {
                        try {
                            work()
                        } catch (failure: Exception) {
                            runner.problem = explain(failure)
                        }
                    }
                }
            }
            item { Box(Modifier.size(4.dp)) }
        }
    }
}

/**
 * One message. A message of one's own that the server has taken looks as it always has.
 * One still waiting to be sent, or refused, is paler and says so under it (mockup
 * send-status-2026-11-28-a): the words carry it, not the colour alone. Pressing one of
 * those, or holding it, opens what can be done about it ([UnsentDialog]).
 */
@Composable
private fun Bubble(message: MessageUi, channel: ChannelUi, act: (suspend () -> Unit) -> Unit) {
    // With a signal a message goes in a moment. It is not called waiting until it has
    // waited a second, so that the words do not flash under everything that is sent.
    var waited by remember(message.id) { mutableStateOf(System.currentTimeMillis() / 1000 - message.sent > 2) }
    if (message.sending == Sending.Waiting && !waited) {
        LaunchedEffect(message.id) {
            delay(1000)
            waited = true
        }
    }
    val waiting = message.sending == Sending.Waiting && waited
    val failed = message.sending == Sending.NotSent
    var explaining by remember(message.id) { mutableStateOf(false) }
    if (explaining && message.sending != Sending.Sent) UnsentDialog(message, channel, close = { explaining = false }, act = act)

    if (message.unreadable) {
        Text(
            "A message arrived that this phone could not open.",
            Modifier.fillMaxWidth(),
            fontSize = 12.5.sp,
            fontStyle = FontStyle.Italic,
            textAlign = TextAlign.Center,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        return
    }
    Column(Modifier.fillMaxWidth(), horizontalAlignment = if (message.mine) Alignment.End else Alignment.Start) {
        if (!message.mine) {
            Text(message.name, fontSize = 12.5.sp, fontWeight = FontWeight.Medium, color = MaterialTheme.colorScheme.primary)
        }
        val fill = when {
            !message.mine -> MaterialTheme.colorScheme.surfaceContainerHigh
            waiting || failed -> MaterialTheme.colorScheme.primaryContainer.copy(alpha = 0.45f)
            else -> MaterialTheme.colorScheme.primaryContainer
        }
        val why = if (waiting || failed) {
            Modifier.combinedClickable(
                onClickLabel = "See what can be done",
                onLongClick = { explaining = true },
                onClick = { explaining = true },
            )
        } else {
            Modifier
        }
        Text(
            message.text,
            Modifier.widthIn(max = 280.dp).clip(RoundedCornerShape(14.dp)).background(fill).then(why).padding(horizontal = 11.dp, vertical = 8.dp),
            fontSize = 15.sp,
        )
        Row(why, horizontalArrangement = Arrangement.spacedBy(4.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(shortTime(message.sent), fontSize = 12.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
            if (waiting || failed) {
                val colour = if (failed) MaterialTheme.colorScheme.error else LocalSignals.current.waiting
                Text("·", Modifier.clearAndSetSemantics {}, fontSize = 12.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
                SendingMark(clock = waiting, colour)
                Text(if (failed) "Not sent" else "Waiting to send", fontSize = 12.sp, fontWeight = FontWeight.Medium, color = colour)
            }
        }
    }
}

/** A small clock, for waiting, or a ringed exclamation mark, for not sent. Decoration: the words beside it say the same. */
@Composable
private fun SendingMark(clock: Boolean, colour: Color) {
    Canvas(Modifier.size(12.dp)) {
        val line = 1.4.dp.toPx()
        val radius = size.minDimension / 2 - line / 2
        drawCircle(colour, radius, style = Stroke(line))
        if (clock) {
            drawLine(colour, center, Offset(center.x, center.y - radius * 0.6f), line, StrokeCap.Round)
            drawLine(colour, center, Offset(center.x + radius * 0.45f, center.y + radius * 0.3f), line, StrokeCap.Round)
        } else {
            drawLine(colour, Offset(center.x, center.y - radius * 0.5f), Offset(center.x, center.y + radius * 0.05f), line, StrokeCap.Round)
            drawCircle(colour, line * 0.65f, Offset(center.x, center.y + radius * 0.5f))
        }
    }
}

/**
 * What can be done with a message of one's own that has not gone: one still waiting to be
 * sent, or one that was not sent. Says where it stands, and offers to try it, to copy its
 * text, or to delete it. Deleting one that is waiting takes it back, so that it never goes.
 */
@Composable
private fun UnsentDialog(message: MessageUi, channel: ChannelUi, close: () -> Unit, act: (suspend () -> Unit) -> Unit) {
    val context = LocalContext.current
    val waiting = message.sending == Sending.Waiting
    val state = when {
        waiting -> "The server has not taken it yet. It is tried again by itself until it goes."
        message.problem == MessageUi.NOT_IN_CHANNEL -> "The server would not take it, because this phone is no longer in ${channel.name}."
        message.problem == MessageUi.TOO_LARGE -> "It is too large to send. It can be sent as several shorter messages."
        else -> "It could not be sent: ${message.problem.orEmpty().trimEnd('.')}."
    }
    AlertDialog(
        onDismissRequest = close,
        title = { Text(if (waiting) "This message is waiting to be sent" else "This message was not sent") },
        text = {
            Column {
                Text("$state Nobody else has it.", Modifier.padding(bottom = 8.dp))
                if (waiting) {
                    Choice("Try now") {
                        close()
                        act { Servers.sendNow(channel.key, message.id) }
                    }
                } else if (message.problem != MessageUi.TOO_LARGE && !channel.unlisted) {
                    // Sent as a new message. Not offered where it could only be refused
                    // again: one too large, or a channel this phone cannot send to.
                    Choice("Try again") {
                        close()
                        act { Servers.sendAgain(channel.key, message.id, message.text) }
                    }
                }
                Choice("Copy text") {
                    val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                    clipboard.setPrimaryClip(ClipData.newPlainText("Message", message.text))
                    close()
                }
                Choice(if (waiting) "Delete, and do not send" else "Delete", MaterialTheme.colorScheme.error) {
                    close()
                    act { Servers.deleteMessage(channel.key, message.id) }
                }
            }
        },
        confirmButton = { TextButton(onClick = close) { Text("Close") } },
    )
}

/** One of the things a dialog offers to do, on a line of its own. */
@Composable
private fun Choice(label: String, colour: Color = MaterialTheme.colorScheme.primary, onClick: () -> Unit) {
    Text(
        label,
        Modifier.fillMaxWidth().clip(RoundedCornerShape(8.dp)).clickable(role = Role.Button, onClick = onClick).padding(vertical = 12.dp, horizontal = 4.dp),
        fontSize = 15.sp,
        fontWeight = FontWeight.Medium,
        color = colour,
    )
}

// ---------------------------------------------------------------- channel details (mockup screen 3)

@Composable
fun ChannelDetailsScreen(channel: ChannelUi, server: ServerUi?, back: () -> Unit, go: (Screen) -> Unit, home: () -> Unit) {
    val runner = rememberRunner()
    val requests = channel.asking
    var invite by remember { mutableStateOf<String?>(null) }
    var confirming by remember { mutableStateOf(false) }
    var renaming by remember { mutableStateOf(false) }
    var naming by remember { mutableStateOf(false) }
    var notice by remember { mutableStateOf<String?>(null) }

    if (renaming) {
        TextDialog("Channel name", "Name", channel.name, 64, onClose = { renaming = false }) { name ->
            renaming = false
            runner.run { Servers.renameChannel(channel.slot, channel.id, name) }
        }
    }
    if (naming) {
        TextDialog("Your name in ${channel.name}", "Name", channel.myName, 32, onClose = { naming = false }) { name ->
            naming = false
            runner.run {
                if (!Servers.setMyName(channel.slot, channel.id, name)) {
                    notice = "Asked. Names are part of the member list, which only the channel's owner signs. Their phone makes the change when it is next connected."
                }
            }
        }
    }
    notice?.let { text -> Notice(text) { notice = null } }

    invite?.let { link ->
        LinkDialog(
            title = "Invite to ${channel.name}",
            note = "Show or send this to one person who is already on ${server?.label ?: "the server"}. It works once, for a day. They ask to join; you then let them in.",
            link = link,
            onClose = { invite = null },
        )
    }
    if (confirming) {
        if (channel.unlisted) {
            Confirm(
                "Remove ${channel.name} from this phone?",
                "Its history and its keys are removed from this phone. If the channel still exists, you would need a new invite to come back.",
                "Remove",
                onNo = { confirming = false },
                onYes = {
                    confirming = false
                    runner.run(then = home) { Servers.leave(channel.slot, channel.id) }
                },
            )
        } else if (channel.ownedByMe) {
            Confirm("Delete ${channel.name}?", "It is removed for everyone, with anything still waiting on the server. This cannot be undone.", "Delete", onNo = { confirming = false }, onYes = {
                confirming = false
                runner.run(then = home) { Servers.deleteChannel(channel.slot, channel.id) }
            })
        } else {
            Confirm("Leave ${channel.name}?", "You stop receiving its messages, and its history is removed from this phone.", "Leave", onNo = { confirming = false }, onYes = {
                confirming = false
                runner.run(then = home) { Servers.leave(channel.slot, channel.id) }
            })
        }
    }

    if (channel.unlisted) {
        Page(title = channel.name, subtitle = server?.label, onBack = back) {
            Note(
                "The server no longer shows this channel to this phone, and has shown nothing signed by the channel's owner to say that you were removed or that the channel was deleted.",
                Modifier.padding(top = 8.dp),
            )
            Note(
                "That happens when the owner deleted the channel with an older version of the app, when the owner's phone was taken off the server, or when this phone was away for more than a month. It can also mean the server is at fault.",
            )
            Note("Nothing can be sent or received here. This phone keeps the channel and its history until you remove it. If the server shows the channel again, it carries on as before.")
            Heading("This channel")
            Item("Channel name", value = channel.name)
            Item("Your name here", value = channel.myName)
            Item("Members", value = channel.members.size.toString(), onClick = { go(Screen.Members(channel.key)) })
            Heading(" ")
            Item("Remove from this phone", danger = true, onClick = { confirming = true })
        }
        return
    }

    Page(title = channel.name, subtitle = server?.label, onBack = back) {
        if (requests > 0) {
            Item(
                "Join requests",
                detail = if (requests == 1) "Someone is asking to join" else "$requests people are asking to join",
                highlight = true,
                onClick = { go(Screen.JoinRequests(channel.key)) },
                end = { Badge(requests) },
            )
        }
        Heading("This channel")
        Item("Channel name", value = channel.name, onClick = if (channel.ownedByMe) ({ renaming = true }) else null)
        Item("Your name here", value = channel.myName, onClick = { naming = true })
        Item(
            "Maximum message age",
            detail = "Undelivered messages are wiped from the server after this",
            value = ageText(channel.maxAge),
            onClick = { go(Screen.MaxAge(channel.key)) },
        )
        Heading("People")
        Item(
            "Members",
            detail = if (channel.ownedByMe) "You own this channel" else null,
            value = channel.members.count { !it.hasLeft }.toString(),
            onClick = { go(Screen.Members(channel.key)) },
        )
        if (channel.ownedByMe) {
            Item("Invite someone", detail = "Make a link or QR code", onClick = {
                runner.run { invite = Servers.inviteToChannel(channel.slot, channel.id) }
            })
        }
        Item("Safety numbers", detail = "Check you are talking to the right people", onClick = { go(Screen.Members(channel.key)) })
        Heading(" ")
        if (channel.ownedByMe) {
            Item("Delete channel", detail = "Removes it for everyone. Cannot be undone.", danger = true, onClick = { confirming = true })
        } else {
            Item("Leave channel", danger = true, onClick = { confirming = true })
        }
    }
}

@Composable
fun MembersScreen(channel: ChannelUi, back: () -> Unit, go: (Screen) -> Unit) {
    val runner = rememberRunner()
    var removing by remember { mutableStateOf<String?>(null) }
    removing?.let { device ->
        val member = channel.members.firstOrNull { it.device == device }
        val name = member?.name ?: device
        val effect = when {
            member?.missing == true -> "The server no longer has their phone. This takes them off the member list. To come back they would need a new invite."
            member?.hasLeft == true -> "The server says they have left, but their phone did not confirm it. This takes them off the member list. To come back they would need a new invite."
            else -> "They stop receiving messages at once, and cannot read anything sent from now on."
        }
        Confirm("Remove $name?", effect, "Remove", onNo = { removing = null }, onYes = {
            removing = null
            runner.run { Servers.removeMember(channel.slot, channel.id, device) }
        })
    }
    Page(title = "Members", subtitle = channel.name, onBack = back) {
        Note("Tap someone to see the safety number you share with them.")
        channel.members.forEach { member ->
            Item(
                member.name,
                detail = listOfNotNull(
                    "You".takeIf { member.isMe },
                    "Owner".takeIf { member.device == channel.owner },
                    when {
                        member.leftConfirmed -> "Has left"
                        member.missing -> "No longer on this server"
                        member.hasLeft -> "Left, not confirmed"
                        else -> null
                    },
                    "Safety number checked".takeIf { member.verified },
                ).joinToString(" · ").ifEmpty { null },
                onClick = if (member.isMe || member.hasLeft) null else ({ go(Screen.Safety(channel.key, member.device)) }),
                end = {
                    // Someone who has left by their own signed word goes off the list by
                    // itself. Anyone else stays until the owner removes them: the server
                    // saying they have gone is not enough to sign them out.
                    if (channel.ownedByMe && !channel.unlisted && member.device != channel.owner && !member.leftConfirmed) {
                        TextButton(onClick = { removing = member.device }, enabled = !runner.busy) {
                            Text("Remove", color = MaterialTheme.colorScheme.error)
                        }
                    }
                },
            )
        }
    }
}

@Composable
fun JoinRequestsScreen(channel: ChannelUi, back: () -> Unit) {
    val runner = rememberRunner()
    var requests by remember { mutableStateOf<List<JoinRequestInfo>?>(null) }
    var reload by remember { mutableIntStateOf(0) }
    // Asked again whenever the server says the number of people asking has changed.
    LaunchedEffect(channel.key, reload, channel.asking) {
        requests = runCatching { Servers.joinRequests(channel.slot, channel.id) }.getOrDefault(emptyList())
    }
    Page(title = "Join requests", subtitle = channel.name, onBack = back) {
        Note("Each of these has an invite you made and is asking to join. Letting someone in gives them messages from then on, not earlier ones.")
        when {
            requests == null -> Note("Looking…")
            requests!!.isEmpty() -> Note("Nobody is asking at the moment.")
        }
        requests.orEmpty().forEach { request ->
            Item(request.name, detail = "Device ${request.device}", end = {
                Primary("Let in", enabled = !runner.busy) {
                    runner.run(then = { reload += 1 }) { Servers.admit(channel.slot, channel.id, request.device) }
                }
            })
        }
    }
}

// ---------------------------------------------------------------- maximum message age (mockup screen 4)

private val AGES = listOf(3600L, 6 * 3600L, 24 * 3600L, 3 * 86_400L, 7 * 86_400L, 30 * 86_400L)

@Composable
fun MaxAgeScreen(channel: ChannelUi, back: () -> Unit) {
    val runner = rememberRunner()
    var chosen by remember { mutableLongStateOf(channel.maxAge) }
    var custom by remember { mutableStateOf(false) }
    val choices = (AGES + channel.maxAge + chosen).distinct().sorted()
    if (custom) {
        AgeDialog(onClose = { custom = false }) { seconds ->
            custom = false
            chosen = seconds
        }
    }
    Page(
        title = "Maximum message age",
        subtitle = channel.name,
        onBack = back,
        footer = {
            if (channel.ownedByMe) {
                Footer {
                    Secondary("Cancel", Modifier.weight(1f), onClick = back)
                    Primary("OK", Modifier.weight(1f), enabled = !runner.busy) {
                        if (chosen == channel.maxAge) back() else runner.run(then = back) { Servers.setMaxAge(channel.slot, channel.id, chosen) }
                    }
                }
            }
        },
    ) {
        Note("A message is wiped from the server as soon as everyone has received it, or when it reaches this age, whichever comes first.")
        if (channel.ownedByMe) {
            choices.forEach { age ->
                Row(
                    Modifier.fillMaxWidth().selectable(selected = chosen == age, role = Role.RadioButton, onClick = { chosen = age }).padding(vertical = 2.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    RadioButton(selected = chosen == age, onClick = null, modifier = Modifier.padding(end = 12.dp, top = 10.dp, bottom = 10.dp))
                    Text(ageText(age), fontSize = 15.sp)
                }
                HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
            }
            Item("Another length", detail = "A number of minutes, hours or days", onClick = { custom = true })
        } else {
            Item("This channel", value = ageText(channel.maxAge))
            Note("Only the channel's owner can change this.")
        }
        Note("A phone that stays off or out of signal for longer than this will miss what was sent in the meantime.")
    }
}

// ---------------------------------------------------------------- starting and joining channels

@Composable
fun NewChannelScreen(servers: List<ServerUi>, back: () -> Unit, created: (String) -> Unit) {
    val runner = rememberRunner()
    var slot by remember { mutableIntStateOf(servers.firstOrNull()?.slot ?: 0) }
    var name by rememberSaveable { mutableStateOf("") }
    var myName by rememberSaveable { mutableStateOf("") }
    Page(
        title = "New channel",
        onBack = back,
        footer = {
            Footer {
                Primary("Create", Modifier.weight(1f), enabled = name.isNotBlank() && myName.isNotBlank() && !runner.busy) {
                    runner.run {
                        val id = Servers.createChannel(slot, name.trim(), myName.trim(), 24 * 3600L)
                        created("$slot/$id")
                    }
                }
            }
        },
    ) {
        if (servers.size > 1) {
            Heading("On which server")
            servers.forEach { server ->
                Row(
                    Modifier.fillMaxWidth().selectable(selected = slot == server.slot, role = Role.RadioButton, onClick = { slot = server.slot }),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    RadioButton(selected = slot == server.slot, onClick = null, modifier = Modifier.padding(end = 12.dp, top = 10.dp, bottom = 10.dp))
                    Text(server.label, fontSize = 15.sp)
                }
            }
        }
        Heading("The channel")
        OutlinedTextField(name, { name = it.take(64) }, Modifier.fillMaxWidth(), label = { Text("Channel name") }, singleLine = true)
        OutlinedTextField(myName, { myName = it.take(32) }, Modifier.fillMaxWidth().padding(top = 10.dp), label = { Text("Your name in this channel") }, singleLine = true)
        Note("You will own this channel: only you can let people in, remove them, or delete it. Messages wait on the server for up to 24 hours; you can change that afterwards.")
        Note("The server can see the channel's name and who is in it, but never what is said.")
    }
}

@Composable
fun JoinChannelScreen(initialLink: String, back: () -> Unit, openLink: (String) -> Unit) {
    val runner = rememberRunner()
    var link by rememberSaveable { mutableStateOf(initialLink) }
    var myName by rememberSaveable { mutableStateOf("") }
    var asked by remember { mutableStateOf(false) }
    var scanning by rememberSaveable { mutableStateOf(false) }
    if (scanning) {
        BackHandler { scanning = false }
        ScanScreen("Scan an invite", "Point the camera at the invite code on the channel owner's phone.", back = { scanning = false }) {
            scanning = false
            openLink(it)
        }
        return
    }
    val info = readLink(link.trim())
    val slot = Servers.serverFor(link.trim())
    val problem = when {
        link.isBlank() -> null
        info == null -> "That is not a complete Dead Drop link."
        info.channel == null -> "That link adds a server; it is not an invite to a channel. Use Servers, Add a server."
        slot == null && !info.hasServerToken -> "This phone is not on that server (${info.host}:${info.port}) yet. Ask the person who runs it for a link that lets you on, then use this invite."
        else -> null
    }
    Page(
        title = "Join with an invite",
        onBack = back,
        footer = {
            Footer {
                if (asked) {
                    Primary("Done", Modifier.weight(1f), onClick = back)
                } else {
                    Primary("Ask to join", Modifier.weight(1f), enabled = info?.channel != null && problem == null && myName.isNotBlank() && !runner.busy) {
                        runner.run(then = { asked = true }) {
                            // An invite can also carry the way onto the server itself.
                            val server = slot ?: Servers.addServer(link.trim(), null, "")
                            Servers.askToJoin(server, link.trim(), myName.trim())
                        }
                    }
                }
            }
        },
    ) {
        if (asked) {
            Text("Asked", Modifier.padding(top = 18.dp), fontSize = 17.sp, fontWeight = FontWeight.Medium)
            Note("The channel's owner has been asked. It appears in your list as soon as they let you in.")
            return@Page
        }
        Primary("Scan a QR code", Modifier.fillMaxWidth().padding(top = 14.dp)) { scanning = true }
        Note("Scan the invite on the channel owner's phone, or paste the link they sent you.")
        OutlinedTextField(link, { link = it }, Modifier.fillMaxWidth(), label = { Text("Or paste a link") }, minLines = 3, maxLines = 5)
        if (problem != null) Text(problem, Modifier.padding(top = 8.dp), fontSize = 13.sp, color = MaterialTheme.colorScheme.error)
        OutlinedTextField(myName, { myName = it.take(32) }, Modifier.fillMaxWidth().padding(top = 10.dp), label = { Text("Your name in this channel") }, singleLine = true)
        Note("The app checks that the channel really belongs to the person the invite came from before it asks.")
        if (info?.channel != null && slot == null && info.hasServerToken) {
            // The invite also holds the way onto a server this phone is not on yet.
            Heading("This invite also adds a server")
            Item("Address", value = "${info.host} : ${info.port}")
            Text("Fingerprint", Modifier.padding(top = 11.dp), fontSize = 15.sp)
            Fingerprint(info.fingerprint)
            Note("This phone is not on that server yet. Asking to join puts it on, with a new identity. The app connects only to the server with this fingerprint.")
        }
    }
}

// ---------------------------------------------------------------- another maximum message age

private val UNITS = listOf("Minutes" to 60L, "Hours" to 3600L, "Days" to 86_400L)

@Composable
private fun AgeDialog(onClose: () -> Unit, onSet: (Long) -> Unit) {
    var number by remember { mutableStateOf("") }
    var unit by remember { mutableIntStateOf(1) }
    // Up to ten years: long enough for anyone, short enough that nothing overflows.
    val seconds = number.toLongOrNull()?.times(UNITS[unit].second)?.takeIf { it in 60L..(3660L * 86_400L) }
    AlertDialog(
        onDismissRequest = onClose,
        title = { Text("Another length", fontSize = 18.sp) },
        text = {
            Column {
                OutlinedTextField(
                    number,
                    { number = it.filter(Char::isDigit).take(5) },
                    label = { Text("How many") },
                    singleLine = true,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                )
                UNITS.forEachIndexed { index, (name, _) ->
                    Row(
                        Modifier.fillMaxWidth().selectable(selected = unit == index, role = Role.RadioButton, onClick = { unit = index }),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        RadioButton(selected = unit == index, onClick = null, modifier = Modifier.padding(end = 12.dp, top = 8.dp, bottom = 8.dp))
                        Text(name, fontSize = 15.sp)
                    }
                }
            }
        },
        confirmButton = { TextButton(onClick = { seconds?.let(onSet) }, enabled = seconds != null) { Text("Set") } },
        dismissButton = { TextButton(onClick = onClose) { Text("Cancel") } },
    )
}

// ---------------------------------------------------------------- safety numbers

/**
 * The safety number this phone shares with one member. If both phones show the same
 * number, each holds the other's real keys and nobody is in between.
 */
@Composable
fun SafetyScreen(channel: ChannelUi, device: String, back: () -> Unit) {
    val member = channel.members.firstOrNull { it.device == device }
    if (member == null) {
        LaunchedEffect(Unit) { back() }
        return
    }
    val runner = rememberRunner()
    var number by remember { mutableStateOf<String?>(null) }
    var scanning by rememberSaveable { mutableStateOf(false) }
    var outcome by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(device) { number = runCatching { Servers.safetyNumber(channel.slot, device) }.getOrNull() }

    val mine = number
    if (scanning && mine != null) {
        BackHandler { scanning = false }
        ScanScreen("Scan ${member.name}'s code", "Point the camera at the safety number code on ${member.name}'s phone.", back = { scanning = false }) { scanned ->
            scanning = false
            when (safetyMatches(scanned, mine)) {
                true -> {
                    outcome = "The numbers match. This phone and ${member.name}'s hold each other's real keys, and nobody is in between."
                    runner.run { Servers.setVerified(channel.slot, device, true) }
                }
                false -> {
                    outcome = "The numbers do NOT match. The device listed as ${member.name} in this channel is not the phone you scanned. Do not rely on this channel for anything private, and tell its owner."
                    runner.run { Servers.setVerified(channel.slot, device, false) }
                }
                null -> outcome = "That code is not a safety number. Ask ${member.name} to open this same screen for you on their phone."
            }
        }
        return
    }
    outcome?.let { text -> Notice(text) { outcome = null } }

    Page(title = "Safety number", subtitle = "You and ${member.name}", onBack = back) {
        Note("If ${member.name}'s phone shows this same number for you, your messages are sealed with each other's real keys and nobody is in between. Compare it in person, or on a call where you know their voice.")
        if (mine == null) {
            Note("Working it out…")
            return@Page
        }
        SafetyDigits(mine)
        QrCode(safetyCode(mine), Modifier.fillMaxWidth().padding(horizontal = 56.dp, vertical = 4.dp))
        Secondary("Scan their code instead", Modifier.fillMaxWidth().padding(top = 10.dp)) { scanning = true }
        Item(
            "I have compared this number",
            detail = if (member.verified) "Marked as checked on this phone" else "Switch on once you have",
            end = { Switch(member.verified, { on -> runner.run { Servers.setVerified(channel.slot, device, on) } }, enabled = !runner.busy) },
        )
        Item("Their device ID", detail = device)
        Note("If the numbers differ, the device listed as ${member.name} is not the phone in their hand. Do not rely on this channel for anything private, and tell its owner.")
        Note("${member.name} gets a new number if they move to a new phone or join the server again.")
    }
}
