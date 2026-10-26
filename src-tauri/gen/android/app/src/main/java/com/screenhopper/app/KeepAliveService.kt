package com.screenhopper.app

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.wifi.WifiManager
import android.os.Build
import android.os.IBinder

/**
 * Keeps Screen Hopper alive and listening while the app is in the background: a foreground service
 * with a notification, plus the Wi-Fi locks that stop Android from dozing the network radio
 * (and from dropping the multicast packets computers use to find each other).
 */
class KeepAliveService : Service() {
    private var multicast: WifiManager.MulticastLock? = null
    private var wifi: WifiManager.WifiLock? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val nm = getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(NotificationChannel("keepalive", "Screen Hopper running", NotificationManager.IMPORTANCE_LOW))
        val open = PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE)
        val n = Notification.Builder(this, "keepalive")
            .setContentTitle("Screen Hopper is running")
            .setContentText("Ready to share mouse, keyboard, clipboard and files")
            .setSmallIcon(R.mipmap.ic_launcher)
            .setContentIntent(open)
            .setOngoing(true)
            .build()
        if (Build.VERSION.SDK_INT >= 34) {
            startForeground(1, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE)
        } else {
            startForeground(1, n)
        }
        val wm = applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
        if (multicast == null) {
            multicast = wm.createMulticastLock("screenhopper-mdns").apply { setReferenceCounted(false); acquire() }
        }
        if (wifi == null) {
            val mode = if (Build.VERSION.SDK_INT >= 29) WifiManager.WIFI_MODE_FULL_LOW_LATENCY else WifiManager.WIFI_MODE_FULL_HIGH_PERF
            wifi = wm.createWifiLock(mode, "screenhopper-wifi").apply { setReferenceCounted(false); acquire() }
        }
        return START_STICKY
    }

    override fun onDestroy() {
        multicast?.let { if (it.isHeld) it.release() }
        wifi?.let { if (it.isHeld) it.release() }
        super.onDestroy()
    }
}
