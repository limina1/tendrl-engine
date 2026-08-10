// NIP-55 client plugin: talks to an Android signer app (Amber,
// com.greenart7c3.nostrsigner) so the user's key never enters this process.
//
// Two mechanisms per the NIP-55 spec:
//  - ContentResolver query on content://<signer-pkg>.<METHOD> — silent, works
//    only for permissions the user pre-approved in the signer app. The
//    payload rides in the *projection args* (the spec's quirk).
//  - ACTION_VIEW intent on the nostrsigner: scheme — foregrounds the signer's
//    approval UI; result returns through onActivityResult (the Plugin base
//    class routes it to the @ActivityCallback named in startActivityForResult).
//
// Rules kept here so every caller inherits them:
//  - pubkeys cross the boundary as lowercase hex (npub decoded in-plugin);
//  - signEvent expects a fully-formed payload (id precomputed, sig "");
//  - signature validity is enforced engine-side (verify_signed_event) — this
//    plugin only transports.
package io.github.limina1.tendrl

import android.app.Activity
import android.content.Intent
import android.net.Uri
import androidx.activity.result.ActivityResult
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

@InvokeArg
class GetPublicKeyArgs {
  lateinit var packageName: String
  // JSON array string of NIP-55 permission requests, e.g.
  // [{"type":"sign_event","kind":30041}, ...] — requested once at connect.
  var permissions: String? = null
}

@InvokeArg
class SignEventArgs {
  lateinit var packageName: String
  // Full event JSON: id precomputed, pubkey set, sig "".
  lateinit var eventJson: String
  // Request id echoed back by the signer so concurrent calls pair up.
  lateinit var id: String
  // Hex pubkey of the account we expect to sign (Amber's current_user).
  lateinit var currentUser: String
}

@InvokeArg
class CipherArgs {
  lateinit var packageName: String
  // Plaintext (encrypt) or ciphertext (decrypt).
  lateinit var content: String
  lateinit var id: String
  lateinit var currentUser: String
  // Counterparty hex pubkey.
  lateinit var pubkey: String
}

@TauriPlugin
class Nip55Plugin(private val activity: Activity) : Plugin(activity) {

  // --- discovery ---------------------------------------------------------

  @Command
  fun getInstalledSignerApps(invoke: Invoke) {
    val intent = Intent(Intent.ACTION_VIEW, Uri.parse(NOSTRSIGNER))
    val pm = activity.packageManager
    // Requires the <queries> manifest entry on Android 11+ — without it this
    // silently returns an empty list.
    val infos = pm.queryIntentActivities(intent, 0)
    val apps = JSArray()
    for (info in infos) {
      val app = JSObject()
      app.put("name", info.loadLabel(pm).toString())
      app.put("packageName", info.activityInfo.packageName)
      apps.put(app)
    }
    val ret = JSObject()
    ret.put("apps", apps)
    invoke.resolve(ret)
  }

  // --- get_public_key (intent always: first contact is consent) ----------

  @Command
  fun getPublicKey(invoke: Invoke) {
    val args = invoke.parseArgs(GetPublicKeyArgs::class.java)
    val intent = signerIntent(args.packageName, NOSTRSIGNER)
    intent.putExtra("type", "get_public_key")
    args.permissions?.let { intent.putExtra("permissions", it) }
    startActivityForResult(invoke, intent, "handleGetPublicKey")
  }

  @ActivityCallback
  fun handleGetPublicKey(invoke: Invoke, result: ActivityResult) {
    if (result.resultCode != Activity.RESULT_OK) {
      invoke.reject("signer request was cancelled or rejected")
      return
    }
    val raw = result.data?.getStringExtra("result")
    if (raw.isNullOrBlank()) {
      invoke.reject("signer returned no public key")
      return
    }
    val hexKey: String
    try {
      hexKey = toHexPubkey(raw)
    } catch (e: IllegalArgumentException) {
      invoke.reject("signer returned an unusable public key: ${e.message}")
      return
    }
    val ret = JSObject()
    ret.put("pubkey", hexKey)
    ret.put("package", result.data?.getStringExtra("package") ?: "")
    invoke.resolve(ret)
  }

  // --- sign_event (ContentResolver first, intent fallback) ----------------

  @Command
  fun signEvent(invoke: Invoke) {
    val args = invoke.parseArgs(SignEventArgs::class.java)
    // ContentResolver queries must not run on the main thread.
    Thread {
      try {
        val silent = querySigner(
          args.packageName,
          "SIGN_EVENT",
          arrayOf(args.eventJson, "", args.currentUser)
        )
        when (silent) {
          is SilentResult.Rejected -> invoke.reject("sign request rejected in the signer app")
          is SilentResult.Ok -> invoke.resolve(silent.data)
          is SilentResult.NotAuthorized -> activity.runOnUiThread {
            val intent = signerIntent(args.packageName, NOSTRSIGNER + args.eventJson)
            intent.putExtra("type", "sign_event")
            intent.putExtra("id", args.id)
            intent.putExtra("current_user", args.currentUser)
            startActivityForResult(invoke, intent, "handleSignEvent")
          }
        }
      } catch (e: Exception) {
        invoke.reject(e.message ?: "sign_event failed")
      }
    }.start()
  }

  @ActivityCallback
  fun handleSignEvent(invoke: Invoke, result: ActivityResult) {
    if (result.resultCode != Activity.RESULT_OK) {
      invoke.reject("signer request was cancelled or rejected")
      return
    }
    val data = result.data
    val event = data?.getStringExtra("event")
    val signature = data?.getStringExtra("result")
    if (event.isNullOrBlank() && signature.isNullOrBlank()) {
      invoke.reject("signer returned no event or signature")
      return
    }
    val ret = JSObject()
    if (!event.isNullOrBlank()) ret.put("event", event)
    if (!signature.isNullOrBlank()) ret.put("signature", signature)
    ret.put("id", data?.getStringExtra("id") ?: "")
    invoke.resolve(ret)
  }

  // --- nip04 / nip44 (implemented for completeness; engine-wired later) ---

  @Command
  fun nip04Encrypt(invoke: Invoke) = cipher(invoke, "NIP04_ENCRYPT", "nip04_encrypt")

  @Command
  fun nip04Decrypt(invoke: Invoke) = cipher(invoke, "NIP04_DECRYPT", "nip04_decrypt")

  @Command
  fun nip44Encrypt(invoke: Invoke) = cipher(invoke, "NIP44_ENCRYPT", "nip44_encrypt")

  @Command
  fun nip44Decrypt(invoke: Invoke) = cipher(invoke, "NIP44_DECRYPT", "nip44_decrypt")

  private fun cipher(invoke: Invoke, method: String, intentType: String) {
    val args = invoke.parseArgs(CipherArgs::class.java)
    Thread {
      try {
        val silent = querySigner(
          args.packageName,
          method,
          arrayOf(args.content, args.pubkey, args.currentUser)
        )
        when (silent) {
          is SilentResult.Rejected -> invoke.reject("$intentType rejected in the signer app")
          is SilentResult.Ok -> invoke.resolve(silent.data)
          is SilentResult.NotAuthorized -> activity.runOnUiThread {
            val intent = signerIntent(args.packageName, NOSTRSIGNER + args.content)
            intent.putExtra("type", intentType)
            intent.putExtra("id", args.id)
            intent.putExtra("current_user", args.currentUser)
            intent.putExtra("pubkey", args.pubkey)
            startActivityForResult(invoke, intent, "handleCipher")
          }
        }
      } catch (e: Exception) {
        invoke.reject(e.message ?: "$intentType failed")
      }
    }.start()
  }

  @ActivityCallback
  fun handleCipher(invoke: Invoke, result: ActivityResult) {
    if (result.resultCode != Activity.RESULT_OK) {
      invoke.reject("signer request was cancelled or rejected")
      return
    }
    val value = result.data?.getStringExtra("result")
    if (value.isNullOrBlank()) {
      invoke.reject("signer returned no result")
      return
    }
    val ret = JSObject()
    ret.put("result", value)
    ret.put("id", result.data?.getStringExtra("id") ?: "")
    invoke.resolve(ret)
  }

  // --- shared machinery ---------------------------------------------------

  private sealed class SilentResult {
    class Ok(val data: JSObject) : SilentResult()
    object Rejected : SilentResult()
    object NotAuthorized : SilentResult()
  }

  /** Query content://<pkg>.<method> with the NIP-55 projection-args payload. */
  private fun querySigner(
    packageName: String,
    method: String,
    projection: Array<String>
  ): SilentResult {
    val uri = Uri.parse("content://$packageName.$method")
    val cursor = activity.contentResolver.query(uri, projection, null, null, null)
      ?: return SilentResult.NotAuthorized
    cursor.use { c ->
      if (c.getColumnIndex("rejected") >= 0) return SilentResult.Rejected
      if (!c.moveToFirst()) return SilentResult.NotAuthorized
      val ret = JSObject()
      val eventIdx = c.getColumnIndex("event")
      if (eventIdx >= 0) {
        val event = c.getString(eventIdx)
        if (!event.isNullOrBlank()) {
          ret.put("event", event)
          return SilentResult.Ok(ret)
        }
      }
      val resultIdx = c.getColumnIndex("result")
      if (resultIdx >= 0) {
        val value = c.getString(resultIdx)
        if (!value.isNullOrBlank()) {
          // sign_event callers get {signature}; cipher callers get {result}.
          ret.put("result", value)
          ret.put("signature", value)
          return SilentResult.Ok(ret)
        }
      }
      return SilentResult.NotAuthorized
    }
  }

  private fun signerIntent(packageName: String, uri: String): Intent {
    val intent = Intent(Intent.ACTION_VIEW, Uri.parse(uri))
    intent.`package` = packageName
    intent.addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP)
    return intent
  }

  companion object {
    private const val NOSTRSIGNER = "nostrsigner:"
    private const val BECH32_CHARSET = "qpzry9x8gf2tvdw0s3jn54khce6mua7l"

    /**
     * Normalize a signer-returned key to lowercase hex. Amber returns npub
     * for get_public_key; decode it here so hex is the only format that ever
     * crosses into the WebView/engine. (Data-part decode only — the value
     * arrives over a local trusted binder round-trip, and the engine
     * re-validates everything against actual signatures.)
     */
    fun toHexPubkey(key: String): String {
      val k = key.trim()
      if (k.startsWith("npub1", ignoreCase = true)) {
        val data = k.substring(5).lowercase()
        if (data.length <= 6) throw IllegalArgumentException("npub too short")
        val values = data.dropLast(6).map { c ->
          val v = BECH32_CHARSET.indexOf(c)
          if (v < 0) throw IllegalArgumentException("invalid bech32 character '$c'")
          v
        }
        var acc = 0
        var bits = 0
        val out = StringBuilder()
        for (v in values) {
          acc = (acc shl 5) or v
          bits += 5
          while (bits >= 8) {
            bits -= 8
            out.append("%02x".format((acc shr bits) and 0xff))
          }
        }
        val hexKey = out.toString()
        if (hexKey.length != 64) throw IllegalArgumentException("npub payload is not 32 bytes")
        return hexKey
      }
      val lower = k.lowercase()
      if (lower.length != 64 || lower.any { it !in "0123456789abcdef" }) {
        throw IllegalArgumentException("expected npub or 64-char hex")
      }
      return lower
    }
  }
}
