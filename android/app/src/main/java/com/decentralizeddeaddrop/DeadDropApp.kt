package com.decentralizeddeaddrop

import android.app.Application
import com.decentralizeddeaddrop.data.Servers
import com.decentralizeddeaddrop.service.ConnectionService
import com.decentralizeddeaddrop.service.Notifier
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.launch

class DeadDropApp : Application() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)

    override fun onCreate() {
        super.onCreate()
        Notifier.prepare(this)
        Servers.start(this)
        // Stay connected in the background exactly while there is a server to be connected to.
        scope.launch {
            combine(Servers.servers, Servers.loaded) { servers, loaded -> loaded && servers.isNotEmpty() }
                .distinctUntilChanged()
                .collect { ConnectionService.update(this@DeadDropApp) }
        }
    }
}
