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
import java.util.UUID

// Inhouse services updater: canonical source in miguelcoxcaballero/inhouse-services-updater.
// Change updater code there first, then sync ALL five apps at the same revision.
// New releases ALWAYS use A with a tiny changelog; B requires the user's explicit request.
// Derived from Inhouse Photos: download, integrity, package, version and signer checks retained.
class PhotosUpdateInstaller(private val activity: Activity, private val listener: Listener) {
  interface Listener { fun progress(downloadedBytes: Long, totalBytes: Long); fun stage(value: String) }
  interface Result { fun success(value: String); fun error(code: String, message: String?, details: Any?) }
  fun installUpdate(url: String, expectedSha256: String?, result: Result) {
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O && !activity.packageManager.canRequestPackageInstalls()) {
      activity.runOnUiThread {
        try {
          activity.startActivity(Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,
            Uri.parse("package:${activity.packageName}")))
          result.success("permission_required")
        } catch (error: Exception) {
          result.error("permission_unavailable", "Open Android settings to allow updates from this app, then try again.", null)
        }
      }
      return
    }

    if (!downloadRunning.compareAndSet(false, true)) {
      result.success("downloading")
      return
    }

    Thread({
      try {
        val updateDirectory = File(activity.cacheDir, "updates")
        if (!updateDirectory.exists() && !updateDirectory.mkdirs()) {
          throw IllegalStateException("Could not prepare secure update storage.")
        }
        val fileKey = MessageDigest.getInstance("SHA-256")
          .digest(url.toByteArray())
          .joinToString("") { byte -> "%02x".format(byte) }
          .take(16)
        val partialFile = File(updateDirectory, "inhouse-photos-$fileKey.apk.part")
        val apkFile = File(updateDirectory, "inhouse-photos-$fileKey.apk")
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
        val verified = verifiedPackage ?: throw SecurityException("Invalid update package.")

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
            // Use a foreground proxy to receive Android's result and preserve
            // the expected version across self-update process termination.
            val installIntent = Intent().apply {
              setClassName(activity.packageName, "com.inhouse.services.updater.InstallActivity")
              data = apkUri
              clipData = ClipData.newRawUri("Update APK", apkUri)
              putExtra("installId", UUID.randomUUID().toString())
              putExtra("versionCode", getVersionCode(verified))
              putExtra("versionName", verified.versionName ?: "")
              addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            }
            activity.startActivity(installIntent)
            result.success("installing")
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
    private val ALLOWED_HOSTS = setOf(
      "github.com",
      "raw.githubusercontent.com",
      "objects.githubusercontent.com",
      "release-assets.githubusercontent.com",
    )
  }
}
