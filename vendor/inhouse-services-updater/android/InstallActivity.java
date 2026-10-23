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
import android.provider.Settings;
import java.lang.ref.WeakReference;
import android.view.Gravity;
import android.widget.Button;
import android.widget.LinearLayout;
import android.widget.ProgressBar;
import android.widget.TextView;
import java.io.InputStream;
import java.io.OutputStream;
import java.security.MessageDigest;

/** Foreground staging UI. PackageInstaller owns the APK and the terminal result. */
public final class InstallActivity extends Activity {
    private final Handler main = new Handler(Looper.getMainLooper());
    private static WeakReference<InstallActivity> foreground = new WeakReference<>(null);
    private static String confirmationId;
    private static Intent confirmationIntent;
    private static final int PERMISSION_REQUEST = 41, CONSENT_REQUEST = 42;
    private String attemptId;
    private boolean permissionOpened;
    private boolean permissionWaiting;
    private TextView statusText;
    private ProgressBar spinner;
    private Button retry;
    private long waitingSince;

    // PackageInstaller delivers this on the main thread. Never start consent
    // from a BroadcastReceiver: newer Android/OEM builds can silently block it.
    public static void requestConfirmation(String id, Intent intent) {
        confirmationId = id; confirmationIntent = intent;
        InstallActivity activity = foreground.get();
        if (activity != null) activity.openConfirmation();
    }
    private void openConfirmation() {
        if (!resumed || returned || isFinishing() || !attemptId.equals(confirmationId) || confirmationIntent == null) return;
        Intent intent = confirmationIntent;
        confirmationIntent = null; confirmationId = null;
        try {
            intent.setFlags(intent.getFlags() & ~Intent.FLAG_ACTIVITY_NEW_TASK);
            startActivityForResult(intent, CONSENT_REQUEST);
            waitingSince = android.os.SystemClock.elapsedRealtime();
        } catch (Exception error) { complete("error", "Android could not open installation confirmation. Try again."); }
    }
    private boolean launched;
    private volatile boolean returned;
    private volatile int sessionId = -1;
    private boolean resumed;
    private final Runnable check = new Runnable() {
        @Override public void run() {
            if (!resumed || returned || isFinishing()) return;
            String state = InstallState.status(InstallActivity.this, attemptId);
            if ("installed".equals(state) || "error".equals(state) || "cancelled".equals(state)) {
                returned = true;
                showOutcome(state, InstallState.message(InstallActivity.this, attemptId)); return;
            }
            if (launched && waitingSince > 0 && android.os.SystemClock.elapsedRealtime() - waitingSince > 90_000) {
                complete("error", "Android has not confirmed installation. Try again."); return;
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
        // Changing REQUEST_INSTALL_PACKAGES kills the app process on Android 11.
        // A retry updates this Activity's in-memory Intent, but Android restores
        // the original launch Intent after death. Recover the current attempt.
        attemptId = saved != null ? saved.getString("installId", getIntent().getStringExtra("installId"))
            : getIntent().getStringExtra("installId");
        getIntent().putExtra("installId", attemptId);
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
                permissionOpened = saved.getBoolean("permissionOpened");
                permissionWaiting = saved.getBoolean("permissionWaiting");
                waitingSince = saved.getLong("waitingSince", 0);
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
        waitingSince = android.os.SystemClock.elapsedRealtime();
        final String jobId = attemptId;
        final Uri apkUri = getIntent().getData();
        final String expectedHash = getIntent().getStringExtra("sha256");
        new Thread(() -> {
            PackageInstaller installer = getPackageManager().getPackageInstaller();
            int created = -1;
            boolean committed = false;
            try {
                PackageInstaller.SessionParams params = new PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL);
                params.setAppPackageName(getPackageName());
                if (Build.VERSION.SDK_INT >= 31) params.setRequireUserAction(PackageInstaller.SessionParams.USER_ACTION_REQUIRED);
                created = installer.createSession(params);
                if (!InstallState.isCurrent(this, jobId)) throw new java.io.IOException("Installation interrupted");
                sessionId = created;
                try (PackageInstaller.Session session = installer.openSession(created)) {
                    try (InputStream input = getContentResolver().openInputStream(apkUri);
                         OutputStream output = session.openWrite("base.apk", 0, -1)) {
                        if (input == null) throw new java.io.IOException("Missing APK");
                        MessageDigest digest = MessageDigest.getInstance("SHA-256");
                        byte[] buffer = new byte[64 * 1024];
                        int count;
                        while ((count = input.read(buffer)) != -1) {
                            if (returned || !jobId.equals(attemptId)) throw new java.io.IOException("Installation interrupted");
                            digest.update(buffer, 0, count);
                            output.write(buffer, 0, count);
                        }
                        StringBuilder actualHash = new StringBuilder();
                        for (byte value : digest.digest()) actualHash.append(String.format(java.util.Locale.ROOT, "%02x", value & 255));
                        if (expectedHash == null || !expectedHash.equalsIgnoreCase(actualHash.toString())) {
                            throw new SecurityException("Update checksum changed. Download the update again.");
                        }
                        session.fsync(output);
                    }
                    if (returned || !InstallState.isCurrent(this, jobId) || !"installing".equals(InstallState.status(this, jobId))) throw new java.io.IOException("Installation interrupted");
                    Intent callback = new Intent(this, InstallReceiver.class)
                        .setAction(getPackageName() + ".INSTALL_RESULT." + jobId)
                        .putExtra("installId", jobId);
                    // The system fills EXTRA_STATUS and the confirmation Intent.
                    // Mutable is required for PackageInstaller, with an explicit component.
                    int flags = PendingIntent.FLAG_UPDATE_CURRENT;
                    if (Build.VERSION.SDK_INT >= 31) flags |= PendingIntent.FLAG_MUTABLE;
                    PendingIntent result = PendingIntent.getBroadcast(this, created, callback, flags);
                    session.commit(result.getIntentSender());
                    committed = true;
                }
            } catch (Exception error) {
                main.post(() -> { if (!returned && jobId.equals(attemptId)) complete("error", "Android could not start the installation. Try again."); });
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
        spinner = new ProgressBar(this);
        content.addView(spinner);
        TextView text = statusText = new TextView(this);
        text.setText("Complete the installation in Android.");
        text.setGravity(Gravity.CENTER);
        text.setPadding(0, padding, 0, padding);
        content.addView(text);
        retry = new Button(this);
        retry.setText("Try again");
        retry.setVisibility(android.view.View.GONE);
        retry.setOnClickListener(view -> {
            if (sessionId >= 0) try { getPackageManager().getPackageInstaller().abandonSession(sessionId); } catch (Exception ignored) {}
            sessionId = -1; launched = false; returned = false; permissionOpened = false;
            attemptId = java.util.UUID.randomUUID().toString();
            getIntent().putExtra("installId", attemptId);
            try { InstallState.begin(this, attemptId, getIntent().getLongExtra("versionCode", 0), getIntent().getStringExtra("versionName")); }
            catch (RuntimeException error) { complete("error", "Could not save installation state. Check available storage."); return; }
            retry.setVisibility(android.view.View.GONE); spinner.setVisibility(android.view.View.VISIBLE);
            statusText.setText("Preparing update…");
            continueInstallation(); main.removeCallbacks(check); main.post(check);
        });
        content.addView(retry);
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
        finally { showOutcome(status, message); }
    }
    private void showOutcome(String status, String message) {
        if ("installed".equals(status)) { finish(); return; }
        if (statusText == null) showProgress();
        spinner.setVisibility(android.view.View.GONE);
        statusText.setText(message);
        retry.setVisibility(android.view.View.VISIBLE);
    }
    private void continueInstallation() {
        if (launched || returned || isFinishing()) return;
        boolean allowed = Build.VERSION.SDK_INT < 26 || getPackageManager().canRequestPackageInstalls();
        if (allowed) permissionWaiting = false; // The result callback may be lost with the old process.
        if (permissionWaiting) return;
        if (!allowed) {
            if (permissionOpened) {
                complete("error", "Allow updates from this app in Android settings, then try again."); return;
            }
            permissionOpened = true; permissionWaiting = true;
            statusText.setText("Allow updates from this app in Android settings.");
            try { startActivityForResult(new Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,
                Uri.parse("package:" + getPackageName())), PERMISSION_REQUEST); }
            catch (Exception error) { permissionWaiting = false; complete("error", "Android could not open installation settings. Try again."); }
            return;
        }
        // Rebind durable state to this process after returning from Settings.
        try { InstallState.begin(this, attemptId, getIntent().getLongExtra("versionCode", 0), getIntent().getStringExtra("versionName")); }
        catch (RuntimeException error) { complete("error", "Could not save installation state. Check available storage."); return; }
        stage();
    }
    @Override protected void onActivityResult(int request, int result, Intent data) {
        super.onActivityResult(request, result, data);
        if (request == PERMISSION_REQUEST) { permissionWaiting = false; if (resumed) continueInstallation(); }
        // The consent Activity can return RESULT_CANCELED before the session
        // commits. Only the PackageInstaller callback or installed version wins.
    }
    @Override protected void onResume() {
        super.onResume(); resumed = true;
        // Start only once this Activity is foreground, so Android can show consent.
        foreground = new WeakReference<>(this);
        if (launched) waitingSince = android.os.SystemClock.elapsedRealtime();
        if (attemptId != null) { continueInstallation(); openConfirmation(); }
        main.removeCallbacks(check); main.post(check);
    }
    @Override protected void onPause() {
        resumed = false; main.removeCallbacks(check); super.onPause();
    }
    @Override public void onBackPressed() {
        if (sessionId >= 0) {
            try { getPackageManager().getPackageInstaller().abandonSession(sessionId); } catch (Exception ignored) {}
        }
        try { if (!returned) InstallState.finish(this, attemptId, "cancelled", "Installation cancelled. You can try again."); }
        catch (RuntimeException ignored) {}
        returned = true; finish();
    }
    @Override protected void onSaveInstanceState(Bundle out) {
        out.putString("installId", attemptId);
        out.putBoolean("launched", launched); out.putInt("sessionId", sessionId);
        out.putBoolean("permissionOpened", permissionOpened); out.putBoolean("permissionWaiting", permissionWaiting);
        out.putLong("waitingSince", waitingSince);
        super.onSaveInstanceState(out);
    }
    @Override protected void onDestroy() {
        main.removeCallbacksAndMessages(null);
        if (foreground.get() == this) foreground.clear();
        // Configuration changes retain the pending confirmation for onResume.
        if (isFinishing() && attemptId != null && attemptId.equals(confirmationId)) {
            confirmationIntent = null; confirmationId = null;
        }
        super.onDestroy();
    }
}
