package com.decentralizeddeaddrop

import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.runtime.mutableStateOf
import com.decentralizeddeaddrop.ui.DeadDropTheme
import com.decentralizeddeaddrop.ui.DeadDropUi

class MainActivity : ComponentActivity() {
    /** A deaddrop:// link the app was opened with, until a screen has dealt with it. */
    private val openedLink = mutableStateOf<String?>(null)

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        openedLink.value = intent?.dataString
        setContent {
            DeadDropTheme {
                DeadDropUi(openedLink = openedLink.value, onLinkHandled = { openedLink.value = null })
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        openedLink.value = intent.dataString
    }
}
