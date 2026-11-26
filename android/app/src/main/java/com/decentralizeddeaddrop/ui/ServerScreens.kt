package com.decentralizeddeaddrop.ui

import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.decentralizeddeaddrop.data.Connection
import com.decentralizeddeaddrop.data.ServerUi
import com.decentralizeddeaddrop.data.Servers
import com.decentralizeddeaddrop.engine.WaysInInfo
import com.decentralizeddeaddrop.engine.readLink

// ---------------------------------------------------------------- add a server (mockup screen 5)

@Composable
fun AddServerScreen(initialLink: String, back: () -> Unit) {
    val runner = rememberRunner()
    var link by rememberSaveable { mutableStateOf(initialLink) }
    var label by rememberSaveable { mutableStateOf("") }
    var password by rememberSaveable { mutableStateOf("") }
    val info = readLink(link.trim())
    Page(
        title = "Add a server",
        onBack = back,
        footer = {
            Footer {
                Primary("Connect", Modifier.weight(1f), enabled = info != null && !runner.busy) {
                    runner.run(then = back) { Servers.addServer(link.trim(), password, label.trim()) }
                }
            }
        },
    ) {
        Note("Paste the link you were given by the person who runs the server. It holds the server's address, its fingerprint and your way in.")
        OutlinedTextField(link, { link = it }, Modifier.fillMaxWidth(), label = { Text("Server link") }, minLines = 3, maxLines = 5)
        when {
            link.isNotBlank() && info == null -> Text(
                "That is not a complete Dead Drop link.",
                Modifier.padding(top = 8.dp),
                fontSize = 13.sp,
                color = MaterialTheme.colorScheme.error,
            )
            info != null -> {
                Heading("This link is for")
                Item("Address", value = "${info.host} : ${info.port}")
                Text("Fingerprint", Modifier.padding(top = 11.dp), fontSize = 15.sp)
                Fingerprint(info.fingerprint)
                Note("The app connects only to the server with this fingerprint. If anyone else answers at that address, it refuses.")
                if (!info.hasServerToken) {
                    OutlinedTextField(
                        password,
                        { password = it },
                        Modifier.fillMaxWidth().padding(top = 4.dp),
                        label = { Text("Server password, if it has one") },
                        singleLine = true,
                    )
                }
                OutlinedTextField(
                    label,
                    { label = it.take(40) },
                    Modifier.fillMaxWidth().padding(top = 10.dp),
                    label = { Text("What to call this server (optional)") },
                    singleLine = true,
                )
                Note("You get a new identity for each server, so two servers cannot tell that the same phone uses both.")
            }
        }
        // Not yet here: scanning a QR code, and typing an address by hand.
    }
}

// ---------------------------------------------------------------- a server (mockup screen 7)

@Composable
fun ServerScreen(server: ServerUi, back: () -> Unit) {
    val runner = rememberRunner()
    var ways by remember { mutableStateOf<WaysInInfo?>(null) }
    var waiting by remember { mutableStateOf<List<String>>(emptyList()) }
    var reload by remember { mutableIntStateOf(0) }
    var invite by remember { mutableStateOf<String?>(null) }
    var settingPassword by remember { mutableStateOf(false) }
    var forgetting by remember { mutableStateOf(false) }

    if (server.operator && server.connection == Connection.Connected) {
        LaunchedEffect(server.slot, reload) {
            ways = runCatching { Servers.waysIn(server.slot) }.getOrNull()
            waiting = runCatching { Servers.waitingDevices(server.slot) }.getOrDefault(emptyList())
        }
    }
    invite?.let { link ->
        LinkDialog(
            title = "Invite a device",
            note = "Send this to one person. It lets one new phone onto this server and works once, for a day.",
            link = link,
            onClose = { invite = null },
        )
    }
    if (settingPassword) {
        PasswordDialog(
            alreadySet = ways?.passwordSet == true,
            onClose = { settingPassword = false },
            onSet = { password ->
                settingPassword = false
                runner.run(then = { reload += 1 }) { Servers.setServerPassword(server.slot, password) }
            },
        )
    }
    if (forgetting) {
        Confirm(
            "Remove ${server.label} from this phone?",
            "Your identity on it, your keys for it and your message history from it are destroyed. You would need a new invite to come back, and would be a new device to everyone there.",
            "Remove",
            onNo = { forgetting = false },
            onYes = {
                forgetting = false
                runner.run(then = back) { Servers.forgetServer(server.slot) }
            },
        )
    }

    val signals = LocalSignals.current
    Page(title = server.label, subtitle = if (server.operator) "You run this server" else null, onBack = back) {
        Item(
            when (server.connection) {
                Connection.Connected -> "Connected"
                Connection.Reconnecting -> "Reconnecting…"
                Connection.AwaitingApproval -> "Waiting for approval"
            },
            detail = server.address,
            end = { Light(if (server.connection == Connection.Connected) signals.ok else signals.waiting) },
        )
        Text("Fingerprint", Modifier.padding(top = 11.dp), fontSize = 15.sp)
        Fingerprint(server.fingerprint)
        Note("This is how phones recognise the server. Read it out to someone to let them check they have the right one.")

        if (server.operator) {
            waiting.forEach { device ->
                Item("Waiting for approval", detail = "Device $device", highlight = true, end = {
                    Primary("Approve", enabled = !runner.busy) {
                        runner.run(then = { reload += 1 }) { Servers.approveDevice(server.slot, device) }
                    }
                })
            }
            Heading("Ways in")
            val current = ways
            if (current == null) {
                Note(if (server.connection == Connection.Connected) "Looking…" else "Shown when the server is reachable.")
            } else {
                val set: (Boolean, Boolean, Boolean) -> Unit = { open, tokens, approval ->
                    runner.run(then = { reload += 1 }) { Servers.setWaysIn(server.slot, open, tokens, approval) }
                }
                Item("Invite tokens", detail = "From a link you make", end = {
                    Switch(current.tokens, { set(current.open, it, current.approval) }, enabled = !runner.busy)
                })
                Item(
                    "Server password",
                    detail = if (current.passwordSet) "Set. Anyone who knows it can come in." else "Not set",
                    onClick = { settingPassword = true },
                )
                Item("Open to anyone", detail = "No token or password needed", end = {
                    Switch(current.open, { set(it, current.tokens, current.approval) }, enabled = !runner.busy)
                })
                Item("Approve new devices", detail = "Each one waits for you, whichever way it came in", end = {
                    Switch(current.approval, { set(current.open, current.tokens, it) }, enabled = !runner.busy)
                })
                Primary("Invite a device", Modifier.fillMaxWidth().padding(top = 12.dp), enabled = !runner.busy) {
                    runner.run { invite = Servers.inviteToServer(server.slot) }
                }
            }
            Note("Running the server gives you no way into channels you are not a member of.")
        }
        Heading(" ")
        Item("Remove from this phone", danger = true, onClick = { forgetting = true })
    }
}

@Composable
private fun PasswordDialog(alreadySet: Boolean, onClose: () -> Unit, onSet: (String?) -> Unit) {
    var password by remember { mutableStateOf("") }
    AlertDialog(
        onDismissRequest = onClose,
        title = { Text("Server password", fontSize = 18.sp, fontWeight = FontWeight.Medium) },
        text = {
            OutlinedTextField(password, { password = it }, label = { Text("New password") }, singleLine = true, textStyle = androidx.compose.ui.text.TextStyle(fontFamily = FontFamily.Monospace))
        },
        confirmButton = { TextButton(onClick = { onSet(password) }, enabled = password.isNotEmpty()) { Text("Set") } },
        dismissButton = {
            if (alreadySet) {
                TextButton(onClick = { onSet(null) }) { Text("Remove password", color = MaterialTheme.colorScheme.error) }
            } else {
                TextButton(onClick = onClose) { Text("Cancel") }
            }
        },
    )
}
