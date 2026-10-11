package com.inhouse.services.updater;

import android.app.Activity;
import android.content.ClipData;
import android.content.Intent;
import android.net.Uri;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.view.Gravity;
import android.widget.LinearLayout;
import android.widget.ProgressBar;
import android.widget.TextView;

/** Foreground proxy that retains Android's installer result and verifies the package. */
public final class InstallActivity extends Activity {
    private static final int INSTALL_REQUEST = 4817;
    private final Handler main = new Handler(Looper.getMainLooper());
    private String attemptId;
    private boolean launched;
    private boolean leftForInstaller;
    private boolean returned;
    private boolean resumed;

    @Override public void onCreate(Bundle saved) {
        super.onCreate(saved);
        attemptId = getIntent().getStringExtra("installId");
        long expected = getIntent().getLongExtra("versionCode", 0);
        Uri uri = getIntent().getData();
        if (attemptId == null || expected <= 0 || uri == null || !"content".equals(uri.getScheme())
                || !(getPackageName() + ".update-provider").equals(uri.getAuthority())) {
            finish(); return;
        }
        try {
            if (saved == null) {
                InstallState.begin(this, attemptId, expected, getIntent().getStringExtra("versionName"));
            } else {
                if (!InstallState.isCurrent(this, attemptId)) { finish(); return; }
                launched = saved.getBoolean("launched");
                leftForInstaller = saved.getBoolean("leftForInstaller");
            }
            if (InstallState.actualVersion(this) >= expected) {
                complete("installed", "Update installed."); return;
            }
            showProgress();
            if (!launched) {
                Intent install = new Intent(Intent.ACTION_INSTALL_PACKAGE).setData(uri)
                    .putExtra(Intent.EXTRA_RETURN_RESULT, true)
                    .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
                install.setClipData(ClipData.newRawUri("Update APK", uri));
                launched = true;
                startActivityForResult(install, INSTALL_REQUEST);
            }
        } catch (Exception error) {
            complete("error", "Android could not open the installer. Try again.");
        }
    }

    private void showProgress() {
        LinearLayout content = new LinearLayout(this);
        content.setOrientation(LinearLayout.VERTICAL);
        content.setGravity(Gravity.CENTER);
        int padding = (int) (24 * getResources().getDisplayMetrics().density);
        content.setPadding(padding, padding, padding, padding);
        content.addView(new ProgressBar(this));
        TextView text = new TextView(this);
        text.setText("Complete the installation in Android.");
        text.setGravity(Gravity.CENTER);
        text.setPadding(0, padding, 0, 0);
        content.addView(text);
        setContentView(content);
    }

    @Override protected void onActivityResult(int request, int result, Intent data) {
        super.onActivityResult(request, result, data);
        if (request != INSTALL_REQUEST || returned) return;
        returned = true;
        long expected = InstallState.expectedVersion(this, attemptId);
        String outcome = InstallResultPolicy.callback(expected, InstallState.actualVersion(this), result);
        String message = "installed".equals(outcome) ? "Update installed."
            : "cancelled".equals(outcome) ? "Installation cancelled. You can try again."
            : result == RESULT_OK ? "Could not confirm the installation. Try again."
            : InstallResultPolicy.failureMessage(data == null ? 0
                : data.getIntExtra("android.intent.extra.INSTALL_RESULT", 0));
        complete(outcome, message);
    }

    private void complete(String status, String message) {
        returned = true;
        try { InstallState.finish(this, attemptId, status, message); }
        catch (RuntimeException ignored) { /* The getter retains an in-memory storage error. */ }
        finally { finish(); }
    }

    @Override protected void onPause() {
        resumed = false;
        if (launched) leftForInstaller = true;
        super.onPause();
    }

    @Override protected void onResume() {
        super.onResume();
        resumed = true;
        checkInstallerReturn();
    }

    @Override public void onWindowFocusChanged(boolean focused) {
        super.onWindowFocusChanged(focused);
        if (focused) checkInstallerReturn();
    }

    private void checkInstallerReturn() {
        if (!launched || !leftForInstaller || returned) return;
        // Some OEM installers lose Activity results. A return without a result
        // is unknown, not proof of success or cancellation; leave retry usable.
        main.postDelayed(() -> {
            if (!resumed || !hasWindowFocus() || returned || isFinishing() || isDestroyed()) return;
            long expected = InstallState.expectedVersion(this, attemptId);
            if (expected > 0 && InstallState.actualVersion(this) >= expected) {
                complete("installed", "Update installed.");
            } else {
                complete("error", "Could not confirm the installation. Try again.");
            }
        }, 500);
    }

    @Override public void onBackPressed() {
        complete("error", "Could not confirm the installation. Try again.");
    }

    @Override protected void onSaveInstanceState(Bundle out) {
        out.putBoolean("launched", launched);
        out.putBoolean("leftForInstaller", leftForInstaller);
        super.onSaveInstanceState(out);
    }

    @Override protected void onDestroy() {
        main.removeCallbacksAndMessages(null);
        if (isFinishing() && !returned) {
            // Also cover system/predictive Back that bypasses onBackPressed.
            try { InstallState.finish(this, attemptId, "error", "Could not confirm the installation. Try again."); }
            catch (RuntimeException ignored) {}
        }
        super.onDestroy();
    }
}
