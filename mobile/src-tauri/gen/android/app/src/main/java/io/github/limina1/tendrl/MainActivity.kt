package io.github.limina1.tendrl

import android.os.Bundle
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    // Android 15+ enforces edge-to-edge (the template also opted in), which
    // slides the WebView under the status/navigation bars — and the SPA has
    // no safe-area handling, so the top bar collided with the clock. Pad the
    // content view by the system-bar + cutout + IME insets instead: the
    // WebView keeps a browser-like coordinate space and the keyboard resizes
    // content the way the shell already expects.
    val content = findViewById<android.view.ViewGroup>(android.R.id.content)
    ViewCompat.setOnApplyWindowInsetsListener(content) { v, insets ->
      val bars = insets.getInsets(
        WindowInsetsCompat.Type.systemBars()
          or WindowInsetsCompat.Type.displayCutout()
          or WindowInsetsCompat.Type.ime()
      )
      v.setPadding(bars.left, bars.top, bars.right, bars.bottom)
      WindowInsetsCompat.CONSUMED
    }
  }
}
