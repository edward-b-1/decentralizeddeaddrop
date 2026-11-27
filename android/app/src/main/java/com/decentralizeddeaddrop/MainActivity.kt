package com.decentralizeddeaddrop

import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.runtime.mutableStateOf
import com.decentralizeddeaddrop.service.ConnectionService
import com.decentralizeddeaddrop.service.Notifier
import com.decentralizeddeaddrop.ui.DeadDropTheme
import com.decentralizeddeaddrop.ui.DeadDropUi

class MainActivity : ComponentActivity() {
    /** A deaddrop:// link the app was opened with, until a screen has dealt with it. */
    private val openedLink = mutableStateOf<String?>(null)

    /** The channel whose notification the app was opened from, until it is on screen. */
    private val openedChannel = mutableStateOf<String?>(null)

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        if (savedInstanceState == null) take(intent)
        setContent {
            DeadDropTheme {
                DeadDropUi(
                    openedLink = openedLink.value,
                    onLinkHandled = { openedLink.value = null },
                    openedChannel = openedChannel.value,
                    onChannelHandled = { openedChannel.value = null },
                )
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        take(intent)
    }

    private fun take(intent: Intent?) {
        intent?.dataString?.let { openedLink.value = it }
        intent?.getStringExtra(Notifier.OPEN_CHANNEL)?.let { openedChannel.value = it }
    }

    override fun onStart() {
        super.onStart()
        Notifier.appVisible = true
        // Android only lets the background connection be started while the app is in view.
        ConnectionService.update(this)
    }

    override fun onStop() {
        Notifier.appVisible = false
        super.onStop()
    }
}
