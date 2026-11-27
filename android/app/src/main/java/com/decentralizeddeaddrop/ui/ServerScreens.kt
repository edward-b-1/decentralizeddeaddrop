package com.decentralizeddeaddrop.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
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
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.decentralizeddeaddrop.data.Connection
import com.decentralizeddeaddrop.data.ServerUi
import com.decentralizeddeaddrop.data.Servers
import com.decentralizeddeaddrop.engine.DeviceInfo
import com.decentralizeddeaddrop.engine.WaysInInfo
import com.decentralizeddeaddrop.engine.readLink

// ---------------------------------------------------------------- add a server (mockup screen 5)

private val WAYS_IN = listOf("Invite token", "Password", "None")

/** An invite token as typed by hand: spaces and dashes dropped, small letters. */
private fun typedToken(text: String): String = text.filter { !it.isWhitespace() && it != '-' }.lowercase()

@Composable
fun AddServerScreen(initialLink: String, back: () -> Unit, go: (Screen) -> Unit, openLink: (String) -> Unit) {
    val runner = rememberRunner()
    var scanning by rememberSaveable { mutableStateOf(false) }
    var link by rememberSaveable { mutableStateOf(initialLink) }
    var label by rememberSaveable { mutableStateOf("") }
    var password by rememberSaveable { mutableStateOf("") }
    // For an address typed by hand.
    var host by rememberSaveable { mutableStateOf("") }
    var port by rememberSaveable { mutableStateOf("7443") }
    var way by rememberSaveable { mutableIntStateOf(0) }
    var secret by rememberSaveable { mutableStateOf("") }

    if (scanning) {
        BackHandler { scanning = false }
        ScanScreen(
            "Scan a QR code",
            "Point the camera at the code for the server, or for the channel you were invited to.",
            back = { scanning = false },
        ) {
            scanning = false
            openLink(it)
        }
        return
    }

    val info = readLink(link.trim())
    val typing = link.isBlank()
    val hostOk = host.isNotBlank() && host.trim().all { it.isLetterOrDigit() || it in ".-:[]" }
    val portNumber = port.trim().toIntOrNull()?.takeIf { it in 1..65535 }
    val secretOk = when (way) {
        0 -> typedToken(secret).length == 32
        1 -> secret.isNotEmpty()
        else -> true
    }
    val typedOk = typing && hostOk && portNumber != null && secretOk

    Page(
        title = "Add a server",
        onBack = back,
        footer = {
            Footer {
                Primary("Connect", Modifier.weight(1f), enabled = (info != null || typedOk) && !runner.busy) {
                    if (info != null) {
                        runner.run(then = back) { Servers.addServer(link.trim(), password, label.trim()) }
                    } else {
                        go(
                            Screen.CheckServer(
                                host = host.trim(),
                                port = portNumber ?: 0,
                                token = typedToken(secret).takeIf { way == 0 },
                                password = secret.takeIf { way == 1 },
                                label = label.trim(),
                            ),
                        )
                    }
                }
            }
        },
    ) {
        Primary("Scan a QR code", Modifier.fillMaxWidth().padding(top = 14.dp)) { scanning = true }
        Note("A QR code or link from the person who runs the server holds its address, its fingerprint and your way in, so there is nothing to type.")
        OutlinedTextField(link, { link = it }, Modifier.fillMaxWidth(), label = { Text("Or paste a link") }, minLines = 2, maxLines = 5)
        when {
            !typing && info == null -> Text(
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
            }
            else -> {
                Heading("Or type it in")
                Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                    OutlinedTextField(host, { host = it }, Modifier.weight(1f), label = { Text("Address") }, singleLine = true)
                    OutlinedTextField(
                        port,
                        { port = it.filter(Char::isDigit).take(5) },
                        Modifier.width(110.dp),
                        label = { Text("Port") },
                        singleLine = true,
                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                    )
                }
                Text("Way in", Modifier.padding(top = 14.dp), fontSize = 15.sp)
                WAYS_IN.forEachIndexed { index, name ->
                    Row(
                        Modifier.fillMaxWidth().selectable(selected = way == index, role = Role.RadioButton, onClick = {
                            way = index
                            secret = ""
                        }),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        RadioButton(selected = way == index, onClick = null, modifier = Modifier.padding(end = 12.dp, top = 8.dp, bottom = 8.dp))
                        Text(name, fontSize = 15.sp)
                    }
                }
                if (way != 2) {
                    OutlinedTextField(
                        secret,
                        { secret = it },
                        Modifier.fillMaxWidth(),
                        label = { Text(if (way == 0) "Token" else "Server password") },
                        singleLine = true,
                        textStyle = TextStyle(fontFamily = FontFamily.Monospace),
                    )
                }
                Note("The person who runs the server tells you which of these it uses. The next screen shows the server's fingerprint for you to check with them.")
            }
        }
        if (info != null || typing) {
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
}

// ---------------------------------------------------------------- check the server (mockup screen 6)

/**
 * Shown only when the address was typed by hand, so there was no fingerprint to check
 * against. The app asks whatever answers there for its fingerprint, and the person
 * compares it with what the server's operator gave them.
 */
@Composable
fun CheckServerScreen(screen: Screen.CheckServer, back: () -> Unit, done: () -> Unit) {
    val runner = rememberRunner()
    var fingerprint by remember { mutableStateOf<String?>(null) }
    var failed by remember { mutableStateOf(false) }
    var attempt by remember { mutableIntStateOf(0) }
    LaunchedEffect(attempt) {
        failed = false
        fingerprint = null
        runCatching { Servers.askFingerprint(screen.host, screen.port) }.onSuccess { fingerprint = it }.onFailure { failed = true }
    }
    Page(
        title = "Check this server",
        subtitle = "${screen.host} : ${screen.port}",
        onBack = back,
        footer = {
            val found = fingerprint
            if (found != null) {
                Footer {
                    Secondary("It does not match", Modifier.weight(1f), onClick = back)
                    Primary("It matches", Modifier.weight(1f), enabled = !runner.busy) {
                        runner.run(then = done) {
                            Servers.addTyped(screen.host, screen.port, found, screen.token, screen.password, screen.label)
                        }
                    }
                }
            }
        },
    ) {
        val found = fingerprint
        when {
            failed -> {
                Text("Nothing answered", Modifier.padding(top = 18.dp), fontSize = 17.sp, fontWeight = FontWeight.Medium)
                Note("No Dead Drop server answered at that address. Check the address and the port, and that this phone is online.")
                Secondary("Try again", Modifier.padding(top = 8.dp)) { attempt += 1 }
            }
            found == null -> Note("Asking the server for its fingerprint…", Modifier.padding(top = 12.dp))
            else -> {
                Text("Is this the right server?", Modifier.padding(top = 18.dp), fontSize = 17.sp, fontWeight = FontWeight.Medium)
                Note("Compare this fingerprint with the one the person running the server gave you.")
                Fingerprint(found)
                Note("If every group matches, this is their server and nobody is in between.")
                Text(
                    "If anything differs, do not connect. Someone else is answering at this address.",
                    Modifier.padding(vertical = 6.dp),
                    fontSize = 13.sp,
                    lineHeight = 19.sp,
                    color = MaterialTheme.colorScheme.error,
                )
                Note("The app remembers the fingerprint, and from then on connects only to the server that has it.")
            }
        }
    }
}

// ---------------------------------------------------------------- a server (mockup screen 7)

@Composable
fun ServerScreen(server: ServerUi, back: () -> Unit, go: (Screen) -> Unit) {
    val runner = rememberRunner()
    var ways by remember { mutableStateOf<WaysInInfo?>(null) }
    var reload by remember { mutableIntStateOf(0) }
    var invite by remember { mutableStateOf<String?>(null) }
    var serverLink by remember { mutableStateOf<String?>(null) }
    var settingPassword by remember { mutableStateOf(false) }
    var forgetting by remember { mutableStateOf(false) }

    if (server.operator && server.connection == Connection.Connected) {
        LaunchedEffect(server.slot, reload) {
            ways = runCatching { Servers.waysIn(server.slot) }.getOrNull()
        }
    }
    invite?.let { link ->
        LinkDialog(
            title = "Invite a device",
            note = "Show or send this to one person. It lets one new phone onto this server and works once, for a day.",
            link = link,
            onClose = { invite = null },
        )
    }
    serverLink?.let { link ->
        LinkDialog(
            title = server.label,
            note = "This holds the server's address and fingerprint, and no way in. Whoever adds the server with it still needs the password or your approval, unless the server is open to anyone.",
            link = link,
            onClose = { serverLink = null },
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
            "Your identity on it, your keys for it and your message history from it are destroyed, and the server is told to forget this phone. " +
                "Channels you own there are deleted for everyone. You would need a new invite to come back, and would be a new device to everyone there." +
                if (server.operator) " Nobody will run this server until a new operator token is made on the machine it runs on." else "",
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
        Item("Show as a link or QR code", detail = "The address and fingerprint, with no way in", onClick = {
            runner.run { serverLink = Servers.serverLink(server.slot) }
        })
        Item("This phone's ID here", detail = server.device)

        if (server.operator) {
            server.waiting.forEach { device ->
                Item("Waiting for approval", detail = "Device $device", highlight = true, end = {
                    Primary("Approve", enabled = !runner.busy) {
                        runner.run { Servers.approveDevice(server.slot, device) }
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
                Item("Invite tokens", detail = "From a link or QR code you make", end = {
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
            Item("Devices on this server", detail = "See them, or remove one", onClick = { go(Screen.Devices(server.slot)) })
            Note("Running the server gives you no way into channels you are not a member of.")
        }
        Heading(" ")
        Item("Remove from this phone", danger = true, onClick = { forgetting = true })
    }
}

/** For the person who runs a server: every device on it, by ID. */
@Composable
fun DevicesScreen(server: ServerUi, back: () -> Unit) {
    val runner = rememberRunner()
    var devices by remember { mutableStateOf<List<DeviceInfo>?>(null) }
    var reload by remember { mutableIntStateOf(0) }
    var removing by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(server.slot, reload, server.waiting.size) {
        devices = runCatching { Servers.devices(server.slot) }.getOrNull()
    }
    removing?.let { device ->
        Confirm(
            "Remove device $device?",
            "It is taken off the server, with its place in every channel and any channel it owns. Whoever holds it would need a new way in to come back.",
            "Remove",
            onNo = { removing = null },
            onYes = {
                removing = null
                runner.run(then = { reload += 1 }) { Servers.removeDevice(server.slot, device) }
            },
        )
    }
    Page(title = "Devices", subtitle = server.label, onBack = back) {
        Note("The server knows devices by ID only: names exist inside channels, which the server cannot read. Someone can find their own ID under Servers, and a member's ID is shown with their safety number.")
        val found = devices
        if (found == null) Note(if (server.connection == Connection.Connected) "Looking…" else "Shown when the server is reachable.")
        found.orEmpty().forEach { device ->
            val mine = device.device == server.device
            Item(
                device.device,
                detail = when {
                    mine -> "This phone. It runs the server."
                    device.operator -> "Runs the server"
                    device.waiting -> "Waiting for approval"
                    else -> null
                },
                end = {
                    if (!mine) {
                        TextButton(onClick = { removing = device.device }, enabled = !runner.busy) {
                            Text("Remove", color = MaterialTheme.colorScheme.error)
                        }
                    }
                },
            )
        }
    }
}

@Composable
private fun PasswordDialog(alreadySet: Boolean, onClose: () -> Unit, onSet: (String?) -> Unit) {
    var password by remember { mutableStateOf("") }
    AlertDialog(
        onDismissRequest = onClose,
        title = { Text("Server password", fontSize = 18.sp, fontWeight = FontWeight.Medium) },
        text = {
            OutlinedTextField(password, { password = it }, label = { Text("New password") }, singleLine = true, textStyle = TextStyle(fontFamily = FontFamily.Monospace))
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
