package com.screenhopper.app

import android.inputmethodservice.InputMethodService
import android.view.Gravity
import android.view.KeyEvent
import android.view.View
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputMethodManager
import android.widget.LinearLayout
import android.widget.TextView

/**
 * A minimal keyboard that exists only to receive typing from your computers. While it is the selected
 * keyboard, text, shortcuts and arrow keys from the computer are delivered straight into the focused
 * field of any app, web pages included (Android's accessibility API cannot see inside web content).
 * It shows a small bar instead of keys; tap "Switch keyboard" to get your normal keyboard back.
 */
class HopperIME : InputMethodService() {
    companion object {
        @Volatile var instance: HopperIME? = null
    }

    override fun onCreate() {
        super.onCreate()
        instance = this
    }

    override fun onDestroy() {
        if (instance === this) instance = null
        super.onDestroy()
    }

    override fun onCreateInputView(): View {
        val d = resources.displayMetrics.density
        val bar = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setBackgroundColor(0xFF18222D.toInt())
            setPadding((16 * d).toInt(), (10 * d).toInt(), (10 * d).toInt(), (10 * d).toInt())
        }
        bar.addView(TextView(this).apply {
            text = "Typing from your computer.\nTap to use this phone's own keyboard."
            setTextColor(0xFFEEF4F8.toInt())
            textSize = 13f
        }, LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f))
        bar.addView(TextView(this).apply {
            text = "Switch keyboard"
            setTextColor(0xFF07231F.toInt())
            setBackgroundColor(0xFF1FB7AD.toInt())
            setPadding((14 * d).toInt(), (10 * d).toInt(), (14 * d).toInt(), (10 * d).toInt())
            textSize = 13f
            setOnClickListener {
                if (!switchToPreviousInputMethod()) {
                    (getSystemService(INPUT_METHOD_SERVICE) as InputMethodManager).showInputMethodPicker()
                }
            }
        })
        return bar
    }

    /** True when a text field is focused and we can type into it. */
    fun ready(): Boolean = currentInputConnection != null && (currentInputEditorInfo?.inputType ?: 0) != 0

    fun commit(text: String) {
        currentInputConnection?.commitText(text, 1)
    }

    fun press(keyCode: Int, meta: Int = 0) {
        val ic = currentInputConnection ?: return
        val t = System.currentTimeMillis()
        ic.sendKeyEvent(KeyEvent(t, t, KeyEvent.ACTION_DOWN, keyCode, 0, meta))
        ic.sendKeyEvent(KeyEvent(t, t, KeyEvent.ACTION_UP, keyCode, 0, meta))
    }

    /** The Enter key: run the field's action (Search, Go, Send...) or insert a new line in multi-line fields. */
    fun enter() {
        val info = currentInputEditorInfo
        val multiline = info != null && (info.inputType and android.text.InputType.TYPE_TEXT_FLAG_MULTI_LINE) != 0
        val action = (info?.imeOptions ?: 0) and EditorInfo.IME_MASK_ACTION
        if (!multiline && action != EditorInfo.IME_ACTION_NONE && action != EditorInfo.IME_ACTION_UNSPECIFIED) {
            currentInputConnection?.performEditorAction(action)
        } else {
            press(KeyEvent.KEYCODE_ENTER)
        }
    }
}
