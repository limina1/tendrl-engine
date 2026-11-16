// Tauri plugin the SPA calls to mirror its narration player into the
// Android media stack: `setState` on every play/pause/track change (starts
// the MediaService foreground service and refreshes its notification +
// MediaSession), `stop` when the player closes. See MediaBridge for the
// return path (system buttons → `tendrl:media` events in the page).
package io.github.limina1.tendrl

import android.Manifest
import android.app.Activity
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.webkit.WebView
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.Plugin

@InvokeArg
class MediaStateArgs {
  var playing: Boolean = false
  var title: String = ""
  var subtitle: String = ""
  var album: String = ""
  var position: Double = 0.0
  var duration: Double = 0.0
  var rate: Double = 1.0
  var hasNext: Boolean = false
  var hasPrev: Boolean = false
}

@TauriPlugin
class MediaPlugin(private val activity: Activity) : Plugin(activity) {
  private var askedNotifications = false

  override fun load(webView: WebView) {
    MediaBridge.webView = webView
  }

  @Command
  fun setState(invoke: Invoke) {
    val a = invoke.parseArgs(MediaStateArgs::class.java)
    askNotificationsOnce()
    MediaBridge.keepAwake = true
    val i = Intent(activity, MediaService::class.java)
      .setAction(MediaService.ACTION_STATE)
      .putExtra(MediaService.EXTRA_PLAYING, a.playing)
      .putExtra(MediaService.EXTRA_TITLE, a.title)
      .putExtra(MediaService.EXTRA_SUBTITLE, a.subtitle)
      .putExtra(MediaService.EXTRA_ALBUM, a.album)
      .putExtra(MediaService.EXTRA_POSITION, a.position)
      .putExtra(MediaService.EXTRA_DURATION, a.duration)
      .putExtra(MediaService.EXTRA_RATE, a.rate)
      .putExtra(MediaService.EXTRA_HAS_NEXT, a.hasNext)
      .putExtra(MediaService.EXTRA_HAS_PREV, a.hasPrev)
    try {
      if (Build.VERSION.SDK_INT >= 26 && !MediaService.running) {
        activity.startForegroundService(i)
      } else {
        activity.startService(i)
      }
      invoke.resolve()
    } catch (e: Exception) {
      // Background-start restriction or similar: the page keeps playing in
      // the foreground regardless; report so the console shows why.
      invoke.reject("media service: ${e.message}")
    }
  }

  @Command
  fun stop(invoke: Invoke) {
    MediaBridge.keepAwake = false
    activity.stopService(Intent(activity, MediaService::class.java))
    MediaBridge.settle()
    invoke.resolve()
  }

  /** Android 13+: the service runs without it, but the controls notification needs it. */
  private fun askNotificationsOnce() {
    if (Build.VERSION.SDK_INT < 33 || askedNotifications) return
    askedNotifications = true
    if (activity.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
      activity.requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 7301)
    }
  }
}
