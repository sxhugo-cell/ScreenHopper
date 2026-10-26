package com.screenhopper.app

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.system.Os
import androidx.activity.enableEdgeToEdge
import androidx.core.content.ContextCompat

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    // The Rust side names this device after the phone model; the env var must exist before the library starts.
    try { Os.setenv("SCREEN_HOPPER_DEVICE_NAME", Build.MODEL ?: "Android", true) } catch (_: Exception) {}
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    Bridge.attach(this)
    androidx.core.view.WindowCompat.getInsetsController(window, window.decorView).isAppearanceLightStatusBars = false
    if (Build.VERSION.SDK_INT >= 33 &&
        checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
      requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 4100)
    }
    ContextCompat.startForegroundService(this, Intent(this, KeepAliveService::class.java))
  }

  override fun onResume() {
    super.onResume()
    Bridge.reportScreen()
    // Android only lets the app in front read the clipboard, so share anything new each time we come forward.
    window.decorView.post { Bridge.onResume() }
  }

  @Deprecated("Deprecated in Java")
  override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
    super.onActivityResult(requestCode, resultCode, data)
    when (requestCode) {
      Bridge.REQ_PICK -> if (resultCode == RESULT_OK) Bridge.onPicked(data) else Bridge.onPickCancelled()
    }
  }
}
