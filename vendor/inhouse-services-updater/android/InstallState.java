package com.inhouse.services.updater;

import android.content.Context;
import android.content.pm.PackageInfo;
import android.os.Build;
import org.json.JSONObject;
import java.util.UUID;

/** Durable outcome of the latest install attempt, including self-update process death. */
public final class InstallState {
    private static final String PREFS = "inhouse-services-install";
    private static final String PROCESS = UUID.randomUUID().toString();
    private static String unsavedFailure;
    private InstallState() {}

    private static JSONObject load(Context context) {
        try {
            return new JSONObject(unsavedFailure != null ? unsavedFailure
                : context.getSharedPreferences(PREFS, 0).getString("attempt", "{}"));
        } catch (Exception ignored) { return new JSONObject(); }
    }

    private static void save(Context context, JSONObject attempt) {
        // An APK replacing this package can kill this process immediately.
        try {
            if (!context.getSharedPreferences(PREFS, 0).edit().putString("attempt", attempt.toString()).commit()) {
                throw new IllegalStateException("Could not save the installation result.");
            }
            unsavedFailure = null;
        } catch (RuntimeException error) {
            // Storage failure must still reach the current foreground UI. Do
            // not launch an install whose recovery state was not persisted.
            try {
                JSONObject failure = new JSONObject(attempt.toString());
                failure.put("status", "error").put("message", "Could not save the installation result. Check available storage and try again.");
                unsavedFailure = failure.toString();
            } catch (org.json.JSONException ignored) {}
            throw error;
        }
    }

    @SuppressWarnings("deprecation")
    public static long actualVersion(Context context) {
        try {
            PackageInfo info = context.getPackageManager().getPackageInfo(context.getPackageName(), 0);
            return Build.VERSION.SDK_INT >= 28 ? info.getLongVersionCode() : info.versionCode;
        } catch (Exception ignored) { return 0; }
    }

    public static synchronized void begin(Context context, String id, long expected, String version) {
        if (id == null || id.isEmpty() || expected <= 0) throw new IllegalArgumentException("Invalid update package.");
        try {
            JSONObject attempt = new JSONObject().put("id", id).put("status", "installing")
                .put("expectedVersionCode", expected).put("expectedVersion", version == null ? "" : version)
                .put("actualVersionCode", actualVersion(context)).put("message", "Complete the installation in Android.")
                .put("process", PROCESS).put("startedAt", System.currentTimeMillis());
            save(context, attempt);
        } catch (org.json.JSONException error) { throw new IllegalStateException(error); }
    }

    public static synchronized boolean isCurrent(Context context, String id) {
        return id != null && id.equals(load(context).optString("id"));
    }

    public static synchronized long expectedVersion(Context context, String id) {
        JSONObject attempt = load(context);
        return id != null && id.equals(attempt.optString("id")) ? attempt.optLong("expectedVersionCode") : 0;
    }

    public static synchronized String status(Context context, String id) {
        JSONObject attempt = load(context);
        return id != null && id.equals(attempt.optString("id")) ? attempt.optString("status") : "";
    }

    public static synchronized String message(Context context, String id) {
        JSONObject attempt = load(context);
        return id != null && id.equals(attempt.optString("id")) ? attempt.optString("message") : "Try again.";
    }

    public static synchronized void finish(Context context, String id, String status, String message) {
        JSONObject attempt = load(context);
        // A late callback from an older Activity must not overwrite a new retry.
        if (id == null || !id.equals(attempt.optString("id"))) return;
        long actual = actualVersion(context);
        if (attempt.optLong("expectedVersionCode") > 0 && actual >= attempt.optLong("expectedVersionCode")) {
            status = "installed";
            message = "Update installed.";
        } else if ("installed".equals(status)) {
            status = "error";
            message = "Could not confirm the installation. Try again.";
        }
        try {
            attempt.put("status", status).put("message", message).put("actualVersionCode", actual);
            save(context, attempt);
        } catch (org.json.JSONException error) { throw new IllegalStateException(error); }
    }

    public static synchronized String read(Context context) {
        JSONObject attempt = load(context);
        if (!attempt.has("id")) return "{}";
        long actual = actualVersion(context);
        String previous = attempt.optString("status");
        String status = InstallResultPolicy.reconcile(attempt.optLong("expectedVersionCode"), actual,
            previous, !PROCESS.equals(attempt.optString("process")));
        try {
            if (!status.equals(previous) || actual != attempt.optLong("actualVersionCode")) {
                attempt.put("status", status).put("actualVersionCode", actual);
                if ("installed".equals(status)) attempt.put("message", "Update installed.");
                else if ("error".equals(status) && !"error".equals(previous)) {
                    attempt.put("message", "Could not confirm the installation. Try again.");
                }
                try { save(context, attempt); }
                catch (RuntimeException error) { attempt = load(context); }
            }
            // Only the UI contract crosses the JavaScript bridge.
            attempt.remove("process");
            return attempt.toString();
        } catch (Exception ignored) {
            return "{}";
        }
    }
}
