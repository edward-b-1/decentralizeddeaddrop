package com.decentralizeddeaddrop.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.decentralizeddeaddrop.data.ChannelUi
import com.decentralizeddeaddrop.data.Connection
import com.decentralizeddeaddrop.data.ServerUi
import com.decentralizeddeaddrop.data.Servers

/** Where the app can be. The screens follow docs/design/app-2026-11-26-a.html. */
sealed interface Screen {
    data object Home : Screen
    data class Conversation(val key: String) : Screen
    data class ChannelDetails(val key: String) : Screen
    data class Members(val key: String) : Screen
    data class JoinRequests(val key: String) : Screen
    data class MaxAge(val key: String) : Screen
    data object NewChannel : Screen
    data class JoinChannel(val link: String) : Screen
    data class AddServer(val link: String) : Screen
    data class Server(val slot: Int) : Screen
}

@Composable
fun DeadDropUi(openedLink: String?, onLinkHandled: () -> Unit) {
    val servers by Servers.servers.collectAsStateWithLifecycle()
    // Kept here so that coming back from another screen returns to the same tab.
    var tab by rememberSaveable { mutableIntStateOf(0) }
    val stack = remember { mutableStateListOf<Screen>(Screen.Home) }
    val back: () -> Unit = { if (stack.size > 1) stack.removeAt(stack.lastIndex) }
    val go: (Screen) -> Unit = { stack.add(it) }
    BackHandler(enabled = stack.size > 1, onBack = back)

    // A link the app was opened with: a channel invite, or a server to add.
    LaunchedEffect(openedLink) {
        if (openedLink != null) {
            val isChannelInvite = com.decentralizeddeaddrop.engine.readLink(openedLink)?.channel != null
            go(if (isChannelInvite) Screen.JoinChannel(openedLink) else Screen.AddServer(openedLink))
            onLinkHandled()
        }
    }

    val channel: (String) -> ChannelUi? = { key -> servers.flatMap { it.channels }.firstOrNull { it.key == key } }
    val server: (Int) -> ServerUi? = { slot -> servers.firstOrNull { it.slot == slot } }

    Surface(Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
        Box(Modifier.fillMaxSize().safeDrawingPadding()) {
            when (val screen = stack.last()) {
                Screen.Home -> Home(servers, tab, { tab = it }, go)
                is Screen.Conversation -> channel(screen.key).orBack(back) { ConversationScreen(it, server(it.slot), back, go) }
                is Screen.ChannelDetails -> channel(screen.key).orBack(back) { ChannelDetailsScreen(it, server(it.slot), back, go) { stack.removeRange(1, stack.size) } }
                is Screen.Members -> channel(screen.key).orBack(back) { MembersScreen(it, back) }
                is Screen.JoinRequests -> channel(screen.key).orBack(back) { JoinRequestsScreen(it, back) }
                is Screen.MaxAge -> channel(screen.key).orBack(back) { MaxAgeScreen(it, back) }
                Screen.NewChannel -> NewChannelScreen(servers, back) { key ->
                    back()
                    go(Screen.Conversation(key))
                }
                is Screen.JoinChannel -> JoinChannelScreen(screen.link, back)
                is Screen.AddServer -> AddServerScreen(screen.link, back)
                is Screen.Server -> server(screen.slot).orBack(back) { ServerScreen(it, back) }
            }
        }
    }
}

/** Shows a screen for something that may have gone away (a channel left, a server removed). */
@Composable
private fun <T> T?.orBack(back: () -> Unit, content: @Composable (T) -> Unit) {
    if (this == null) LaunchedEffect(Unit) { back() } else content(this)
}

@Composable
private fun Home(servers: List<ServerUi>, tab: Int, onTab: (Int) -> Unit, go: (Screen) -> Unit) {
    Column(Modifier.fillMaxSize()) {
        // Tabs at the top, as in Pocket Radio.
        Row(Modifier.fillMaxWidth().background(MaterialTheme.colorScheme.surfaceVariant).padding(top = 6.dp)) {
            listOf("Channels", "Servers", "Settings").forEachIndexed { index, name ->
                val selected = tab == index
                Column(Modifier.weight(1f).clickable { onTab(index) }, horizontalAlignment = Alignment.CenterHorizontally) {
                    Text(
                        name,
                        Modifier.padding(vertical = 10.dp),
                        fontSize = 14.sp,
                        fontWeight = if (selected) FontWeight.Medium else FontWeight.Normal,
                        color = if (selected) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Box(Modifier.fillMaxWidth().height(3.dp).background(if (selected) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.surfaceVariant))
                }
            }
        }
        when (tab) {
            0 -> ChannelsTab(servers, go)
            1 -> ServersTab(servers, go)
            else -> SettingsTab()
        }
    }
}

@Composable
fun ServerHeader(server: ServerUi) {
    val signals = LocalSignals.current
    Row(Modifier.fillMaxWidth().padding(top = 18.dp, bottom = 2.dp), verticalAlignment = Alignment.CenterVertically) {
        Light(if (server.connection == Connection.Connected) signals.ok else signals.waiting)
        Text(server.label, Modifier.padding(start = 8.dp).weight(1f), fontSize = 13.5.sp, fontWeight = FontWeight.Medium, maxLines = 1, overflow = TextOverflow.Ellipsis)
        when (server.connection) {
            // A server with no name of its own is already labelled with its address.
            Connection.Connected -> if (server.label != server.address) {
                Text(server.address, fontSize = 12.5.sp, fontFamily = FontFamily.Monospace, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            Connection.Reconnecting -> Text("Reconnecting…", fontSize = 12.5.sp, color = signals.waiting)
            Connection.AwaitingApproval -> Text("Waiting for approval", fontSize = 12.5.sp, color = signals.waiting)
        }
    }
}

@Composable
private fun ChannelsTab(servers: List<ServerUi>, go: (Screen) -> Unit) {
    Column(Modifier.fillMaxSize()) {
        Column(Modifier.weight(1f).fillMaxWidth().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp)) {
            if (servers.isEmpty()) {
                Spacer(Modifier.height(32.dp))
                Text("No servers yet", fontSize = 17.sp, fontWeight = FontWeight.Medium)
                Note("Dead Drop has no central service. To begin, add a server that you or someone you know runs.")
                Primary("Add a server", Modifier.padding(top = 8.dp)) { go(Screen.AddServer("")) }
            }
            servers.forEach { server ->
                ServerHeader(server)
                server.channels.forEach { channel ->
                    Row(
                        Modifier.fillMaxWidth().clickable { go(Screen.Conversation(channel.key)) }.padding(vertical = 11.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Column(Modifier.weight(1f)) {
                            Text(channel.name, fontSize = 15.sp, fontWeight = FontWeight.Medium, maxLines = 1, overflow = TextOverflow.Ellipsis)
                            Text(
                                channel.lastLine.ifEmpty { "No messages yet" },
                                fontSize = 13.5.sp,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                            if (channel.asking > 0) {
                                Text(
                                    if (channel.asking == 1) "Someone is asking to join" else "${channel.asking} people are asking to join",
                                    fontSize = 13.sp,
                                    color = MaterialTheme.colorScheme.primary,
                                )
                            }
                        }
                        Column(horizontalAlignment = Alignment.End, verticalArrangement = Arrangement.spacedBy(4.dp)) {
                            Text(shortTime(channel.lastTime), fontSize = 12.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
                            if (channel.unread > 0) Badge(channel.unread)
                        }
                    }
                    HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
                }
                server.asked.forEach { _ ->
                    Column(Modifier.fillMaxWidth().padding(vertical = 11.dp)) {
                        Text("A channel you asked to join", fontSize = 15.sp, fontWeight = FontWeight.Medium)
                        Text("Waiting for the owner to let you in", fontSize = 13.5.sp, fontStyle = FontStyle.Italic, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                    HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
                }
                if (server.channels.isEmpty() && server.asked.isEmpty()) Note("No channels on this server yet.")
            }
        }
        if (servers.isNotEmpty()) {
            Footer {
                Secondary("Join by invite", Modifier.weight(1f)) { go(Screen.JoinChannel("")) }
                Primary("New channel", Modifier.weight(1f)) { go(Screen.NewChannel) }
            }
        }
    }
}

@Composable
private fun ServersTab(servers: List<ServerUi>, go: (Screen) -> Unit) {
    val signals = LocalSignals.current
    Column(Modifier.fillMaxSize()) {
        Column(Modifier.weight(1f).fillMaxWidth().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp)) {
            if (servers.isEmpty()) Note("This phone is not on any server yet.", Modifier.padding(top = 16.dp))
            servers.forEach { server ->
                Item(
                    title = server.label,
                    detail = when (server.connection) {
                        // A server with no name of its own is already titled with its address.
                        Connection.Connected -> listOfNotNull(
                            server.address.takeIf { it != server.label },
                            "you run this server".takeIf { server.operator },
                            when (server.waiting.size) {
                                0 -> null
                                1 -> "1 device waiting for approval"
                                else -> "${server.waiting.size} devices waiting for approval"
                            },
                        ).joinToString(" · ").replaceFirstChar { it.uppercase() }.ifEmpty { "Connected" }
                        Connection.Reconnecting -> "Reconnecting…"
                        Connection.AwaitingApproval -> "Waiting for the person who runs it to approve this phone"
                    },
                    onClick = { go(Screen.Server(server.slot)) },
                    end = { Light(if (server.connection == Connection.Connected) signals.ok else signals.waiting) },
                )
            }
        }
        Footer { Primary("Add a server", Modifier.weight(1f)) { go(Screen.AddServer("")) } }
    }
}

@Composable
private fun SettingsTab() {
    Column(Modifier.fillMaxSize().padding(horizontal = 16.dp)) {
        Heading("About")
        Item("Decentralized Dead Drop", detail = "Secure communication systems.")
        Item("Version", value = "0.1.0")
        Note("Messages are sealed on this phone and can be opened only by the members of a channel. Servers hold them only until they are delivered or too old, and can never read them.")
        Note("This is an early version. It has had no independent security review.")
    }
}
