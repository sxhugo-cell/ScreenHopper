package com.screenhopper.app

import android.accessibilityservice.AccessibilityService
import android.accessibilityservice.GestureDescription
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Path
import android.graphics.PixelFormat
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.view.Gravity
import android.view.KeyEvent
import android.view.View
import android.view.WindowManager
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import kotlin.math.abs
import kotlin.math.hypot
import kotlin.math.max
import kotlin.math.min

/**
 * Turns the mouse and keyboard of another computer into something Android understands:
 * a drawn pointer, taps, drags, scrolling swipes, Back/Home, and typing into the focused text field.
 */
class HopperService : AccessibilityService() {
    companion object {
        @Volatile var instance: HopperService? = null
    }

    private val ui = Handler(Looper.getMainLooper())
    private lateinit var wm: WindowManager
    private var cursor: CursorView? = null
    private var params: WindowManager.LayoutParams? = null
    private var curX = 0f
    private var curY = 0f
    private var cursorShown = false
    private var cursorDirty = false

    private val density get() = resources.displayMetrics.density
    private val screenW get() = resources.displayMetrics.widthPixels.toFloat()
    private val screenH get() = resources.displayMetrics.heightPixels.toFloat()

    override fun onServiceConnected() {
        super.onServiceConnected()
        instance = this
        wm = getSystemService(WINDOW_SERVICE) as WindowManager
        Bridge.attachService(this)
        Bridge.nativeA11y(true)
        Bridge.reportScreen()
    }

    override fun onAccessibilityEvent(event: AccessibilityEvent?) {}
    override fun onInterrupt() {}

    override fun onUnbind(intent: android.content.Intent?): Boolean {
        removeCursor()
        instance = null
        Bridge.nativeA11y(false)
        return super.onUnbind(intent)
    }

    override fun onConfigurationChanged(newConfig: android.content.res.Configuration) {
        super.onConfigurationChanged(newConfig)
        Bridge.reportScreen()
    }

    // ------------------------------------------------------------------ the pointer

    private class CursorView(c: android.content.Context) : View(c) {
        private val fill = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Color.WHITE; style = Paint.Style.FILL }
        private val edge = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Color.BLACK; style = Paint.Style.STROKE; strokeWidth = 2.2f * c.resources.displayMetrics.density; strokeJoin = Paint.Join.ROUND }
        private val shape = Path()
        override fun onDraw(canvas: Canvas) {
            val d = resources.displayMetrics.density
            shape.reset()
            shape.moveTo(2 * d, 2 * d)
            shape.lineTo(2 * d, 20 * d)
            shape.lineTo(7 * d, 15 * d)
            shape.lineTo(11 * d, 24 * d)
            shape.lineTo(14.5f * d, 22.5f * d)
            shape.lineTo(10.5f * d, 13.5f * d)
            shape.lineTo(17 * d, 13.5f * d)
            shape.close()
            canvas.drawPath(shape, fill)
            canvas.drawPath(shape, edge)
        }
        override fun onMeasure(w: Int, h: Int) {
            val d = resources.displayMetrics.density
            setMeasuredDimension((22 * d).toInt(), (28 * d).toInt())
        }
    }

    private fun ensureCursor() {
        if (cursor != null) return
        val v = CursorView(this)
        val lp = WindowManager.LayoutParams(
            WindowManager.LayoutParams.WRAP_CONTENT,
            WindowManager.LayoutParams.WRAP_CONTENT,
            WindowManager.LayoutParams.TYPE_ACCESSIBILITY_OVERLAY,
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or
                WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE or
                WindowManager.LayoutParams.FLAG_LAYOUT_IN_SCREEN or
                WindowManager.LayoutParams.FLAG_LAYOUT_NO_LIMITS,
            PixelFormat.TRANSLUCENT,
        )
        lp.gravity = Gravity.TOP or Gravity.START
        lp.x = curX.toInt()
        lp.y = curY.toInt()
        wm.addView(v, lp)
        cursor = v
        params = lp
        cursorShown = true
        Bridge.imeAutoSwitch(true)
    }

    private fun removeCursor() {
        cursor?.let { try { wm.removeView(it) } catch (_: Exception) {} }
        cursor = null
        params = null
        cursorShown = false
    }

    /** Called very often by Rust; the actual window update is coalesced to one per frame. */
    fun cursorTo(x: Float, y: Float) {
        curX = x.coerceIn(0f, screenW - 1)
        curY = y.coerceIn(0f, screenH - 1)
        if (!cursorDirty) {
            cursorDirty = true
            ui.post {
                cursorDirty = false
                ensureCursor()
                val p = params ?: return@post
                p.x = curX.toInt()
                p.y = curY.toInt()
                if (!cursorShown) { cursorShown = true; Bridge.imeAutoSwitch(true) }
                cursor?.visibility = View.VISIBLE
                try { wm.updateViewLayout(cursor, p) } catch (_: Exception) {}
            }
        }
        dragMoved()
    }

    fun hideCursor() {
        ui.post {
            cursor?.visibility = View.GONE
            cursorShown = false
            Bridge.imeAutoSwitch(false)
        }
        cancelPress()
    }

    // ------------------------------------------------------------------ clicks and drags

    private var pressed = false
    private var dragging = false
    private var ending = false
    private var inFlight = false
    private var downX = 0f
    private var downY = 0f
    private var downT = 0L
    private var lastX = 0f
    private var lastY = 0f
    private var stroke: GestureDescription.StrokeDescription? = null
    private val longPress = Runnable { if (pressed && !dragging) startDrag() }

    private fun slop() = 8 * density

    fun button(button: Int, down: Boolean, x: Float, y: Float) {
        ui.post {
            when (button) {
                0 -> if (down) pressDown(x, y) else pressUp(x, y)
                1 -> if (down) performGlobalAction(GLOBAL_ACTION_BACK)
                2 -> if (down) performGlobalAction(GLOBAL_ACTION_HOME)
                3 -> if (down) performGlobalAction(GLOBAL_ACTION_BACK)
                4 -> if (down) performGlobalAction(GLOBAL_ACTION_RECENTS)
            }
        }
    }

    private fun pressDown(x: Float, y: Float) {
        curX = x; curY = y
        pressed = true; dragging = false; ending = false
        downX = x; downY = y; downT = System.currentTimeMillis()
        ui.removeCallbacks(longPress)
        ui.postDelayed(longPress, 550)   // hold still: becomes a long press
    }

    private fun pressUp(x: Float, y: Float) {
        ui.removeCallbacks(longPress)
        if (!pressed) return
        pressed = false
        if (!dragging) {
            tap(downX, downY, 45)
        } else {
            curX = x; curY = y
            ending = true
            pump()
        }
    }

    private fun cancelPress() {
        ui.removeCallbacks(longPress)
        if (pressed && dragging) { ending = true; pump() }
        pressed = false
    }

    private fun tap(x: Float, y: Float, ms: Long) {
        val p = Path().apply { moveTo(x, y) }
        dispatchGesture(GestureDescription.Builder().addStroke(GestureDescription.StrokeDescription(p, 0, ms)).build(), null, null)
    }

    private fun dragMoved() {
        ui.post {
            if (pressed && !dragging && hypot(curX - downX, curY - downY) > slop()) startDrag()
            else if (dragging) pump()
        }
    }

    /** Begin a held touch that later segments continue, so dragging follows the mouse live. */
    private fun startDrag() {
        if (dragging) return
        dragging = true
        lastX = downX; lastY = downY
        val p = Path().apply { moveTo(downX, downY) }
        val s = GestureDescription.StrokeDescription(p, 0, 20, true)
        stroke = s
        inFlight = true
        val ok = dispatchGesture(GestureDescription.Builder().addStroke(s).build(), object : GestureResultCallback() {
            override fun onCompleted(g: GestureDescription?) { inFlight = false; pump() }
            override fun onCancelled(g: GestureDescription?) { inFlight = false; resetDrag() }
        }, null)
        if (!ok) { inFlight = false; resetDrag() }
    }

    private fun resetDrag() { dragging = false; ending = false; stroke = null; inFlight = false }

    private fun pump() {
        if (!dragging || inFlight) return
        val s = stroke ?: return resetDrag()
        val dist = hypot(curX - lastX, curY - lastY)
        val finish = ending
        if (!finish && dist < 1f) {
            // Held still: keep the touch alive with short continuations.
            ui.postDelayed({ pump() }, 40)
            return
        }
        val path = Path().apply { moveTo(lastX, lastY); lineTo(curX, curY) }
        val dur = max(16L, min(120L, (dist / density * 0.6f).toLong()))
        val next = s.continueStroke(path, 0, dur, !finish)
        stroke = next
        lastX = curX; lastY = curY
        inFlight = true
        val ok = dispatchGesture(GestureDescription.Builder().addStroke(next).build(), object : GestureResultCallback() {
            override fun onCompleted(g: GestureDescription?) {
                inFlight = false
                if (finish) resetDrag() else pump()
            }
            override fun onCancelled(g: GestureDescription?) { inFlight = false; resetDrag() }
        }, null)
        if (!ok) { inFlight = false; resetDrag() }
    }

    // ------------------------------------------------------------------ scrolling

    private var wheelDx = 0
    private var wheelDy = 0
    private var wheelScheduled = false

    /** Mouse wheel, in 1/120-notch style units (about 3 per dp). Positive dy = content moves down. */
    fun wheel(dx: Int, dy: Int) {
        ui.post {
            wheelDx += dx; wheelDy += dy
            if (!wheelScheduled) {
                wheelScheduled = true
                ui.postDelayed({ flushWheel() }, 50)
            }
        }
    }

    private fun flushWheel() {
        wheelScheduled = false
        val vx = (wheelDx / 3f) * density
        val vy = (wheelDy / 3f) * density
        wheelDx = 0; wheelDy = 0
        if (abs(vx) < 2f && abs(vy) < 2f) return
        val cx = if (cursorShown) curX else screenW / 2
        val cy = if (cursorShown) curY else screenH / 2
        val m = 6 * density
        // Swipe the finger along the scroll direction, centred on the pointer.
        val sx = (cx - vx / 2).coerceIn(m, screenW - m)
        val sy = (cy - vy / 2).coerceIn(m, screenH - m)
        val ex = (cx + vx / 2).coerceIn(m, screenW - m)
        val ey = (cy + vy / 2).coerceIn(m, screenH - m)
        if (hypot(ex - sx, ey - sy) < 4f) return
        val p = Path().apply { moveTo(sx, sy); lineTo(ex, ey) }
        val ms = max(60L, min(240L, (hypot(ex - sx, ey - sy) / density * 1.2f).toLong()))
        dispatchGesture(GestureDescription.Builder().addStroke(GestureDescription.StrokeDescription(p, 0, ms)).build(), null, null)
    }

    // ------------------------------------------------------------------ keyboard

    private val mods = HashSet<String>()
    private var caps = false

    fun resetKeys() { ui.post { mods.clear() } }

    fun key(hid: Int, down: Boolean) {
        ui.post { handleKey(hid, down) }
    }

    private fun modName(hid: Int): String? = when (hid) {
        0xE0, 0xE4 -> "ctrl"; 0xE1, 0xE5 -> "shift"; 0xE2, 0xE6 -> "alt"; 0xE3, 0xE7 -> "meta"; else -> null
    }

    private fun handleKey(hid: Int, down: Boolean) {
        modName(hid)?.let { if (down) mods.add(it) else mods.remove(it); return }
        if (!down) return
        if (hid == 0x39) { caps = !caps; return }
        val ctrl = "ctrl" in mods || "meta" in mods
        val shift = "shift" in mods
        val ime = HopperIME.instance
        if (ime != null && ime.ready()) { typeViaKeyboard(ime, hid, ctrl, shift); return }
        val field = focusedField()

        if (ctrl) {
            when (hid) {
                0x06 -> field?.performAction(AccessibilityNodeInfo.ACTION_COPY)          // C
                0x1B -> field?.performAction(AccessibilityNodeInfo.ACTION_CUT)           // X
                0x19 -> field?.performAction(AccessibilityNodeInfo.ACTION_PASTE)         // V
                0x04 -> field?.let { f ->                                                // A
                    val n = f.text?.length ?: 0
                    val b = Bundle().apply {
                        putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_START_INT, 0)
                        putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_END_INT, n)
                    }
                    f.performAction(AccessibilityNodeInfo.ACTION_SET_SELECTION, b)
                }
            }
            return
        }
        when (hid) {
            0x29 -> { performGlobalAction(GLOBAL_ACTION_BACK); return }                   // Esc
            0x4E -> { scrollPage(true); return }                                         // Page Down
            0x4B -> { scrollPage(false); return }                                        // Page Up
        }
        val f = field
        if (f == null) {
            if (hid == 0x2C) scrollPage(!shift)                                          // Space scrolls
            return
        }
        when (hid) {
            0x2A -> edit(f) { t, s, e -> if (s != e) Triple(t.removeRange(s, e), s, s) else if (s > 0) Triple(t.removeRange(s - 1, s), s - 1, s - 1) else null }
            0x4C -> edit(f) { t, s, e -> if (s != e) Triple(t.removeRange(s, e), s, s) else if (s < t.length) Triple(t.removeRange(s, s + 1), s, s) else null }
            0x28, 0x58 -> enter(f)
            0x50 -> moveCaret(f, -1, shift)
            0x4F -> moveCaret(f, +1, shift)
            0x4A -> moveCaretTo(f, 0)
            0x4D -> moveCaretTo(f, f.text?.length ?: 0)
            else -> keyChar(hid, shift)?.let { ch -> edit(f) { t, s, e -> Triple(t.replaceRange(s, e, ch.toString()), s + 1, s + 1) } }
        }
    }

    /** Typing through our keyboard works in every app, including web pages. */
    private fun typeViaKeyboard(ime: HopperIME, hid: Int, ctrl: Boolean, shift: Boolean) {
        val alt = "alt" in mods
        val meta = (if (ctrl) KeyEvent.META_CTRL_ON or KeyEvent.META_CTRL_LEFT_ON else 0) or
            (if (shift) KeyEvent.META_SHIFT_ON or KeyEvent.META_SHIFT_LEFT_ON else 0) or
            (if (alt) KeyEvent.META_ALT_ON or KeyEvent.META_ALT_LEFT_ON else 0)
        // Shortcuts (Ctrl/Cmd + letter) must arrive as real key presses.
        if (ctrl || alt) {
            val code = when (hid) {
                in 0x04..0x1D -> KeyEvent.KEYCODE_A + (hid - 0x04)
                in 0x1E..0x26 -> KeyEvent.KEYCODE_1 + (hid - 0x1E)
                0x27 -> KeyEvent.KEYCODE_0
                else -> 0
            }
            if (code != 0) { ime.press(code, meta); return }
        }
        when (hid) {
            0x29 -> performGlobalAction(GLOBAL_ACTION_BACK)
            0x28, 0x58 -> ime.enter()
            0x2A -> ime.press(KeyEvent.KEYCODE_DEL, meta)
            0x4C -> ime.press(KeyEvent.KEYCODE_FORWARD_DEL, meta)
            0x2B -> ime.press(KeyEvent.KEYCODE_TAB, meta)
            0x50 -> ime.press(KeyEvent.KEYCODE_DPAD_LEFT, meta)
            0x4F -> ime.press(KeyEvent.KEYCODE_DPAD_RIGHT, meta)
            0x52 -> ime.press(KeyEvent.KEYCODE_DPAD_UP, meta)
            0x51 -> ime.press(KeyEvent.KEYCODE_DPAD_DOWN, meta)
            0x4A -> ime.press(KeyEvent.KEYCODE_MOVE_HOME, meta)
            0x4D -> ime.press(KeyEvent.KEYCODE_MOVE_END, meta)
            0x4B -> ime.press(KeyEvent.KEYCODE_PAGE_UP, meta)
            0x4E -> ime.press(KeyEvent.KEYCODE_PAGE_DOWN, meta)
            else -> keyChar(hid, shift)?.let { ime.commit(it.toString()) }
        }
    }

    private fun focusedField(): AccessibilityNodeInfo? {
        val root = rootInActiveWindow ?: return null
        val f = root.findFocus(AccessibilityNodeInfo.FOCUS_INPUT)
        return if (f != null && f.isEditable) f else null
    }

    private fun currentText(f: AccessibilityNodeInfo): String {
        if (Build.VERSION.SDK_INT >= 26 && f.isShowingHintText) return ""
        return f.text?.toString() ?: ""
    }

    private fun edit(f: AccessibilityNodeInfo, change: (String, Int, Int) -> Triple<String, Int, Int>?) {
        val text = currentText(f)
        val s = (if (f.textSelectionStart >= 0) f.textSelectionStart else text.length).coerceIn(0, text.length)
        val e = (if (f.textSelectionEnd >= 0) f.textSelectionEnd else s).coerceIn(s, text.length)
        val r = change(text, s, e) ?: return
        val set = Bundle().apply { putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, r.first) }
        f.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT, set)
        val sel = Bundle().apply {
            putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_START_INT, r.second)
            putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_END_INT, r.third)
        }
        f.performAction(AccessibilityNodeInfo.ACTION_SET_SELECTION, sel)
    }

    private fun enter(f: AccessibilityNodeInfo) {
        if (Build.VERSION.SDK_INT >= 30 && !f.isMultiLine) {
            f.performAction(AccessibilityNodeInfo.AccessibilityAction.ACTION_IME_ENTER.id)
        } else {
            edit(f) { t, s, e -> Triple(t.replaceRange(s, e, "\n"), s + 1, s + 1) }
        }
    }

    private fun moveCaret(f: AccessibilityNodeInfo, by: Int, extend: Boolean) {
        val text = currentText(f)
        val s = if (f.textSelectionStart >= 0) f.textSelectionStart else text.length
        val e = if (f.textSelectionEnd >= 0) f.textSelectionEnd else s
        val b = Bundle()
        if (extend) {
            b.putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_START_INT, s)
            b.putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_END_INT, (e + by).coerceIn(0, text.length))
        } else {
            val p = ((if (by < 0) min(s, e) else max(s, e)) + (if (s != e) 0 else by)).coerceIn(0, text.length)
            b.putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_START_INT, p)
            b.putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_END_INT, p)
        }
        f.performAction(AccessibilityNodeInfo.ACTION_SET_SELECTION, b)
    }

    private fun moveCaretTo(f: AccessibilityNodeInfo, p: Int) {
        val b = Bundle().apply {
            putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_START_INT, p)
            putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_END_INT, p)
        }
        f.performAction(AccessibilityNodeInfo.ACTION_SET_SELECTION, b)
    }

    private fun scrollPage(forward: Boolean) {
        val x = screenW / 2
        val span = screenH * 0.6f
        val from = if (forward) screenH * 0.75f else screenH * 0.25f
        val p = Path().apply { moveTo(x, from); lineTo(x, from + (if (forward) -span else span) * 0.8f) }
        dispatchGesture(GestureDescription.Builder().addStroke(GestureDescription.StrokeDescription(p, 0, 220)).build(), null, null)
    }

    /** US keyboard layout: USB HID usage -> character. */
    private fun keyChar(hid: Int, shift: Boolean): Char? {
        val up = shift != caps
        return when (hid) {
            in 0x04..0x1D -> { val c = 'a' + (hid - 0x04); if (up) c.uppercaseChar() else c }
            in 0x1E..0x26 -> if (shift) "!@#$%^&*(".getOrNull(hid - 0x1E) else '1' + (hid - 0x1E)
            0x27 -> if (shift) ')' else '0'
            0x2C -> ' '
            0x2D -> if (shift) '_' else '-'
            0x2E -> if (shift) '+' else '='
            0x2F -> if (shift) '{' else '['
            0x30 -> if (shift) '}' else ']'
            0x31 -> if (shift) '|' else '\\'
            0x33 -> if (shift) ':' else ';'
            0x34 -> if (shift) '"' else '\''
            0x35 -> if (shift) '~' else '`'
            0x36 -> if (shift) '<' else ','
            0x37 -> if (shift) '>' else '.'
            0x38 -> if (shift) '?' else '/'
            else -> null
        }
    }
}
