// Shared state between the WebView host and the narration media service.
//
// The HTML5 <audio> element in the SPA stays the one player: bytes stream
// from Blossom into the WebView, and the Kotlin side only (a) keeps the
// WebView un-paused while a playlist is loaded, (b) holds a foreground
// service + MediaSession so Android lets the process keep decoding with the
// screen off, and (c) relays lock-screen / notification / headset transport
// buttons back into the page as a `tendrl:media` CustomEvent.
package io.github.limina1.tendrl

import android.os.Handler
import android.os.Looper
import android.webkit.WebView
import org.json.JSONObject

object MediaBridge {
  /** The Tauri WebView, captured in MediaPlugin.load(). */
  @Volatile var webView: WebView? = null

  /**
   * True from the first setState until stop: the SPA has a playlist loaded
   * (playing or paused). While set, MainActivity.onPause() re-resumes the
   * WebView right after wry pauses it, so the page is never left hidden
   * with media suspended when the activity leaves the foreground (screen
   * off, app switch).
   */
  @Volatile var keepAwake: Boolean = false

  /** Mirrors MainActivity's resumed state so settle() can apply a skipped pause. */
  @Volatile var activityResumed: Boolean = false

  private val main = Handler(Looper.getMainLooper())

  /** Transport action from the system → the page (NarrationPlayer listens). */
  fun dispatch(action: String, position: Double? = null) {
    val wv = webView ?: return
    val detail = JSONObject().put("action", action)
    if (position != null) detail.put("position", position)
    val js = "window.dispatchEvent(new CustomEvent('tendrl:media',{detail:$detail}))"
    main.post { wv.evaluateJavascript(js, null) }
  }

  /** keepAwake dropped while backgrounded: apply the WebView pause the activity skipped. */
  fun settle() {
    if (!keepAwake && !activityResumed) main.post { webView?.onPause() }
  }
}
