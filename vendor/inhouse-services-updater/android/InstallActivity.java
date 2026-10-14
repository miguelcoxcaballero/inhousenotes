package com.inhouse.services.updater;

import android.app.Activity;
import android.app.PendingIntent;
import android.content.Intent;
import android.content.pm.PackageInstaller;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.view.Gravity;
import android.widget.Button;
import android.widget.LinearLayout;
import android.widget.ProgressBar;
import android.widget.TextView;
import java.io.InputStream;
import java.io.OutputStream;

/** Foreground staging UI. PackageInstaller owns the APK and the terminal result. */
public final class InstallActivity extends Activity {
    private final Handler main = new Handler(Looper.getMainLooper());
    private String attemptId;
    private boolean launched;
    private volatile boolean returned;
    private volatile int sessionId = -1;
    private boolean resumed;
    private final Runnable check = new Runnable() {
        @Override public void run() {
            if (!resumed || returned || isFinishing()) return;
            String state = InstallState.status(InstallActivity.this, attemptId);
            if ("installed".equals(state) || "error".equals(state) || "cancelled".equals(state)) {
                returned = true; finish(); return;
            }
            // A self-update may kill the process before the receiver executes.
            long expected = InstallState.expectedVersion(InstallActivity.this, attemptId);
            if (expected > 0 && InstallState.actualVersion(InstallActivity.this) >= expected) {
                complete("installed", "Update installed."); return;
            }
            main.postDelayed(this, 500);
        }
    };

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
            if (saved == null) InstallState.begin(this, attemptId, expected, getIntent().getStringExtra("versionName"));
            else {
                if (!InstallState.isCurrent(this, attemptId)) { finish(); return; }
                launched = saved.getBoolean("launched");
                sessionId = saved.getInt("sessionId", -1);
            }
            if (InstallState.actualVersion(this) >= expected) {
                complete("installed", "Update installed."); return;
            }
            showProgress();
        } catch (Exception error) {
            complete("error", "Android could not prepare the installer. Try again.");
        }
    }

    private void stage() {
        launched = true;
        new Thread(() -> {
            PackageInstaller installer = getPackageManager().getPackageInstaller();
            int created = -1;
            boolean committed = false;
            try {
                PackageInstaller.SessionParams params = new PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL);
                params.setAppPackageName(getPackageName());
                if (Build.VERSION.SDK_INT >= 31) params.setRequireUserAction(PackageInstaller.SessionParams.USER_ACTION_REQUIRED);
                created = installer.createSession(params);
                sessionId = created;
                try (PackageInstaller.Session session = installer.openSession(created)) {
                    try (InputStream input = getContentResolver().openInputStream(getIntent().getData());
                         OutputStream output = session.openWrite("base.apk", 0, -1)) {
                        if (input == null) throw new java.io.IOException("Missing APK");
                        byte[] buffer = new byte[64 * 1024];
                        int count;
                        while ((count = input.read(buffer)) != -1) {
                            if (returned) throw new java.io.IOException("Installation interrupted");
                            output.write(buffer, 0, count);
                        }
                        session.fsync(output);
                    }
                    if (returned || !InstallState.isCurrent(this, attemptId) || !"installing".equals(InstallState.status(this, attemptId))) throw new java.io.IOException("Installation interrupted");
                    Intent callback = new Intent(this, InstallReceiver.class)
                        .setAction(getPackageName() + ".INSTALL_RESULT." + attemptId)
                        .putExtra("installId", attemptId);
                    // The system fills EXTRA_STATUS and the confirmation Intent.
                    // Mutable is required for PackageInstaller, with an explicit component.
                    int flags = PendingIntent.FLAG_UPDATE_CURRENT;
                    if (Build.VERSION.SDK_INT >= 31) flags |= PendingIntent.FLAG_MUTABLE;
                    PendingIntent result = PendingIntent.getBroadcast(this, created, callback, flags);
                    session.commit(result.getIntentSender());
                    committed = true;
                }
            } catch (Exception error) {
                main.post(() -> { if (!returned) complete("error", "Android could not start the installation. Use Download APK or try again."); });
            } finally {
                if (!committed && created >= 0) {
                    try { installer.abandonSession(created); } catch (Exception ignored) {}
                }
            }
        }, "InhouseInstallSession").start();
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
        text.setPadding(0, padding, 0, padding);
        content.addView(text);
        Button back = new Button(this);
        back.setText("Back to app");
        back.setOnClickListener(view -> onBackPressed());
        content.addView(back);
        setContentView(content);
    }

    private void complete(String status, String message) {
        returned = true;
        try { InstallState.finish(this, attemptId, status, message); }
        catch (RuntimeException ignored) {}
        finally { finish(); }
    }
    @Override protected void onResume() {
        super.onResume(); resumed = true;
        // Start only once this Activity is foreground, so Android can show consent.
        if (!launched && !returned && !isFinishing() && attemptId != null) stage();
        main.removeCallbacks(check); main.post(check);
    }
    @Override protected void onPause() {
        resumed = false; main.removeCallbacks(check); super.onPause();
    }
    @Override public void onBackPressed() {
        if (sessionId >= 0) {
            try { getPackageManager().getPackageInstaller().abandonSession(sessionId); } catch (Exception ignored) {}
        }
        complete("error", "Installation has not been confirmed. Use Download APK or try again.");
    }
    @Override protected void onSaveInstanceState(Bundle out) {
        out.putBoolean("launched", launched); out.putInt("sessionId", sessionId);
        super.onSaveInstanceState(out);
    }
    @Override protected void onDestroy() {
        main.removeCallbacksAndMessages(null);
        super.onDestroy();
    }
}
