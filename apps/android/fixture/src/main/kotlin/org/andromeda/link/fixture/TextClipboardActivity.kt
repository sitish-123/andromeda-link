package org.andromeda.link.fixture

import android.app.Activity
import android.content.ClipData
import android.content.ClipboardManager

/** Copies the `text` extra, with the `html` extra as its HTML when given, as any other app's copy would. */
class TextClipboardActivity : Activity() {
    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (!hasFocus || isFinishing) return
        val text = intent.getStringExtra("text").orEmpty()
        val html = intent.getStringExtra("html")
        val clip = if (html == null) ClipData.newPlainText("E2E", text) else ClipData.newHtmlText("E2E", text, html)
        getSystemService(ClipboardManager::class.java).setPrimaryClip(clip)
        finish()
    }
}
