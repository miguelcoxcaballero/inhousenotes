package com.inhouse.services.updater;

/**
 * The updater's own FileProvider class for "${applicationId}.update-provider".
 * Android keeps one provider instance per class. When a host already registers
 * androidx FileProvider (Capacitor's ".fileprovider" in Notes and Read), that
 * instance also answers ".update-provider" and rejects its URIs because the
 * authority does not match, so the package installer cannot read the update.
 * Canonical code: github.com/miguelcoxcaballero/inhouse-services-updater.
 */
public class UpdateFileProvider extends androidx.core.content.FileProvider {}
