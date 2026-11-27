package com.decentralizeddeaddrop.data

import android.content.ContentValues
import com.decentralizeddeaddrop.keys.Vault
import net.zetetic.database.sqlcipher.SQLiteDatabase
import org.json.JSONObject
import java.io.File
import java.security.SecureRandom

/**
 * Message history: one database for every server on this phone, encrypted as a whole
 * (SQLCipher), so that the file shows nothing without its key: not the messages, not who
 * wrote them, not when, not how many there are.
 *
 * The database has its own random key. That key is kept in the [Vault], sealed with a key
 * the phone's secure hardware holds, and is in the app's memory only while the app runs.
 *
 * Each message is a row, so adding one writes one row, a conversation can be read a page
 * at a time, and a single message can be deleted. Deleted rows are overwritten
 * (`secure_delete`), not just unlinked.
 */
class MessageDb(directory: File, vault: Vault) {
    private val db: SQLiteDatabase

    init {
        System.loadLibrary("sqlcipher")
        val file = File(directory, FILE)
        var key = vault.read(KEY_FILE)
        if (key == null || key.size != KEY_BYTES) {
            // No key means nothing already here could be opened: start afresh.
            wipe(file)
            key = ByteArray(KEY_BYTES).also { SecureRandom().nextBytes(it) }
            vault.write(KEY_FILE, key)
        }
        // The x'…' form hands SQLCipher the key itself, so it does not stretch a
        // password (the key is already random).
        val password = "x'" + key.joinToString("") { "%02x".format(it) } + "'"
        db = runCatching { open(file, password) }.getOrElse {
            // A damaged file, or one sealed with a lost key. Its contents are gone either way.
            wipe(file)
            open(file, password)
        }
    }

    private fun open(file: File, password: String): SQLiteDatabase {
        val opened = SQLiteDatabase.openOrCreateDatabase(file, password, null, null, null)
        try {
            opened.rawQuery("PRAGMA secure_delete = ON", null).use { it.moveToFirst() }
            if (opened.version == 0) {
                opened.beginTransaction()
                try {
                    opened.execSQL(
                        "CREATE TABLE messages (id INTEGER PRIMARY KEY AUTOINCREMENT, slot INTEGER NOT NULL, channel TEXT NOT NULL, " +
                            "mine INTEGER NOT NULL, name TEXT NOT NULL, body TEXT NOT NULL, sent INTEGER NOT NULL, unreadable INTEGER NOT NULL)",
                    )
                    opened.execSQL("CREATE INDEX messages_by_channel ON messages (slot, channel, id)")
                    opened.execSQL("CREATE TABLE unread (slot INTEGER NOT NULL, channel TEXT NOT NULL, count INTEGER NOT NULL, PRIMARY KEY (slot, channel))")
                    opened.version = 1
                    opened.setTransactionSuccessful()
                } finally {
                    opened.endTransaction()
                }
            } else {
                // Proves the key fits: a wrong key fails here, not on the pragma above.
                opened.rawQuery("SELECT count(*) FROM messages", null).use { it.moveToFirst() }
            }
        } catch (failure: Throwable) {
            opened.close()
            throw failure
        }
        return opened
    }

    private fun wipe(file: File) {
        for (suffix in listOf("", "-journal", "-wal", "-shm")) File(file.path + suffix).delete()
    }

    /** Stores a message and returns it with its place in the history. */
    @Synchronized
    fun add(slot: Int, channel: String, message: MessageUi): MessageUi {
        val id = db.insertOrThrow("messages", null, row(slot, channel, message))
        return message.copy(id = id)
    }

    /** Up to [limit] messages of a channel that come before [before], oldest first. */
    // Values are handed to rawQuery one by one: it takes a variable number of them, and an
    // array handed over whole would count as a single value.
    @Synchronized
    fun page(slot: Int, channel: String, before: Long = Long.MAX_VALUE, limit: Int): List<MessageUi> {
        val found = ArrayList<MessageUi>()
        db.rawQuery(
            "SELECT id, mine, name, body, sent, unreadable FROM messages WHERE slot = ? AND channel = ? AND id < ? ORDER BY id DESC LIMIT ?",
            slot, channel, before, limit,
        ).use { rows ->
            while (rows.moveToNext()) {
                found += MessageUi(
                    id = rows.getLong(0),
                    mine = rows.getInt(1) != 0,
                    name = rows.getString(2),
                    text = rows.getString(3),
                    sent = rows.getLong(4),
                    unreadable = rows.getInt(5) != 0,
                )
            }
        }
        return found.asReversed()
    }

    /** Every channel of a server that has history here, with its count of unread messages. */
    @Synchronized
    fun channels(slot: Int): Map<String, Int> {
        val found = HashMap<String, Int>()
        db.rawQuery("SELECT DISTINCT channel FROM messages WHERE slot = ?", slot).use { rows ->
            while (rows.moveToNext()) found[rows.getString(0)] = 0
        }
        db.rawQuery("SELECT channel, count FROM unread WHERE slot = ?", slot).use { rows ->
            while (rows.moveToNext()) found[rows.getString(0)] = rows.getInt(1)
        }
        return found
    }

    @Synchronized
    fun setUnread(slot: Int, channel: String, count: Int) {
        db.execSQL("INSERT OR REPLACE INTO unread (slot, channel, count) VALUES (?, ?, ?)", arrayOf<Any>(slot, channel, count))
    }

    @Synchronized
    fun forgetChannel(slot: Int, channel: String) {
        db.execSQL("DELETE FROM messages WHERE slot = ? AND channel = ?", arrayOf<Any>(slot, channel))
        db.execSQL("DELETE FROM unread WHERE slot = ? AND channel = ?", arrayOf<Any>(slot, channel))
    }

    @Synchronized
    fun forgetServer(slot: Int) {
        db.execSQL("DELETE FROM messages WHERE slot = ?", arrayOf<Any>(slot))
        db.execSQL("DELETE FROM unread WHERE slot = ?", arrayOf<Any>(slot))
    }

    /**
     * Takes in a server's history as versions up to 0.1.0 kept it: one file holding
     * every channel. All of it goes in, or none of it.
     */
    @Synchronized
    fun takeOld(slot: Int, channels: JSONObject) {
        db.beginTransaction()
        try {
            for (channel in channels.keys()) {
                val saved = channels.getJSONObject(channel)
                val list = saved.getJSONArray("messages")
                for (index in 0 until list.length()) {
                    val message = list.getJSONObject(index)
                    db.insertOrThrow(
                        "messages",
                        null,
                        row(
                            slot,
                            channel,
                            MessageUi(
                                mine = message.getBoolean("mine"),
                                name = message.getString("name"),
                                text = message.getString("text"),
                                sent = message.getLong("sent"),
                                unreadable = message.optBoolean("unreadable"),
                            ),
                        ),
                    )
                }
                db.execSQL("INSERT OR REPLACE INTO unread (slot, channel, count) VALUES (?, ?, ?)", arrayOf<Any>(slot, channel, saved.optInt("unread")))
            }
            db.setTransactionSuccessful()
        } finally {
            db.endTransaction()
        }
    }

    private fun row(slot: Int, channel: String, message: MessageUi) = ContentValues().apply {
        put("slot", slot)
        put("channel", channel)
        put("mine", if (message.mine) 1 else 0)
        put("name", message.name)
        put("body", message.text)
        put("sent", message.sent)
        put("unreadable", if (message.unreadable) 1 else 0)
    }

    private companion object {
        const val FILE = "messages.db"
        const val KEY_FILE = "messages.key"
        const val KEY_BYTES = 32
    }
}
