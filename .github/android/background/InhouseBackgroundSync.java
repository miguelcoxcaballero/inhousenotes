package com.local.inhousenotes;

import android.content.Context;
import android.webkit.WebView;
import androidx.webkit.WebViewCompat;
import androidx.webkit.WebViewFeature;
import androidx.work.*;
import java.util.Collections;
import java.util.concurrent.TimeUnit;
import org.json.JSONObject;

/** Only the first-party top-level page can schedule account synchronization. */
public final class InhouseBackgroundSync {
    private static final String ONCE = "inhouse-offline-outbox";
    private static final String PERIODIC = "inhouse-offline-retry";
    public interface Completion { void complete(JSONObject result); }
    public static void attach(Context context, WebView webView) { attach(context, webView, null); }
    public static void attach(Context context, WebView webView, Completion completion) {
        if (!WebViewFeature.isFeatureSupported(WebViewFeature.WEB_MESSAGE_LISTENER)) return;
        Context app = context.getApplicationContext();
        WebViewCompat.addWebMessageListener(webView, "InhouseBackgroundChannel",
            Collections.singleton("https://inhousenotes.com"), (view, message, origin, mainFrame, reply) -> {
                if (!mainFrame || !"https".equals(origin.getScheme()) || !"inhousenotes.com".equals(origin.getHost())) return;
                try {
                    JSONObject data = new JSONObject(message.getData());
                    switch (data.optString("type")) {
                        case "schedule": if (completion == null) schedule(app); break;
                        case "cancel": if (completion == null) cancel(app); break;
                        case "complete": if (completion != null) completion.complete(data.optJSONObject("result")); break;
                    }
                } catch (Exception ignored) { /* An invalid message cannot alter the queue. */ }
            });
    }
    public static void schedule(Context context) {
        Constraints online = new Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build();
        WorkManager work = WorkManager.getInstance(context);
        work.enqueueUniqueWork(ONCE, ExistingWorkPolicy.KEEP,
            new OneTimeWorkRequest.Builder(InhouseSyncWorker.class).setConstraints(online)
                .setBackoffCriteria(BackoffPolicy.EXPONENTIAL, 30, TimeUnit.SECONDS).build());
        work.enqueueUniquePeriodicWork(PERIODIC, ExistingPeriodicWorkPolicy.KEEP,
            new PeriodicWorkRequest.Builder(InhouseSyncWorker.class, 15, TimeUnit.MINUTES)
                .setConstraints(online).build());
    }
    public static void cancel(Context context) {
        WorkManager work = WorkManager.getInstance(context);
        work.cancelUniqueWork(ONCE); work.cancelUniqueWork(PERIODIC);
    }
    private InhouseBackgroundSync() {}
}
