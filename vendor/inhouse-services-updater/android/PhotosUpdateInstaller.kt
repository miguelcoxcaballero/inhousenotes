package com.inhouse.services.updater
import android.app.Activity
import android.content.ClipData
import android.content.Intent
import android.content.pm.PackageInfo
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.SystemClock
import android.provider.Settings
import androidx.core.content.FileProvider
import java.io.BufferedInputStream
import java.io.File
import java.io.FileOutputStream
import java.net.HttpURLConnection
import java.net.URL
import java.security.MessageDigest
import java.util.concurrent.atomic.AtomicBoolean

// ============================================================================
// FROZEN FILE - DO NOT EDIT. Read FROZEN_INSTALLER.md before touching anything.
// ============================================================================
// This is the Android installer of the Inhouse services updater, frozen at the
// version verified on a real Samsung phone on 2026-10-20: Inhouse Notes 1.0.27
// installed 1.0.28 from inside the app. Inhouse Photos has always installed its
// updates with this same ACTION_VIEW handoff.
//
// Every "improvement" of this path broke in-app updates:
//   - PackageInstaller sessions: "Android could not start the installation".
//   - ACTION_INSTALL_PACKAGE: Android's installer opened and closed on its own.
//   - ACTION_VIEW while ".update-provider" shared androidx FileProvider's class
//     with Capacitor's ".fileprovider": the installer vanished after the user
//     picked it in the "Open with" chooser.
// The intent was never the problem. The provider class was: Android keeps one
// provider instance per class, and the shared instance rejected every update
// URI, so no installer could read the APK. See UpdateFileProvider.java.
//
// What must stay exactly as it is (tests/test_frozen_installer.py and the
// "Frozen installer" workflow compare this file with tag installer-known-good):
//   - ACTION_VIEW + setDataAndType(APK MIME) + ClipData + read URI grant.
//   - The authority "${packageName}.update-provider" and cacheDir/updates.
//   - Every check: trusted SHA-256, package name, newer versionCode, signer.
//   - No PackageInstaller sessions, no ACTION_INSTALL_PACKAGE, no
//     DownloadManager, no third-party installer library, no extra intent flags.
// Only the user, in their own words, can unfreeze it; then a real phone must
// install an update from inside the app before the tag moves.
//
// Inhouse services updater: canonical source in miguelcoxcaballero/inhouse-services-updater.
// Change updater code there first, then sync ALL five apps at the same revision.
// New releases ALWAYS use A with a tiny changelog; B requires the user's explicit request.
// Derived from Inhouse Photos: download, integrity, package, version and signer checks retained.
class PhotosUpdateInstaller(private val activity: Activity, private val listener: Listener) {
  interface Listener { fun progress(downloadedBytes: Long, totalBytes: Long); fun stage(value: String) }
  interface Result { fun success(value: String); fun error(code: String, message: String?, details: Any?) }
  fun installUpdate(url: String, expectedSha256: String?, result: Result) {
    // The catalog's SHA-256 is mandatory: never hand Android bytes we cannot
    // prove are the published APK.
    if (expectedSha256 == null || !expectedSha256.matches(Regex("[0-9a-fA-F]{64}"))) {
      result.error("invalid_checksum", "The update is missing a trusted checksum. Check for updates again.", null)
      return
    }
    // Android 8+: this app must be allowed to "install unknown apps" before the
    // package installer accepts its APK. Open that setting and report
    // permission_required; the user comes back and taps Continue. Requires
    // REQUEST_INSTALL_PACKAGES in the manifest.
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O && !activity.packageManager.canRequestPackageInstalls()) {
      activity.startActivity(
        Intent(
          Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,
          Uri.parse("package:${activity.packageName}"),
        ),
      )
      result.success("permission_required")
      return
    }
    // One download at a time; a second tap only reports the running download.
    if (!downloadRunning.compareAndSet(false, true)) {
      result.success("downloading")
      return
    }

    Thread({
      try {
        // cacheDir/updates is exactly the root declared for ".update-provider"
        // in res/xml/inhouse_update_paths.xml (cache-path name="updates"
        // path="updates/", written by register_android.py). Moving the file
        // anywhere else makes FileProvider.getUriForFile throw.
        val updateDirectory = File(activity.cacheDir, "updates")
        if (!updateDirectory.exists() && !updateDirectory.mkdirs()) {
          throw IllegalStateException("Could not prepare secure update storage.")
        }
        // Stable per-URL file name. The "inhouse-photos-" prefix is inherited
        // from Photos and is only a name; it is the same in every app.
        val fileKey = MessageDigest.getInstance("SHA-256")
          .digest(url.toByteArray())
          .joinToString("") { byte -> "%02x".format(byte) }
          .take(16)
        val partialFile = File(updateDirectory, "inhouse-photos-$fileKey.apk.part")
        val apkFile = File(updateDirectory, "inhouse-photos-$fileKey.apk")
        // A partial download resumes; a complete one is re-verified, never trusted.
        // Retry the already verified package instead of downloading it again.
        var verifiedPackage: PackageInfo? = null
        if (apkFile.isFile) {
          try { verifiedPackage = verifyApk(apkFile, expectedSha256) }
          catch (_: Exception) { apkFile.delete() }
        }
        if (verifiedPackage == null) {
          download(url, partialFile)
          if (!partialFile.renameTo(apkFile)) {
            partialFile.copyTo(apkFile, overwrite = true)
            partialFile.delete()
          }
          publishStage("verifying")
          try { verifiedPackage = verifyApk(apkFile, expectedSha256) }
          catch (error: Exception) {
            apkFile.delete(); partialFile.delete(); throw error
          }
        }
        if (verifiedPackage == null) throw SecurityException("Invalid update package.")

        // ".update-provider" is served by UpdateFileProvider, the updater's OWN
        // provider class. It must never share a class with another provider of
        // the app: Android would answer this authority with that provider's
        // instance, which rejects the URI ("authority does not match"), and the
        // package installer could not read the APK. That was the bug behind
        // every failed in-app update in Notes and Read before 2026-10-20.
        val apkUri = FileProvider.getUriForFile(
          activity,
          "${activity.packageName}.update-provider",
          apkFile,
        )
        activity.runOnUiThread {
          try {
            publishStage("installing")
            if (activity.isFinishing || activity.isDestroyed) {
              throw IllegalStateException("Open the app and try the update again.")
            }
            // Hand the verified APK straight to Android's package installer with
            // ACTION_VIEW, exactly as Inhouse Photos always has. A content URI
            // (never file://) plus FLAG_GRANT_READ_URI_PERMISSION and the
            // ClipData copy of the URI let whichever installer the user picks in
            // the "Open with" chooser read the file. This exact intent is the one
            // verified on a real phone: do not add flags, a package filter or a
            // chooser of our own. PackageInstaller sessions and
            // ACTION_INSTALL_PACKAGE were tried and are not coming back; they
            // failed for the provider reason above, not because of the intent.
            // Success is never assumed here: the web updater confirms it from the
            // installed versionCode when the app opens again.
            val installIntent = Intent(Intent.ACTION_VIEW).apply {
              setDataAndType(apkUri, APK_MIME_TYPE)
              clipData = ClipData.newRawUri("Update APK", apkUri)
              addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            }
            activity.startActivity(installIntent)
            // "ready" only means Android's installer was opened, not installed.
            result.success("ready")
          } catch (error: Exception) {
            result.error("installer_unavailable", error.message ?: "Android could not open the installer.", null)
          }
        }
      } catch (error: Exception) {
        activity.runOnUiThread {
          result.error("update_failed", error.message ?: "The update could not be installed.", null)
        }
      } finally {
        downloadRunning.set(false)
      }
    }, "InhousePhotosUpdate").start()
  }

  // Redirects are followed by hand so every hop is checked against
  // ALLOWED_HOSTS (GitHub release downloads redirect to
  // release-assets.githubusercontent.com). A Range request resumes a partial
  // file; 416 restarts the download from zero once.
  private fun download(initialUrl: String, destination: File) {
    var currentUrl = URL(initialUrl)
    var redirects = 0
    var restartedWithoutRange = false

    while (true) {
      validateUrl(currentUrl)
      val connection = currentUrl.openConnection() as HttpURLConnection
      connection.instanceFollowRedirects = false
      connection.connectTimeout = 15_000
      connection.readTimeout = 120_000
      connection.setRequestProperty("Accept", APK_MIME_TYPE)
      val resumeFrom = destination.length().takeIf { it > 0L } ?: 0L
      if (resumeFrom > 0L) {
        connection.setRequestProperty("Range", "bytes=$resumeFrom-")
      }

      try {
        val responseCode = connection.responseCode
        if (responseCode in 300..399) {
          val location = connection.getHeaderField("Location")
            ?: throw IllegalStateException("The update redirect is invalid.")
          if (++redirects > MAX_REDIRECTS) {
            throw IllegalStateException("The update has too many redirects.")
          }
          currentUrl = URL(currentUrl, location)
          continue
        }
        if (responseCode == 416 && !restartedWithoutRange) {
          if (destination.exists() && !destination.delete()) {
            throw IllegalStateException("Could not restart the interrupted update download.")
          }
          currentUrl = URL(initialUrl)
          redirects = 0
          restartedWithoutRange = true
          continue
        }
        if (responseCode !in 200..299) {
          throw IllegalStateException("Update download failed ($responseCode).")
        }

        val isResuming = responseCode == HttpURLConnection.HTTP_PARTIAL && resumeFrom > 0L
        var downloadedBytes = if (isResuming) resumeFrom else 0L
        val contentRangeTotal = connection.getHeaderField("Content-Range")
          ?.substringAfterLast('/', "")
          ?.toLongOrNull()
          ?.takeIf { it > 0L }
        val responseBytes = connection.contentLengthLong.takeIf { it > 0L }
        val expectedBytes = contentRangeTotal ?: responseBytes?.let { it + downloadedBytes }
        var lastProgressUpdate = 0L
        publishDownloadProgress(downloadedBytes, expectedBytes)
        BufferedInputStream(connection.inputStream).use { input ->
          FileOutputStream(destination, isResuming).use { output ->
            val buffer = ByteArray(32 * 1024)
            while (true) {
              val count = input.read(buffer)
              if (count < 0) break
              output.write(buffer, 0, count)
              downloadedBytes += count

              val now = SystemClock.elapsedRealtime()
              if (now - lastProgressUpdate >= PROGRESS_UPDATE_INTERVAL_MS || downloadedBytes == expectedBytes) {
                lastProgressUpdate = now
                publishDownloadProgress(downloadedBytes, expectedBytes)
              }
            }
            output.fd.sync()
          }
        }
        if (downloadedBytes < MINIMUM_APK_BYTES) {
          throw IllegalStateException("The downloaded update is incomplete.")
        }
        publishDownloadProgress(downloadedBytes, expectedBytes ?: downloadedBytes)
        return
      } finally {
        connection.disconnect()
      }
    }
  }

  private fun publishDownloadProgress(downloadedBytes: Long, totalBytes: Long?) {
    activity.runOnUiThread { listener.progress(downloadedBytes, totalBytes ?: -1L) }
  }
  private fun publishStage(stage: String) {
    activity.runOnUiThread { listener.stage(stage) }
  }
  private fun validateUrl(url: URL) {
    if (!url.protocol.equals("https", ignoreCase = true) || url.host.lowercase() !in ALLOWED_HOSTS) {
      throw SecurityException("The update URL is not allowed.")
    }
  }

  // Defence in depth before Android sees the file: exact catalog SHA-256, same
  // package, strictly newer versionCode and the same signing certificate as the
  // installed app. Android rejects a different signer anyway; failing here
  // gives a clear message and deletes the bad file.
  private fun verifyApk(apkFile: File, expectedSha256: String?): PackageInfo {
    if (!expectedSha256.isNullOrBlank()) {
      val actualSha256 = apkFile.inputStream().use { input ->
        val digest = MessageDigest.getInstance("SHA-256")
        val buffer = ByteArray(64 * 1024)
        while (true) {
          val count = input.read(buffer)
          if (count < 0) break
          digest.update(buffer, 0, count)
        }
        digest.digest().joinToString("") { byte -> "%02x".format(byte) }
      }
      if (!actualSha256.equals(expectedSha256, ignoreCase = true)) {
        throw SecurityException("The update checksum did not match the trusted SHA-256 value.")
      }
    }
    val archive = getArchivePackageInfo(apkFile)
      ?: throw SecurityException("The downloaded file is not a valid Android package.")
    val installed = getInstalledPackageInfo()

    if (archive.packageName != activity.packageName) {
      throw SecurityException("The update belongs to a different application.")
    }
    if (getVersionCode(archive) <= getVersionCode(installed)) {
      throw SecurityException("The downloaded build is not newer than the installed app.")
    }
    if (getSigningDigests(archive) != getSigningDigests(installed)) {
      throw SecurityException("The update signature does not match Inhouse Photos.")
    }
    return archive
  }

  @Suppress("DEPRECATION")
  private fun getArchivePackageInfo(apkFile: File): PackageInfo? {
    val flags = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
      PackageManager.GET_SIGNING_CERTIFICATES
    } else {
      PackageManager.GET_SIGNATURES
    }
    return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
      activity.packageManager.getPackageArchiveInfo(
        apkFile.absolutePath,
        PackageManager.PackageInfoFlags.of(flags.toLong()),
      )
    } else {
      activity.packageManager.getPackageArchiveInfo(apkFile.absolutePath, flags)
    }
  }

  @Suppress("DEPRECATION")
  private fun getInstalledPackageInfo(): PackageInfo {
    val flags = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
      PackageManager.GET_SIGNING_CERTIFICATES
    } else {
      PackageManager.GET_SIGNATURES
    }
    return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
      activity.packageManager.getPackageInfo(
        activity.packageName,
        PackageManager.PackageInfoFlags.of(flags.toLong()),
      )
    } else {
      activity.packageManager.getPackageInfo(activity.packageName, flags)
    }
  }

  @Suppress("DEPRECATION")
  private fun getSigningDigests(packageInfo: PackageInfo): Set<String> {
    val signatures = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
      packageInfo.signingInfo?.apkContentsSigners.orEmpty()
    } else {
      packageInfo.signatures.orEmpty()
    }
    if (signatures.isEmpty()) {
      throw SecurityException("The update signature could not be verified.")
    }
    return signatures.map { signature ->
      MessageDigest.getInstance("SHA-256").digest(signature.toByteArray()).joinToString("") { byte ->
        "%02x".format(byte)
      }
    }.toSet()
  }

  @Suppress("DEPRECATION")
  private fun getVersionCode(packageInfo: PackageInfo): Long =
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) packageInfo.longVersionCode else packageInfo.versionCode.toLong()

  companion object {
    private const val CHANNEL_NAME = "com.inhousesoftware.photos/updates"
    private const val APK_MIME_TYPE = "application/vnd.android.package-archive"
    private const val MINIMUM_APK_BYTES = 100_000L
    private const val MAX_REDIRECTS = 5
    private const val PROGRESS_UPDATE_INTERVAL_MS = 200L
    private val downloadRunning = AtomicBoolean(false)
    // Only GitHub release hosts. Every extra host widens what the app will
    // download and hand to the package installer.
    private val ALLOWED_HOSTS = setOf(
      "github.com",
      "raw.githubusercontent.com",
      "objects.githubusercontent.com",
      "release-assets.githubusercontent.com",
    )
  }
}
