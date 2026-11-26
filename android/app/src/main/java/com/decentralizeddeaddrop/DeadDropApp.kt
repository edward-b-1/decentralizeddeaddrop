package com.decentralizeddeaddrop

import android.app.Application
import com.decentralizeddeaddrop.data.Servers

class DeadDropApp : Application() {
    override fun onCreate() {
        super.onCreate()
        Servers.start(this)
    }
}
