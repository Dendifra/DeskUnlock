package com.sy.syauth.android.bg

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.util.Log

public class PackageReplacedReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != Intent.ACTION_MY_PACKAGE_REPLACED) return

        Log.i(
            "syauth.bg.package",
            "package replaced: restoring background service",
        )

        val started = resurrectIfDead(context.applicationContext)

        Log.i(
            "syauth.bg.package",
            "package replaced: resurrect dispatched=$started",
        )
    }
}
