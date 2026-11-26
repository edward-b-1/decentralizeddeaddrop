package com.decentralizeddeaddrop.ui

import androidx.compose.foundation.background
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
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.decentralizeddeaddrop.data.ChannelUi
import com.decentralizeddeaddrop.data.MessageUi
import com.decentralizeddeaddrop.data.ServerUi
import com.decentralizeddeaddrop.data.Servers
import com.decentralizeddeaddrop.engine.JoinRequestInfo
import com.decentralizeddeaddrop.engine.readLink

// ---------------------------------------------------------------- a channel (mockup screen 2)

@Composable
fun ConversationScreen(channel: ChannelUi, server: ServerUi?, back: () -> Unit, go: (Screen) -> Unit) {
    val all by Servers.messages.collectAsStateWithLifecycle()
    val messages = all[channel.key].orEmpty()
    val runner = rememberRunner()
    var draft by rememberSaveable(channel.key) { mutableStateOf("") }
    val list = rememberLazyListState()

    // While this channel is on screen its messages are read as they arrive.
    DisposableEffect(channel.key) {
        Servers.open = channel.key
        onDispose { Servers.open = null }
    }
    LaunchedEffect(messages.size) {
        Servers.markRead(channel.key)
        if (messages.isNotEmpty()) list.animateScrollToItem(messages.lastIndex + 1)
    }

    val present = channel.members.count { !it.hasLeft }
    Page(
        title = channel.name,
        subtitle = "${server?.label.orEmpty()} · $present ${if (present == 1) "member" else "members"}",
        onBack = back,
        scrolls = false,
        action = {
            IconButton(onClick = { go(Screen.ChannelDetails(channel.key)) }) { Icon(Icons.Filled.Info, contentDescription = "Channel details") }
        },
        footer = {
            Column {
                HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
                Row(Modifier.fillMaxWidth().padding(start = 12.dp, end = 8.dp, top = 6.dp, bottom = 8.dp), verticalAlignment = Alignment.CenterVertically) {
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
            items(messages) { message -> Bubble(message) }
            item { Box(Modifier.size(4.dp)) }
        }
    }
}

@Composable
private fun Bubble(message: MessageUi) {
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
        Text(
            message.text,
            Modifier.widthIn(max = 280.dp).clip(RoundedCornerShape(14.dp))
                .background(if (message.mine) MaterialTheme.colorScheme.primaryContainer else MaterialTheme.colorScheme.surfaceContainerHigh)
                .padding(horizontal = 11.dp, vertical = 8.dp),
            fontSize = 15.sp,
        )
        Text(shortTime(message.sent), fontSize = 12.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

// ---------------------------------------------------------------- channel details (mockup screen 3)

@Composable
fun ChannelDetailsScreen(channel: ChannelUi, server: ServerUi?, back: () -> Unit, go: (Screen) -> Unit, home: () -> Unit) {
    val runner = rememberRunner()
    var requests by remember { mutableIntStateOf(0) }
    var invite by remember { mutableStateOf<String?>(null) }
    var confirming by remember { mutableStateOf(false) }

    if (channel.ownedByMe) {
        LaunchedEffect(channel.key, channel.members.size) {
            requests = runCatching { Servers.joinRequests(channel.slot, channel.id).size }.getOrDefault(0)
        }
    }
    invite?.let { link ->
        LinkDialog(
            title = "Invite to ${channel.name}",
            note = "Send this to one person who is already on ${server?.label ?: "the server"}. It works once, for a day. They ask to join; you then let them in.",
            link = link,
            onClose = { invite = null },
        )
    }
    if (confirming) {
        if (channel.ownedByMe) {
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
        Item("Channel name", value = channel.name)
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
            Item("Invite someone", detail = "Make a link to send them", onClick = {
                runner.run { invite = Servers.inviteToChannel(channel.slot, channel.id) }
            })
        }
        Heading(" ")
        if (channel.ownedByMe) {
            Item("Delete channel", detail = "Removes it for everyone. Cannot be undone.", danger = true, onClick = { confirming = true })
        } else {
            Item("Leave channel", danger = true, onClick = { confirming = true })
        }
        // Not yet here: renaming, changing your name in the channel, safety numbers.
    }
}

@Composable
fun MembersScreen(channel: ChannelUi, back: () -> Unit) {
    val runner = rememberRunner()
    var removing by remember { mutableStateOf<String?>(null) }
    removing?.let { device ->
        val name = channel.members.firstOrNull { it.device == device }?.name ?: device
        Confirm("Remove $name?", "They stop receiving messages at once, and cannot read anything sent from now on.", "Remove", onNo = { removing = null }, onYes = {
            removing = null
            runner.run { Servers.removeMember(channel.slot, channel.id, device) }
        })
    }
    Page(title = "Members", subtitle = channel.name, onBack = back) {
        channel.members.forEach { member ->
            Item(
                member.name,
                detail = when {
                    member.hasLeft -> "Has left"
                    member.device == channel.owner -> "Owner"
                    else -> null
                },
                end = {
                    if (channel.ownedByMe && member.device != channel.owner && !member.hasLeft) {
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
    LaunchedEffect(channel.key, reload) {
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
    val choices = if (channel.maxAge in AGES) AGES else (AGES + channel.maxAge).sorted()
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
fun JoinChannelScreen(initialLink: String, back: () -> Unit) {
    val runner = rememberRunner()
    var link by rememberSaveable { mutableStateOf(initialLink) }
    var myName by rememberSaveable { mutableStateOf("") }
    var asked by remember { mutableStateOf(false) }
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
        Note("Paste the invite link the channel's owner sent you.")
        OutlinedTextField(link, { link = it }, Modifier.fillMaxWidth(), label = { Text("Invite link") }, minLines = 3, maxLines = 5)
        if (problem != null) Text(problem, Modifier.padding(top = 8.dp), fontSize = 13.sp, color = MaterialTheme.colorScheme.error)
        OutlinedTextField(myName, { myName = it.take(32) }, Modifier.fillMaxWidth().padding(top = 10.dp), label = { Text("Your name in this channel") }, singleLine = true)
        Note("The app checks that the channel really belongs to the person the invite came from before it asks.")
    }
}
