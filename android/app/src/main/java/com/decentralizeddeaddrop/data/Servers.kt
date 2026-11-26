package com.decentralizeddeaddrop.data

import android.content.Context
import com.decentralizeddeaddrop.engine.EngineException
import com.decentralizeddeaddrop.engine.JoinRequestInfo
import com.decentralizeddeaddrop.engine.News
import com.decentralizeddeaddrop.engine.Session
import com.decentralizeddeaddrop.engine.StateStore
import com.decentralizeddeaddrop.engine.WaysInInfo
import com.decentralizeddeaddrop.engine.readLink
import com.decentralizeddeaddrop.keys.DeviceKeys
import com.decentralizeddeaddrop.keys.Vault
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.util.concurrent.ConcurrentHashMap
import kotlin.concurrent.thread

data class MemberUi(val device: String, val name: String, val hasLeft: Boolean)

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
) {
    val key: String get() = "$slot/$id"
}

enum class Connection { Connected, Reconnecting, AwaitingApproval }

data class ServerUi(
    val slot: Int,
    val label: String,
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

    private class History(var unread: Int = 0, val messages: MutableList<MessageUi> = mutableListOf())

    private class Entry(val slot: Int, var label: String, val keys: DeviceKeys, val session: Session, val store: Store) {
        @Volatile var connection = Connection.Reconnecting
        @Volatile var stopped = false
        val history = ConcurrentHashMap<String, History>()

        /** People asking to join each channel this phone owns. */
        val asking = ConcurrentHashMap<String, Int>()

        /** Devices waiting for approval, if this phone runs the server. */
        @Volatile var waiting: List<String> = emptyList()
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

    /** The channel on screen right now, whose messages do not count as unread. */
    @Volatile var open: String? = null

    private fun stateFile(slot: Int) = "server-$slot.state"
    private fun historyFile(slot: Int) = "server-$slot.history"

    fun start(context: Context) {
        if (::vault.isInitialized) return
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
                loadHistory(entry)
                entries[slot] = entry
                listen(entry)
            }
            publish()
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
            ServerUi(
                slot = entry.slot,
                label = entry.label,
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
                        members = channel.members.map { MemberUi(it.device, it.name, it.hasLeft) },
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
            entry.history.map { (channel, history) -> "${entry.slot}/$channel" to history.messages.toList() }
        }.toMap()
    }

    private fun record(entry: Entry, channel: String, message: MessageUi) {
        val history = entry.history.getOrPut(channel) { History() }
        synchronized(history) {
            history.messages += message
            if (!message.mine && open != "${entry.slot}/$channel") history.unread += 1
        }
        saveHistory(entry)
    }

    fun markRead(key: String) {
        val (slot, channel) = key.split("/", limit = 2).let { it[0].toInt() to it[1] }
        val entry = entries[slot] ?: return
        val history = entry.history[channel] ?: return
        if (history.unread == 0) return
        history.unread = 0
        thread {
            saveHistory(entry)
            publish()
        }
    }

    private fun saveHistory(entry: Entry) {
        val channels = JSONObject()
        for ((channel, history) in entry.history) {
            synchronized(history) {
                channels.put(
                    channel,
                    JSONObject().put("unread", history.unread).put(
                        "messages",
                        JSONArray(history.messages.map {
                            JSONObject().put("mine", it.mine).put("name", it.name).put("text", it.text)
                                .put("sent", it.sent).put("unreadable", it.unreadable)
                        }),
                    ),
                )
            }
        }
        vault.write(historyFile(entry.slot), channels.toString().toByteArray())
    }

    private fun loadHistory(entry: Entry) {
        val channels = vault.read(historyFile(entry.slot))?.let { JSONObject(String(it)) } ?: return
        for (channel in channels.keys()) {
            val saved = channels.getJSONObject(channel)
            val list = saved.getJSONArray("messages")
            val history = History(unread = saved.getInt("unread"))
            for (index in 0 until list.length()) {
                val message = list.getJSONObject(index)
                history.messages += MessageUi(
                    mine = message.getBoolean("mine"),
                    name = message.getString("name"),
                    text = message.getString("text"),
                    sent = message.getLong("sent"),
                    unreadable = message.optBoolean("unreadable"),
                )
            }
            entry.history[channel] = history
        }
    }

    private fun saveRegistry() {
        val registry = JSONArray(entries.values.sortedBy { it.slot }.map { JSONObject().put("slot", it.slot).put("label", it.label) })
        vault.write(REGISTRY, registry.toString().toByteArray())
    }

    // ------------------------------------------------------------------ collecting messages

    private fun take(entry: Entry, news: List<News>) {
        for (item in news) {
            when (item) {
                is News.Message -> record(
                    entry,
                    item.channel,
                    MessageUi(mine = false, name = item.senderName ?: item.sender, text = item.text, sent = item.sent),
                )
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
            while (!entry.stopped) {
                try {
                    take(entry, entry.session.sync())
                    lookAround(entry)
                    entry.connection = Connection.Connected
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
                    Thread.sleep(5_000)
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
        saveHistory(entry)
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
    suspend fun approveDevice(slot: Int, device: String) = on(slot) {
        it.approveDevice(device)
        entry(slot).waiting = it.waitingDevices()
    }

    private const val REGISTRY = "servers"
}
