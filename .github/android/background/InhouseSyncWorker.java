package com.local.inhousenotes;

import android.content.Context;
import android.net.Uri;
import android.os.Handler;
import android.os.Looper;
import android.webkit.WebView;
import android.webkit.WebViewClient;
import android.webkit.WebResourceRequest;
import androidx.annotation.NonNull;
import androidx.work.Worker;
import androidx.work.WorkerParameters;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;

/** A separate editor database uses the same origin, credentials and durable outbox.
 * No Activity is required. WorkManager restores this work after process death/reboot.
 */
public final class InhouseSyncWorker extends Worker {
    private final Handler main = new Handler(Looper.getMainLooper());
    private final CountDownLatch done = new CountDownLatch(1);
    private final AtomicBoolean success = new AtomicBoolean(false);
    private WebView webView;
    public InhouseSyncWorker(@NonNull Context context, @NonNull WorkerParameters params) { super(context, params); }
    @NonNull @Override public Result doWork() {
        main.post(() -> {
            if (isStopped()) { done.countDown(); return; }
            try {
                webView = new WebView(getApplicationContext());
                webView.getSettings().setJavaScriptEnabled(true);
                webView.getSettings().setDomStorageEnabled(true);
                webView.getSettings().setDatabaseEnabled(true);
                webView.getSettings().setAllowFileAccess(false);
                webView.getSettings().setAllowContentAccess(false);
                webView.getSettings().setUserAgentString(webView.getSettings().getUserAgentString() + " InhouseNotesApp/Background");
                webView.setWebViewClient(new WebViewClient() {
                    @Override public boolean shouldOverrideUrlLoading(WebView view, WebResourceRequest request) {
                        Uri url = request.getUrl();
                        return !"https".equals(url.getScheme()) || !"inhousenotes.com".equals(url.getHost());
                    }
                });
                InhouseBackgroundSync.attach(getApplicationContext(), webView, result -> {
                    success.set(result != null && !result.optBoolean("busy") && result.optInt("pending", 0) == 0
                        && (result.optJSONArray("errors") == null || result.optJSONArray("errors").length() == 0));
                    done.countDown();
                });
                webView.loadUrl("https://inhousenotes.com/?inhouse_app=1&offline-worker=1");
            } catch (Exception error) { done.countDown(); }
        });
        try { done.await(8, TimeUnit.MINUTES); }
        catch (InterruptedException error) { Thread.currentThread().interrupt(); }
        finally { destroy(); }
        return success.get() ? Result.success() : Result.retry();
    }
    private void destroy() { main.post(() -> { if (webView != null) {webView.stopLoading();webView.destroy();webView=null;} }); }
    @Override public void onStopped() { done.countDown();destroy();super.onStopped(); }
}
