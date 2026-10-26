package com.screenhopper.app

import android.app.Activity
import android.content.ClipData
import android.content.ClipboardManager
import android.content.ContentValues
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.provider.MediaStore
import android.provider.OpenableColumns
import android.provider.Settings
import android.util.Log
import androidx.annotation.Keep
import java.io.File
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/**
 * The single object Rust talks to. Rust calls the instance methods below (via JNI, on the object
 * passed to nativeInit); Kotlin reports back through the static native* functions.
 */
@Keep
object Bridge {
    init {
        System.loadLibrary("screen_hopper_lib")
    }

    // ---- Kotlin -> Rust (implemented in src/platform/android.rs) ----
    @JvmStatic external fun nativeInit(bridge: Any, model: String)
    @JvmStatic external fun nativeScreen(wDp: Float, hDp: Float)
    @JvmStatic external fun nativeA11y(on: Boolean)
    @JvmStatic external fun nativeClipboard(text: String)

    const val REQ_PICK = 4101

    @Volatile var activity: Activity? = null
    lateinit var app: Context
    private var lastReportedClip: String? = null

    fun attach(a: Activity) {
        activity = a
        app = a.applicationContext
        nativeInit(this, Build.MODEL ?: "Android")
        reportScreen()
    }

    /** The accessibility service (re)connected: make sure Rust has the app context even if the UI never opened. */
    fun attachService(svc: android.content.Context) {
        if (!::app.isInitialized) {
            app = svc.applicationContext
            nativeInit(this, Build.MODEL ?: "Android")
        }
    }

    fun reportScreen() {
        val m = (if (::app.isInitialized) app else return).resources.displayMetrics
        nativeScreen(m.widthPixels / m.density, m.heightPixels / m.density)
    }

    private val density: Float get() = app.resources.displayMetrics.density

    // ---------------------------------------------------------------- pointer / keyboard
    fun cursorTo(xDp: Float, yDp: Float) {
        HopperService.instance?.cursorTo(xDp * density, yDp * density)
    }

    fun hideCursor() {
        HopperService.instance?.hideCursor()
    }

    fun button(button: Int, down: Boolean, xDp: Float, yDp: Float) {
        HopperService.instance?.button(button, down, xDp * density, yDp * density)
    }

    fun wheel(dx: Int, dy: Int) {
        HopperService.instance?.wheel(dx, dy)
    }

    fun key(hid: Int, down: Boolean) {
        HopperService.instance?.key(hid, down)
    }

    fun resetKeys() {
        HopperService.instance?.resetKeys()
    }

    fun openAccessibilitySettings() {
        val i = Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        app.startActivity(i)
    }

    // ---------------------------------------------------------------- keyboard
    private val imeId: String get() = "${app.packageName}/.HopperIME"

    /** "active" (our keyboard is selected), "enabled" (turned on but another keyboard is selected), or "off". */
    fun keyboardState(): String {
        val cur = Settings.Secure.getString(app.contentResolver, Settings.Secure.DEFAULT_INPUT_METHOD) ?: ""
        if (cur.startsWith(app.packageName)) return "active"
        val imm = app.getSystemService(Context.INPUT_METHOD_SERVICE) as android.view.inputmethod.InputMethodManager
        return if (imm.enabledInputMethodList.any { it.packageName == app.packageName }) "enabled" else "off"
    }

    fun openKeyboardSettings() {
        app.startActivity(Intent(Settings.ACTION_INPUT_METHOD_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
    }

    fun pickKeyboard() {
        activity?.runOnUiThread {
            (app.getSystemService(Context.INPUT_METHOD_SERVICE) as android.view.inputmethod.InputMethodManager).showInputMethodPicker()
        }
    }

    private var previousIme: String? = null

    /**
     * If the user granted WRITE_SECURE_SETTINGS once over adb, switch to our keyboard while a computer is
     * driving this phone and back afterwards. Without that grant the user picks the keyboard by hand.
     */
    fun imeAutoSwitch(on: Boolean) {
        try {
            val cr = app.contentResolver
            if (on) {
                val cur = Settings.Secure.getString(cr, Settings.Secure.DEFAULT_INPUT_METHOD) ?: return
                if (cur.startsWith(app.packageName)) return
                previousIme = cur
                Settings.Secure.putString(cr, Settings.Secure.DEFAULT_INPUT_METHOD, imeId)
            } else {
                previousIme?.let { Settings.Secure.putString(cr, Settings.Secure.DEFAULT_INPUT_METHOD, it) }
                previousIme = null
            }
        } catch (_: SecurityException) {
        } catch (_: Exception) {
        }
    }

    // ---------------------------------------------------------------- clipboard
    private val clipboard: ClipboardManager get() = app.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager

    fun setClipboard(text: String): Boolean {
        lastReportedClip = text
        clipboard.setPrimaryClip(ClipData.newPlainText("Screen Hopper", text))
        return true
    }

    /** Only works while the app is in front: Android hides the clipboard from background apps. */
    fun readClipboard(): String? {
        val c = clipboard.primaryClip ?: return null
        if (c.itemCount == 0) return null
        return c.getItemAt(0).coerceToText(app)?.toString()
    }

    /** Called when the app comes to the front: share anything copied since last time. */
    fun onResume() {
        val t = try { readClipboard() } catch (e: Exception) { null } ?: return
        if (t.isNotEmpty() && t != lastReportedClip) {
            lastReportedClip = t
            nativeClipboard(t)
        }
    }

    // ---------------------------------------------------------------- files
    @Volatile private var pickLatch: CountDownLatch? = null
    @Volatile private var pickResult: Array<String> = emptyArray()

    /** Blocks (on a Rust worker thread) until the system picker closes; returns private copies. */
    fun pickFiles(): Array<String> {
        val a = activity ?: return emptyArray()
        val latch = CountDownLatch(1)
        pickLatch = latch
        pickResult = emptyArray()
        a.runOnUiThread {
            val i = Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
                addCategory(Intent.CATEGORY_OPENABLE)
                type = "*/*"
                putExtra(Intent.EXTRA_ALLOW_MULTIPLE, true)
            }
            a.startActivityForResult(i, REQ_PICK)
        }
        latch.await(10, TimeUnit.MINUTES)
        return pickResult
    }

    fun onPicked(data: Intent?) {
        Thread {
            val uris = ArrayList<Uri>()
            data?.clipData?.let { cd -> for (i in 0 until cd.itemCount) uris.add(cd.getItemAt(i).uri) }
            data?.data?.let { if (uris.isEmpty()) uris.add(it) }
            val dir = File(app.cacheDir, "send").apply { deleteRecursively(); mkdirs() }
            val out = ArrayList<String>()
            for (u in uris) {
                try {
                    var name = "file"
                    app.contentResolver.query(u, null, null, null, null)?.use { c ->
                        val idx = c.getColumnIndex(OpenableColumns.DISPLAY_NAME)
                        if (c.moveToFirst() && idx >= 0) name = c.getString(idx) ?: name
                    }
                    val f = File(dir, name.replace('/', '_'))
                    app.contentResolver.openInputStream(u)?.use { i -> f.outputStream().use { o -> i.copyTo(o) } }
                    out.add(f.absolutePath)
                } catch (e: Exception) {
                    Log.w("ScreenHopper", "pick copy failed: $e")
                }
            }
            pickResult = out.toTypedArray()
            pickLatch?.countDown()
        }.start()
    }

    fun onPickCancelled() {
        pickResult = emptyArray()
        pickLatch?.countDown()
    }

    /** Copy a received file or folder into Downloads/Screen Hopper (MediaStore, no permission needed). */
    fun publishToDownloads(path: String): Boolean {
        return try {
            publishTree(File(path), "Download/Screen Hopper")
            File(path).deleteRecursively()
            true
        } catch (e: Exception) {
            Log.w("ScreenHopper", "publish failed: $e")
            false
        }
    }

    private fun publishTree(f: File, rel: String) {
        if (f.isDirectory) {
            f.listFiles()?.forEach { publishTree(it, "$rel/${f.name}") }
            return
        }
        val mime = android.webkit.MimeTypeMap.getSingleton()
            .getMimeTypeFromExtension(f.extension.lowercase()) ?: "application/octet-stream"
        // If a file with this name is already recorded (even a stale record), retry as "name (2).ext", "name (3).ext"...
        var lastError: Exception? = null
        for (n in 1..30) {
            val name = if (n == 1) f.name else "${f.nameWithoutExtension} ($n)${if (f.extension.isEmpty()) "" else "." + f.extension}"
            val values = ContentValues().apply {
                put(MediaStore.MediaColumns.DISPLAY_NAME, name)
                put(MediaStore.MediaColumns.MIME_TYPE, mime)
                put(MediaStore.MediaColumns.RELATIVE_PATH, rel)
            }
            try {
                val uri = app.contentResolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values)
                    ?: throw IllegalStateException("MediaStore insert failed")
                app.contentResolver.openOutputStream(uri)?.use { o -> f.inputStream().use { it.copyTo(o) } }
                return
            } catch (e: Exception) {
                lastError = e
            }
        }
        throw lastError ?: IllegalStateException("publish failed")
    }

    fun openDownloads() {
        app.startActivity(Intent(DownloadManagerView.ACTION).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
    }
    private object DownloadManagerView { const val ACTION = "android.intent.action.VIEW_DOWNLOADS" }
}
