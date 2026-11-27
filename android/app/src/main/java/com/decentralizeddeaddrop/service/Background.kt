package com.decentralizeddeaddrop.service

import android.Manifest
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.os.IBinder
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import com.decentralizeddeaddrop.MainActivity
import com.decentralizeddeaddrop.R
import com.decentralizeddeaddrop.data.Connection
import com.decentralizeddeaddrop.data.ServerUi
import com.decentralizeddeaddrop.data.Servers
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.launch

/** The two choices on the Settings tab. Nothing secret is kept here. */
object Prefs {
    private const val FILE = "settings"
    private fun file(context: Context) = context.getSharedPreferences(FILE, Context.MODE_PRIVATE)

    /** Stay connected while the app is closed. */
    fun background(context: Context) = file(context).getBoolean("background", true)
    fun setBackground(context: Context, on: Boolean) = file(context).edit().putBoolean("background", on).apply()

    /** Show who wrote and what they wrote in notifications. Off: only that there is a message. */
    fun showText(context: Context) = file(context).getBoolean("show_text", false)
    fun setShowText(context: Context, on: Boolean) = file(context).edit().putBoolean("show_text", on).apply()

    /** Whether the person has been asked once to allow notifications. */
    fun askedNotifications(context: Context) = file(context).getBoolean("asked_notifications", false)
    fun setAskedNotifications(context: Context) = file(context).edit().putBoolean("asked_notifications", true).apply()
}

/** Notifications: one that stays while the app is connected in the background, and one per channel with new messages. */
object Notifier {
    private const val CONNECTION = "connection"
    private const val MESSAGES = "messages"
    const val CONNECTION_ID = 1

    /** Extra on the intent that opens the app from a message notification: the channel to show. */
    const val OPEN_CHANNEL = "open_channel"

    /** Whether the app's screens are showing. Messages that arrive then need no notification. */
    @Volatile var appVisible = false

    fun prepare(context: Context) {
        val manager = context.getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(
            NotificationChannel(CONNECTION, "Staying connected", NotificationManager.IMPORTANCE_LOW).apply {
                description = "Shown while Dead Drop keeps a connection open to your servers"
                setShowBadge(false)
            },
        )
        manager.createNotificationChannel(
            NotificationChannel(MESSAGES, "Messages", NotificationManager.IMPORTANCE_HIGH).apply {
                description = "New messages in your channels"
            },
        )
    }

    private fun openApp(context: Context, channel: String?): PendingIntent {
        val intent = Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)
        if (channel != null) intent.putExtra(OPEN_CHANNEL, channel)
        return PendingIntent.getActivity(
            context,
            channel?.hashCode() ?: 0,
            intent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
    }

    fun connection(context: Context, servers: List<ServerUi>): Notification {
        val connected = servers.count { it.connection == Connection.Connected }
        val text = when {
            servers.isEmpty() -> "No servers"
            connected == servers.size -> if (servers.size == 1) "Connected to ${servers[0].label}" else "Connected to ${servers.size} servers"
            servers.size == 1 -> "Reconnecting to ${servers[0].label}…"
            else -> "Connected to $connected of ${servers.size} servers"
        }
        return NotificationCompat.Builder(context, CONNECTION)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle("Dead Drop")
            .setContentText(text)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setShowWhen(false)
            .setContentIntent(openApp(context, null))
            .build()
    }

    /**
     * Says there is something new in a channel. What it shows is the person's choice:
     * by default nothing of the message, nor who sent it, nor which channel, since a
     * notification can be read on a locked phone.
     */
    fun message(context: Context, key: String, channelName: String, sender: String, text: String, unread: Int) {
        if (appVisible) return
        if (ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) return
        val count = if (unread == 1) "New message" else "$unread new messages"
        val builder = NotificationCompat.Builder(context, MESSAGES)
            .setSmallIcon(R.drawable.ic_notification)
            .setAutoCancel(true)
            .setCategory(NotificationCompat.CATEGORY_MESSAGE)
            .setContentIntent(openApp(context, key))
        if (Prefs.showText(context)) {
            builder.setContentTitle(channelName).setContentText("$sender: $text").setSubText(if (unread > 1) count else null)
            // On a locked phone, still only that there is something.
            builder.setPublicVersion(
                NotificationCompat.Builder(context, MESSAGES).setSmallIcon(R.drawable.ic_notification)
                    .setContentTitle("Dead Drop").setContentText(count).build(),
            )
        } else {
            builder.setContentTitle("Dead Drop").setContentText(count)
        }
        context.getSystemService(NotificationManager::class.java).notify(key.hashCode(), builder.build())
    }

    fun clear(context: Context, key: String) {
        context.getSystemService(NotificationManager::class.java).cancel(key.hashCode())
    }
}

/**
 * Keeps the app alive while it is closed, so that the connections to its servers stay
 * open and messages arrive (docs/PROTOCOL.md section 9). The connections themselves
 * belong to [Servers]; this only tells Android that the app is doing something the
 * person asked for, and shows that it is.
 *
 * Nothing goes through Google or any push service.
 */
class ConnectionService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        Notifier.prepare(this)
        Servers.start(applicationContext)
        show(Servers.servers.value)
        scope.launch {
            combine(Servers.servers, Servers.loaded) { servers, loaded -> servers to loaded }.collect { (servers, loaded) ->
                if (servers.isEmpty() && loaded) stopSelf() else show(servers)
            }
        }
    }

    private fun show(servers: List<ServerUi>) {
        runCatching {
            ServiceCompat.startForeground(this, Notifier.CONNECTION_ID, Notifier.connection(this, servers), ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE)
        }
    }

    // If Android stops the app to free memory, it starts this again when it can.
    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int = START_STICKY

    override fun onDestroy() {
        scope.cancel()
        super.onDestroy()
    }

    companion object {
        /** Starts or stops the service to match the setting and whether there are any servers. */
        fun update(context: Context) {
            val wanted = Prefs.background(context) && (Servers.servers.value.isNotEmpty() || !Servers.loaded.value)
            val intent = Intent(context, ConnectionService::class.java)
            // Android refuses to start this from the background in some states; the next
            // time the app is opened it is started again.
            runCatching { if (wanted) ContextCompat.startForegroundService(context, intent) else context.stopService(intent) }
        }
    }
}

/** Reconnects after the phone restarts, or after the app is updated, without the app being opened. */
class BootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action == Intent.ACTION_BOOT_COMPLETED || intent.action == Intent.ACTION_MY_PACKAGE_REPLACED) {
            ConnectionService.update(context)
        }
    }
}
