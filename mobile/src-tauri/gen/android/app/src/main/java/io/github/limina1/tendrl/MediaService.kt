// Foreground service for narration playback. It plays nothing itself — the
// WebView does — but Android only keeps a backgrounded process decoding audio
// (and exempt from Doze network limits) behind a mediaPlayback foreground
// service. The MediaSession here gives lock-screen / notification / headset
// controls; every button is relayed to the page through MediaBridge.
package io.github.limina1.tendrl

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.graphics.drawable.Icon
import android.media.MediaMetadata
import android.media.session.MediaSession
import android.media.session.PlaybackState
import android.os.Build
import android.os.IBinder
import android.os.PowerManager
import android.os.SystemClock

class MediaService : Service() {
  companion object {
    const val CHANNEL = "tendrl.narration"
    const val NOTIF_ID = 4137
    const val ACTION_STATE = "io.github.limina1.tendrl.media.STATE"
    const val ACTION_BUTTON = "io.github.limina1.tendrl.media.BUTTON"
    const val ACTION_STOP = "io.github.limina1.tendrl.media.STOP"
    const val EXTRA_PLAYING = "playing"
    const val EXTRA_TITLE = "title"
    const val EXTRA_SUBTITLE = "subtitle"
    const val EXTRA_ALBUM = "album"
    const val EXTRA_POSITION = "position" // seconds
    const val EXTRA_DURATION = "duration" // seconds
    const val EXTRA_RATE = "rate"
    const val EXTRA_HAS_NEXT = "hasNext"
    const val EXTRA_HAS_PREV = "hasPrev"
    const val EXTRA_ACTION = "action"

    @Volatile var running: Boolean = false
  }

  private var session: MediaSession? = null
  private var wakeLock: PowerManager.WakeLock? = null
  private var playing = false
  private var title = ""
  private var subtitle = ""
  private var album = ""
  private var hasNext = false
  private var hasPrev = false

  override fun onBind(intent: Intent?): IBinder? = null

  override fun onCreate() {
    super.onCreate()
    running = true
    createChannel()
    session = MediaSession(this, "tendrl-narration").apply {
      if (Build.VERSION.SDK_INT < 26) legacyFlags(this)
      setCallback(object : MediaSession.Callback() {
        override fun onPlay() = MediaBridge.dispatch("play")
        override fun onPause() = MediaBridge.dispatch("pause")
        override fun onSkipToNext() = MediaBridge.dispatch("next")
        override fun onSkipToPrevious() = MediaBridge.dispatch("prev")
        override fun onStop() = MediaBridge.dispatch("stop")
        override fun onSeekTo(pos: Long) = MediaBridge.dispatch("seek", pos / 1000.0)
        override fun onFastForward() = MediaBridge.dispatch("forward")
        override fun onRewind() = MediaBridge.dispatch("rewind")
      })
      isActive = true
    }
  }

  @Suppress("DEPRECATION")
  private fun legacyFlags(s: MediaSession) {
    s.setFlags(MediaSession.FLAG_HANDLES_MEDIA_BUTTONS or MediaSession.FLAG_HANDLES_TRANSPORT_CONTROLS)
  }

  override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
    when (intent?.action) {
      ACTION_STATE -> apply(intent)
      ACTION_BUTTON -> MediaBridge.dispatch(intent.getStringExtra(EXTRA_ACTION) ?: "pause")
      ACTION_STOP -> { MediaBridge.keepAwake = false; stopSelf(); MediaBridge.settle() }
      // Restarted by the system with no intent: nothing to show, go away.
      null -> stopSelf()
    }
    return START_NOT_STICKY
  }

  private fun apply(i: Intent) {
    playing = i.getBooleanExtra(EXTRA_PLAYING, false)
    title = i.getStringExtra(EXTRA_TITLE) ?: ""
    subtitle = i.getStringExtra(EXTRA_SUBTITLE) ?: ""
    album = i.getStringExtra(EXTRA_ALBUM) ?: ""
    hasNext = i.getBooleanExtra(EXTRA_HAS_NEXT, false)
    hasPrev = i.getBooleanExtra(EXTRA_HAS_PREV, false)
    val posMs = (i.getDoubleExtra(EXTRA_POSITION, 0.0) * 1000).toLong().coerceAtLeast(0)
    val durMs = (i.getDoubleExtra(EXTRA_DURATION, 0.0) * 1000).toLong().coerceAtLeast(0)
    val rate = i.getDoubleExtra(EXTRA_RATE, 1.0).toFloat()

    session?.setMetadata(
      MediaMetadata.Builder()
        .putString(MediaMetadata.METADATA_KEY_TITLE, title)
        .putString(MediaMetadata.METADATA_KEY_ARTIST, subtitle)
        .putString(MediaMetadata.METADATA_KEY_ALBUM, album)
        .putLong(MediaMetadata.METADATA_KEY_DURATION, durMs)
        .build()
    )
    var actions = PlaybackState.ACTION_PLAY or PlaybackState.ACTION_PAUSE or
      PlaybackState.ACTION_PLAY_PAUSE or PlaybackState.ACTION_STOP or
      PlaybackState.ACTION_SEEK_TO or PlaybackState.ACTION_FAST_FORWARD or PlaybackState.ACTION_REWIND
    if (hasNext) actions = actions or PlaybackState.ACTION_SKIP_TO_NEXT
    if (hasPrev) actions = actions or PlaybackState.ACTION_SKIP_TO_PREVIOUS
    session?.setPlaybackState(
      PlaybackState.Builder()
        .setActions(actions)
        .setState(
          if (playing) PlaybackState.STATE_PLAYING else PlaybackState.STATE_PAUSED,
          posMs,
          if (playing) rate else 0f,
          SystemClock.elapsedRealtime()
        )
        .build()
    )

    val n = buildNotification()
    if (Build.VERSION.SDK_INT >= 29) {
      startForeground(NOTIF_ID, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK)
    } else {
      startForeground(NOTIF_ID, n)
    }
    if (playing) acquireWake() else releaseWake()
  }

  private fun buttonIntent(action: String, code: Int): PendingIntent =
    PendingIntent.getService(
      this, code,
      Intent(this, MediaService::class.java).setAction(ACTION_BUTTON).putExtra(EXTRA_ACTION, action),
      PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
    )

  private fun buildNotification(): Notification {
    val open = PendingIntent.getActivity(
      this, 0,
      Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
      PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
    )
    val stop = PendingIntent.getService(
      this, 9,
      Intent(this, MediaService::class.java).setAction(ACTION_STOP),
      PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
    )
    val b = if (Build.VERSION.SDK_INT >= 26) Notification.Builder(this, CHANNEL) else legacyBuilder()
    b.setSmallIcon(R.drawable.ic_narration)
      .setContentTitle(title.ifEmpty { "tendrl" })
      .setContentText(subtitle)
      .setSubText(album)
      .setContentIntent(open)
      .setDeleteIntent(stop)
      .setOngoing(playing)
      .setOnlyAlertOnce(true)
      .setVisibility(Notification.VISIBILITY_PUBLIC)
      .setCategory(Notification.CATEGORY_TRANSPORT)

    val compact = ArrayList<Int>()
    var idx = 0
    if (hasPrev) {
      b.addAction(action(android.R.drawable.ic_media_previous, "Previous", buttonIntent("prev", 1)))
      compact.add(idx); idx++
    }
    if (playing) {
      b.addAction(action(android.R.drawable.ic_media_pause, "Pause", buttonIntent("pause", 2)))
    } else {
      b.addAction(action(android.R.drawable.ic_media_play, "Play", buttonIntent("play", 3)))
    }
    compact.add(idx); idx++
    if (hasNext) {
      b.addAction(action(android.R.drawable.ic_media_next, "Next", buttonIntent("next", 4)))
      compact.add(idx); idx++
    }
    b.addAction(action(android.R.drawable.ic_menu_close_clear_cancel, "Stop", stop))

    session?.let { s ->
      b.setStyle(
        Notification.MediaStyle()
          .setMediaSession(s.sessionToken)
          .setShowActionsInCompactView(*compact.take(3).toIntArray())
      )
    }
    return b.build()
  }

  @Suppress("DEPRECATION")
  private fun legacyBuilder(): Notification.Builder = Notification.Builder(this)

  private fun action(icon: Int, label: String, pi: PendingIntent): Notification.Action =
    Notification.Action.Builder(Icon.createWithResource(this, icon), label, pi).build()

  private fun createChannel() {
    if (Build.VERSION.SDK_INT < 26) return
    val nm = getSystemService(NotificationManager::class.java)
    if (nm.getNotificationChannel(CHANNEL) != null) return
    val ch = NotificationChannel(CHANNEL, "Narration playback", NotificationManager.IMPORTANCE_LOW)
    ch.description = "Controls for a publication being read aloud"
    ch.setShowBadge(false)
    nm.createNotificationChannel(ch)
  }

  private fun acquireWake() {
    if (wakeLock?.isHeld == true) return
    val pm = getSystemService(PowerManager::class.java)
    wakeLock = pm.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "tendrl:narration").also {
      it.setReferenceCounted(false)
      it.acquire(6 * 60 * 60 * 1000L) // safety cap: six hours
    }
  }

  private fun releaseWake() {
    wakeLock?.let { if (it.isHeld) it.release() }
    wakeLock = null
  }

  override fun onDestroy() {
    running = false
    releaseWake()
    session?.isActive = false
    session?.release()
    session = null
    stopForeground(STOP_FOREGROUND_REMOVE)
    super.onDestroy()
  }
}
