package com.inhouse.services.updater;

// ============================================================================
// FROZEN FILE - DO NOT EDIT, RENAME, MOVE OR DELETE. Read FROZEN_INSTALLER.md.
// ============================================================================
// The updater's own FileProvider class for "${applicationId}.update-provider".
// It is empty on purpose: the class NAME is the whole fix.
//
// Android keeps one ContentProvider instance per class. Notes and Read are
// Capacitor apps, and Capacitor already registers androidx.core.content.FileProvider
// for "${applicationId}.fileprovider". Until 2026-10-20 the updater registered
// ".update-provider" with that same androidx class, so Android answered
// ".update-provider" with the instance created for ".fileprovider", and
// ContentProvider.validateIncomingUri rejected every update URI because its
// authority did not match ("The authority of the uri ... does not match").
// Nothing could read the downloaded APK:
//   - Notes, PackageInstaller sessions: "Android could not start the installation".
//   - Read, ACTION_INSTALL_PACKAGE: the installer opened and closed on its own.
//   - Both, ACTION_VIEW: the installer vanished after picking it in "Open with".
// Inhouse Photos never failed because it registers androidx FileProvider only once.
//
// With this subclass the update provider has its own instance and its own
// authority. Verified on a real Samsung phone: Inhouse Notes 1.0.27 installed
// 1.0.28 from inside the app.
//
// Keep it registered by tools/register_android.py exactly as
//   <provider android:name="com.inhouse.services.updater.UpdateFileProvider"
//             android:authorities="${applicationId}.update-provider"
//             android:exported="false" android:grantUriPermissions="true">
// and NEVER register ".update-provider" with androidx.core.content.FileProvider
// or any class that another provider of the app also uses.
// Canonical code: github.com/miguelcoxcaballero/inhouse-services-updater.
public class UpdateFileProvider extends androidx.core.content.FileProvider {}
