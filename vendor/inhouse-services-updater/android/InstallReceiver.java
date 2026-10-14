package com.inhouse.services.updater;

import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageInstaller;

/** Explicit, non-exported receiver: survives self-update and reports session failures. */
public final class InstallReceiver extends BroadcastReceiver {
    @Override public void onReceive(Context context, Intent intent) {
        String id = intent.getStringExtra("installId");
        if (!InstallState.isCurrent(context, id)) return;
        int status = intent.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE);
        try {
            if (status == PackageInstaller.STATUS_PENDING_USER_ACTION) {
                Intent confirmation = intent.getParcelableExtra(Intent.EXTRA_INTENT);
                if (confirmation == null) throw new IllegalStateException("Missing confirmation");
                // This is the system-provided consent UI, never an APK VIEW intent.
                confirmation.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
                context.startActivity(confirmation);
                return;
            }
            String outcome = status == PackageInstaller.STATUS_SUCCESS ? "installed"
                : status == PackageInstaller.STATUS_FAILURE_ABORTED ? "cancelled" : "error";
            String message = InstallResultPolicy.sessionMessage(status);
            InstallState.finish(context, id, outcome, message);
        } catch (Exception error) {
            InstallState.finish(context, id, "error", "Android could not open the installation confirmation. Use Download APK.");
        }
    }
}
