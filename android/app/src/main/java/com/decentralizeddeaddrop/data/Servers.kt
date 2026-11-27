package com.decentralizeddeaddrop.data

import android.content.Context
import com.decentralizeddeaddrop.engine.DeviceInfo
import com.decentralizeddeaddrop.engine.EngineException
import com.decentralizeddeaddrop.engine.JoinRequestInfo
import com.decentralizeddeaddrop.engine.News
import com.decentralizeddeaddrop.engine.Session
import com.decentralizeddeaddrop.engine.StateStore
import com.decentralizeddeaddrop.engine.WaysInInfo
import com.decentralizeddeaddrop.engine.readLink
import com.decentralizeddeaddrop.keys.DeviceKeys
import com.decentralizeddeaddrop.keys.Vault
import com.decentralizeddeaddrop.service.Notifier
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.util.concurrent.ConcurrentHashMap
import kotlin.concurrent.thread

data class MemberUi(
    val device: String,
    val name: String,
    val hasLeft: Boolean,
    /** This phone's own entry. */
    val isMe: Boolean,
    /** The person holding this phone has compared safety numbers with this member. */
    val verified: Boolean,
)

data class ChannelUi(
    val slot: Int,
    val id: String,
    val name: String,
    /** Device ID of the owner. */
    val owner: String,
    val ownedByMe: Boolean,
    /** Maximum message age in seconds. */
    val maxAge: Long,
    val members: List<MemberUi>,
    val lastLine: String,
    val lastTime: Long,
    val unread: Int,
    /** For the owner: how many people are asking to join. */
    val asking: Int,
    /** This phone's name in the channel. */
    val myName: String,
) {
    val key: String get() = "$slot/$id"
}

enum class Connection { Connected, Reconnecting, AwaitingApproval }

data class ServerUi(
    val slot: Int,
    val label: String,
    /** This phone's device ID on this server. */
    val device: String,
    val address: String,
    val fingerprint: String,
    val operator: Boolean,
    val connection: Connection,
    val channels: List<ChannelUi>,
    /** Channels asked to join, waiting for their owners. */
    val asked: List<String>,
    /** For the person who runs the server: devices waiting to be approved. */
    val waiting: List<String>,
)

data class MessageUi(
    /** Its place in this phone's history. 0 until it has been stored. */
    val id: Long = 0,
    val mine: Boolean,
    val name: String,
    val text: String,
    /** Unix seconds. */
    val sent: Long,
    /** A message that arrived but could not be opened. */
    val unreadable: Boolean = false,
)

/**
 * Every server this phone is on. Each has its own identity, its own engine session and
 * its own background thread that collects messages while the app is running.
 *
 * Everything here that talks to a server blocks, so it runs off the main thread.
 */
object Servers {
    val servers = MutableStateFlow<List<ServerUi>>(emptyList())

    /** Message history by [ChannelUi.key]. Kept on this phone only. */
    val messages = MutableStateFlow<Map<String, List<MessageUi>>>(emptyMap())

    /** The channels ([ChannelUi.key]) that have older messages than the ones in [messages]. */
    val earlier = MutableStateFlow<Set<String>>(emptySet())

    /** A channel's messages as far back as they have been read in; [earlier] says the database holds older ones. */
    private class History(var unread: Int = 0, val messages: MutableList<MessageUi> = mutableListOf(), @Volatile var earlier: Boolean = false)

    private class Entry(val slot: Int, var label: String, val keys: DeviceKeys, val session: Session, val store: Store) {
        @Volatile var connection = Connection.Reconnecting
        @Volatile var stopped = false
        val history = ConcurrentHashMap<String, History>()

        /** People asking to join each channel this phone owns. */
        val asking = ConcurrentHashMap<String, Int>()

        /** Devices waiting for approval, if this phone runs the server. */
        @Volatile var waiting: List<String> = emptyList()

        /** Devices whose safety number the person holding this phone has compared. */
        val verified: MutableSet<String> = ConcurrentHashMap.newKeySet()
    }

    /** Writes the engine's state the moment the engine asks, before it goes on. */
    private class Store(private val slot: Int) : StateStore {
        /** Set when the server is taken off the phone, so nothing is written once its files are gone. */
        @Volatile var closed = false

        override fun save(state: ByteArray) {
            if (!closed) vault.write(stateFile(slot), state)
        }
    }

    private val entries = ConcurrentHashMap<Int, Entry>()
    private lateinit var vault: Vault
    private lateinit var app: Context

    /** Message history. Opened the first time it is needed, which is never on the main thread. */
    private val db: MessageDb by lazy { MessageDb(File(app.filesDir, "vault"), vault) }

    /** True once the servers stored on this phone have been read back in. */
    val loaded = MutableStateFlow(false)

    /** The channel on screen right now, whose messages do not count as unread. */
    @Volatile var open: String? = null

    private fun stateFile(slot: Int) = "server-$slot.state"
    private fun historyFile(slot: Int) = "server-$slot.history"

    fun start(context: Context) {
        if (::vault.isInitialized) return
        app = context.applicationContext
        vault = Vault(File(context.filesDir, "vault"))
        thread(name = "servers-start") {
            val registry = vault.read(REGISTRY)?.let { JSONArray(String(it)) } ?: JSONArray()
            for (index in 0 until registry.length()) {
                val saved = registry.getJSONObject(index)
                val slot = saved.getInt("slot")
                val state = vault.read(stateFile(slot)) ?: continue
                val keys = DeviceKeys(slot)
                val store = Store(slot)
                val session = runCatching { Session.restore(state, keys, store) }.getOrNull() ?: continue
                val entry = Entry(slot, saved.getString("label"), keys, session, store)
                saved.optJSONArray("verified")?.let { list ->
                    for (at in 0 until list.length()) entry.verified += list.getString(at)
                }
                loadHistory(entry)
                entries[slot] = entry
                listen(entry)
            }
            publish()
            loaded.value = true
        }
    }

    // ------------------------------------------------------------------ what the screens show

    // One at a time, so that an older picture can never replace a newer one.
    @Synchronized
    private fun publish() {
        val shown = entries.values.sortedBy { it.slot }.map { entry ->
            val session = entry.session
            // These wait while the engine is busy with the server, so the connection
            // state is read after them, not before.
            val address = session.address()
            val channels = session.channels()
            val me = session.device()
            ServerUi(
                slot = entry.slot,
                label = entry.label,
                device = me,
                address = address,
                fingerprint = session.fingerprint(),
                operator = session.isOperator(),
                connection = entry.connection,
                channels = channels.sortedBy { it.name.lowercase() }.map { channel ->
                    val history = entry.history[channel.id]
                    val last = history?.messages?.lastOrNull()
                    ChannelUi(
                        slot = entry.slot,
                        id = channel.id,
                        name = channel.name,
                        owner = channel.owner,
                        ownedByMe = channel.ownedByMe,
                        maxAge = channel.maxAge.toLong(),
                        members = channel.members.map {
                            MemberUi(it.device, it.name, it.hasLeft, isMe = it.device == me, verified = it.device in entry.verified)
                        },
                        myName = channel.members.firstOrNull { it.device == me }?.name.orEmpty(),
                        lastLine = when {
                            last == null -> ""
                            last.unreadable -> "A message could not be read"
                            last.mine -> "You: ${last.text}"
                            else -> "${last.name}: ${last.text}"
                        },
                        lastTime = last?.sent ?: 0,
                        unread = history?.unread ?: 0,
                        asking = if (channel.ownedByMe) entry.asking[channel.id] ?: 0 else 0,
                    )
                },
                asked = session.asked(),
                waiting = entry.waiting,
            )
        }
        servers.value = shown
        messages.value = entries.values.flatMap { entry ->
            entry.history.map { (channel, history) -> "${entry.slot}/$channel" to synchronized(history) { history.messages.toList() } }
        }.toMap()
        earlier.value = entries.values.flatMap { entry ->
            entry.history.filterValues { it.earlier }.keys.map { "${entry.slot}/$it" }
        }.toSet()
    }

    private fun record(entry: Entry, channel: String, message: MessageUi) {
        val history = entry.history.getOrPut(channel) { History() }
        synchronized(history) {
            history.messages += db.add(entry.slot, channel, message)
            // Read at once only if its channel is on screen and the screen is being looked at.
            if (!message.mine && (open != "${entry.slot}/$channel" || !Notifier.appVisible)) {
                history.unread += 1
                db.setUnread(entry.slot, channel, history.unread)
            }
        }
    }

    fun markRead(key: String) {
        val (slot, channel) = key.split("/", limit = 2).let { it[0].toInt() to it[1] }
        val entry = entries[slot] ?: return
        val history = entry.history[channel] ?: return
        Notifier.clear(app, key)
        if (history.unread == 0) return
        history.unread = 0
        thread {
            db.setUnread(slot, channel, 0)
            publish()
        }
    }

    private fun loadHistory(entry: Entry) {
        // Versions up to 0.1.0 kept a server's whole history in one file: take it in, once.
        vault.read(historyFile(entry.slot))?.let { old ->
            if (runCatching { db.takeOld(entry.slot, JSONObject(String(old))) }.isSuccess) vault.delete(historyFile(entry.slot))
        }
        for ((channel, unread) in db.channels(entry.slot)) {
            entry.history[channel] = History(unread).also { read(entry.slot, channel, it) }
        }
    }

    /** Puts the page of messages before the ones already in memory at the front of them. */
    private fun read(slot: Int, channel: String, history: History) = synchronized(history) {
        val before = history.messages.firstOrNull()?.id ?: Long.MAX_VALUE
        val page = db.page(slot, channel, before, PAGE + 1)
        history.earlier = page.size > PAGE
        history.messages.addAll(0, page.takeLast(PAGE))
    }

    /** Brings the next page of a channel's older messages into [messages]. */
    suspend fun loadEarlier(key: String) = withContext(Dispatchers.IO) {
        val (slot, channel) = key.split("/", limit = 2).let { it[0].toInt() to it[1] }
        val history = entries[slot]?.history?.get(channel) ?: return@withContext
        read(slot, channel, history)
        publish()
    }

    private fun saveRegistry() {
        val registry = JSONArray(
            entries.values.sortedBy { it.slot }.map {
                JSONObject().put("slot", it.slot).put("label", it.label).put("verified", JSONArray(it.verified.sorted()))
            },
        )
        vault.write(REGISTRY, registry.toString().toByteArray())
    }

    // ------------------------------------------------------------------ collecting messages

    private fun take(entry: Entry, news: List<News>) {
        for (item in news) {
            when (item) {
                is News.Message -> {
                    val name = item.senderName ?: item.sender
                    record(entry, item.channel, MessageUi(mine = false, name = name, text = item.text, sent = item.sent))
                    val key = "${entry.slot}/${item.channel}"
                    if (open != key || !Notifier.appVisible) {
                        val channel = servers.value.flatMap { it.channels }.firstOrNull { it.key == key }?.name ?: "A channel"
                        Notifier.message(app, key, channel, name, item.text, entry.history[item.channel]?.unread ?: 1)
                    }
                }
                is News.Unreadable -> record(
                    entry,
                    item.channel,
                    MessageUi(mine = false, name = "", text = "", sent = System.currentTimeMillis() / 1000, unreadable = true),
                )
                is News.ChannelsChanged -> {}
            }
        }
    }

    /**
     * What is waiting on the person holding this phone: people asking to join channels
     * they own, and devices to approve if they run the server. The server says when
     * either changes, so this follows every sync.
     */
    private fun lookAround(entry: Entry) {
        val session = entry.session
        val owned = session.channels().filter { it.ownedByMe }.map { it.id }
        entry.asking.keys.retainAll(owned.toSet())
        for (channel in owned) {
            runCatching { session.joinRequests(channel).size }.onSuccess { entry.asking[channel] = it }
        }
        if (session.isOperator()) runCatching { session.waitingDevices() }.onSuccess { entry.waiting = it }
    }

    /** Collects what is waiting, then waits for the server to say there is more. */
    private fun listen(entry: Entry) {
        thread(name = "server-${entry.slot}") {
            // After a failure, wait longer each time before trying again (up to a minute),
            // so that a server that is down, or a phone with no signal, costs little battery.
            var pause = FIRST_RETRY_MS
            while (!entry.stopped) {
                try {
                    take(entry, entry.session.sync())
                    lookAround(entry)
                    entry.connection = Connection.Connected
                    pause = FIRST_RETRY_MS
                    publish()
                    while (!entry.stopped && !entry.session.waitForNews(25u)) {
                        // Quiet. Keep waiting.
                    }
                } catch (waiting: EngineException.AwaitingApproval) {
                    entry.connection = Connection.AwaitingApproval
                    publish()
                    Thread.sleep(10_000)
                } catch (failure: EngineException) {
                    entry.connection = Connection.Reconnecting
                    publish()
                    Thread.sleep(pause)
                    pause = (pause * 2).coerceAtMost(LONGEST_RETRY_MS)
                }
            }
        }
    }

    // ------------------------------------------------------------------ what the screens do

    private fun entry(slot: Int): Entry = entries[slot] ?: error("That server is no longer on this phone.")

    /** Runs engine work off the main thread and refreshes what the screens show. */
    private suspend fun <T> on(slot: Int, work: (Session) -> T): T = withContext(Dispatchers.IO) {
        val result = runCatching { work(entry(slot).session) }
        publish()
        result.getOrThrow()
    }

    /** Joins the server a link points to, as a new identity. Returns its slot. */
    suspend fun addServer(link: String, password: String?, label: String): Int = withContext(Dispatchers.IO) {
        val info = readLink(link) ?: throw IllegalArgumentException("That is not a complete Dead Drop link.")
        entries.values.firstOrNull { it.session.fingerprint() == info.fingerprint }?.let {
            throw IllegalArgumentException("This phone is already on that server (${it.label}).")
        }
        val slot = (entries.keys.maxOrNull() ?: 0) + 1
        val keys = DeviceKeys(slot)
        val store = Store(slot)
        val session = try {
            Session.join(link, password?.takeIf { it.isNotEmpty() }, keys, store)
        } catch (failure: Exception) {
            store.closed = true
            keys.destroy()
            vault.delete(stateFile(slot))
            // The server says only that it did not let the phone in; say which way in that was.
            if (failure is EngineException.Refused && failure.message == "not accepted") {
                throw IllegalStateException(
                    when {
                        info.hasServerToken -> "The server did not accept that invite. It may have been used already or have run out."
                        !password.isNullOrEmpty() -> "The server did not accept that password."
                        else -> "The server did not let this phone in. It needs a password, or an invite link from the person who runs it."
                    },
                )
            }
            throw failure
        }
        val entry = Entry(slot, label.ifBlank { "${info.host}:${info.port}" }, keys, session, store)
        entries[slot] = entry
        saveRegistry()
        listen(entry)
        publish()
        slot
    }

    /**
     * Asks whatever answers at an address typed by hand for its fingerprint. It means
     * nothing until the person has compared it with what the server's operator gave them.
     */
    suspend fun askFingerprint(host: String, port: Int): String = withContext(Dispatchers.IO) {
        com.decentralizeddeaddrop.engine.askFingerprint(host, port.toUShort())
    }

    /** Joins a server whose address was typed by hand, once the person has checked its fingerprint. */
    suspend fun addTyped(host: String, port: Int, fingerprint: String, token: String?, password: String?, label: String): Int {
        val link = com.decentralizeddeaddrop.engine.linkFromParts(host, port.toUShort(), fingerprint, token)
            ?: throw IllegalArgumentException("That address or token cannot be used. Check what you typed.")
        return addServer(link, password, label)
    }

    /** The server a channel invite is for, if this phone is on it. */
    fun serverFor(link: String): Int? {
        val info = readLink(link) ?: return null
        return servers.value.firstOrNull { it.fingerprint == info.fingerprint }?.slot
    }

    /**
     * Takes this server off the phone: its identity, its keys and its history are
     * destroyed. The server is told first, so that nothing goes on waiting there for a
     * phone that will never collect it; if it cannot be reached, the phone forgets it
     * all the same.
     */
    suspend fun forgetServer(slot: Int) = withContext(Dispatchers.IO) {
        val entry = entries[slot] ?: return@withContext
        runCatching { entry.session.leaveServer() }
        entries.remove(slot)
        entry.stopped = true
        entry.store.closed = true
        entry.keys.destroy()
        vault.delete(stateFile(slot))
        vault.delete(historyFile(slot))
        db.forgetServer(slot)
        saveRegistry()
        publish()
    }

    suspend fun createChannel(slot: Int, name: String, myName: String, maxAge: Long): String =
        on(slot) { it.createChannel(name, myName, maxAge.toULong()) }

    suspend fun askToJoin(slot: Int, link: String, myName: String) = on(slot) { it.askToJoin(link, myName) }

    suspend fun inviteToChannel(slot: Int, channel: String): String = on(slot) { it.inviteToChannel(channel) }

    suspend fun joinRequests(slot: Int, channel: String): List<JoinRequestInfo> = on(slot) { session ->
        session.joinRequests(channel).also { entry(slot).asking[channel] = it.size }
    }

    suspend fun admit(slot: Int, channel: String, device: String) = on(slot) {
        it.admit(channel, device)
        entry(slot).asking[channel] = it.joinRequests(channel).size
    }

    suspend fun removeMember(slot: Int, channel: String, device: String) = on(slot) { it.removeMember(channel, device) }

    suspend fun setMaxAge(slot: Int, channel: String, seconds: Long) = on(slot) { it.setMaxAge(channel, seconds.toULong()) }

    suspend fun renameChannel(slot: Int, channel: String, name: String) = on(slot) { it.renameChannel(channel, name) }

    /** True if the name has changed already; false if the channel's owner has been asked. */
    suspend fun setMyName(slot: Int, channel: String, name: String): Boolean = on(slot) { it.setMyName(channel, name) }

    suspend fun safetyNumber(slot: Int, device: String): String = on(slot) { it.safetyNumber(device) }

    /** Records that the person holding this phone has, or has not, compared safety numbers with a device. */
    suspend fun setVerified(slot: Int, device: String, verified: Boolean) = withContext(Dispatchers.IO) {
        val entry = entry(slot)
        if (verified) entry.verified += device else entry.verified -= device
        saveRegistry()
        publish()
    }

    suspend fun leave(slot: Int, channel: String) = on(slot) {
        it.leave(channel)
        forgetHistory(slot, channel)
    }

    suspend fun deleteChannel(slot: Int, channel: String) = on(slot) {
        it.deleteChannel(channel)
        forgetHistory(slot, channel)
    }

    private fun forgetHistory(slot: Int, channel: String) {
        val entry = entry(slot)
        entry.history.remove(channel)
        db.forgetChannel(slot, channel)
    }

    /** Sends a message. It appears in the conversation at once; the engine keeps it safe until the server has it. */
    suspend fun send(slot: Int, channel: String, text: String) = withContext(Dispatchers.IO) {
        val entry = entry(slot)
        entry.session.send(channel, text)
        record(entry, channel, MessageUi(mine = true, name = "You", text = text, sent = System.currentTimeMillis() / 1000))
        publish()
    }

    // For the phone that runs a server.
    suspend fun inviteToServer(slot: Int): String = on(slot) { it.inviteToServer() }
    suspend fun waysIn(slot: Int): WaysInInfo = on(slot) { it.waysIn() }
    suspend fun setWaysIn(slot: Int, open: Boolean, tokens: Boolean, approval: Boolean) = on(slot) { it.setWaysIn(open, tokens, approval) }
    suspend fun setServerPassword(slot: Int, password: String?) = on(slot) { it.setServerPassword(password) }
    suspend fun serverLink(slot: Int): String = on(slot) { it.serverLink() }
    suspend fun devices(slot: Int): List<DeviceInfo> = on(slot) { it.devices() }
    suspend fun removeDevice(slot: Int, device: String) = on(slot) {
        it.removeDevice(device)
        entry(slot).waiting = it.waitingDevices()
    }

    suspend fun approveDevice(slot: Int, device: String) = on(slot) {
        it.approveDevice(device)
        entry(slot).waiting = it.waitingDevices()
    }

    private const val REGISTRY = "servers"

    /** How many of a channel's messages are read in at a time. */
    private const val PAGE = 200
    private const val FIRST_RETRY_MS = 5_000L
    private const val LONGEST_RETRY_MS = 60_000L
}
