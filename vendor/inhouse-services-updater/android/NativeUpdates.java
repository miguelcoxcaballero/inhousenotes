package com.inhouse.services.updater;

import android.app.Activity;
import android.content.Context;
import android.content.Intent;
import android.content.MutableContextWrapper;
import android.os.Handler;
import android.os.Looper;
import android.util.Log;
import android.view.View;
import android.view.ViewGroup;
import android.view.ViewParent;
import android.view.ViewTreeObserver;
import android.webkit.JavascriptInterface;
import android.webkit.RenderProcessGoneDetail;
import android.webkit.WebSettings;
import android.webkit.WebView;


import java.lang.ref.WeakReference;

/**
 * Native Activity host for the existing Inhouse Read checker and Photos popup.
 * The checker configuration and bridges come from MainActivity.startUpdateChecks;
 * version comparison, network requests, popup and installation use Inhouse services updater.
 * Change the central repository and sync all five apps. A by default; B only on explicit request.
 * YouTube itself never receives a JavaScript bridge or a WebView interface.
 */
public final class NativeUpdates {
    private final Context application;
    private final Handler main = new Handler(Looper.getMainLooper());
    private WeakReference<Activity> host = new WeakReference<>(null);
    private WeakReference<ViewTreeObserver> focusObserver = new WeakReference<>(null);
    private MutableContextWrapper checkerContext;
    private WebView updateChecker;
    private String pendingUpdate = "";
    private boolean resumed;
    private String offeredKey="";
    private int generation;

    private final ViewTreeObserver.OnWindowFocusChangeListener focusListener = focus -> {
        if (focus) showPendingUpdate();
    };

    public NativeUpdates(Context context) {
        application = context.getApplicationContext();
    }

    public void onResumed(Activity activity) {
        if (activity instanceof UpdateActivity || activity.isFinishing() || activity.isDestroyed()) return;
        try {
            boolean created = updateChecker == null;
            startUpdateChecks();
            if (host.get() != activity) {
                detachFromHost();
                host = new WeakReference<>(activity);
                checkerContext.setBaseContext(activity);
                View content = activity.findViewById(android.R.id.content);
                ViewGroup parent = content instanceof ViewGroup
                    ? (ViewGroup) content : (ViewGroup) activity.getWindow().getDecorView();
                parent.addView(updateChecker, new ViewGroup.LayoutParams(1, 1));
                ViewTreeObserver observer = activity.getWindow().getDecorView().getViewTreeObserver();
                observer.addOnWindowFocusChangeListener(focusListener);
                focusObserver = new WeakReference<>(observer);
            }
            resumed = true;
            updateChecker.onResume();
            if (created) {
                updateChecker.loadUrl("https://appassets.androidplatform.net/updates/index.html?inhouse_app=1&quiet=1");
            } else {
                updateChecker.evaluateJavascript("window.dispatchEvent(new Event('focus'));", null);
            }
            showPendingUpdate();
        } catch (RuntimeException error) {
            // An unavailable Android System WebView must not stop the native app.
            Log.w("CalmaUpdates", "Unable to start update checker", error);
            destroyChecker();
        }
    }

    public void onPaused(Activity activity) {
        if (host.get() != activity) return;
        resumed = false;
        if (updateChecker != null) updateChecker.onPause();
    }

    public void onDestroyed(Activity activity) {
        if (host.get() == activity) destroyChecker();
    }

    public void onMemoryPressure() {
        if (!resumed) destroyChecker();
    }

    private void startUpdateChecks() {
        if (updateChecker != null) return;
        checkerContext = new MutableContextWrapper(application);
        updateChecker = new WebView(checkerContext);
        updateChecker.setVisibility(View.INVISIBLE);
        updateChecker.setFocusable(false);
        updateChecker.setImportantForAccessibility(View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS);
        updateChecker.getSettings().setJavaScriptEnabled(true);
        updateChecker.getSettings().setDomStorageEnabled(true);
        updateChecker.getSettings().setAllowFileAccess(false);
        updateChecker.getSettings().setAllowContentAccess(false);
        updateChecker.getSettings().setMixedContentMode(WebSettings.MIXED_CONTENT_NEVER_ALLOW);
        updateChecker.getSettings().setUserAgentString(updateChecker.getSettings().getUserAgentString()
            + " InhouseReadApp/" + UpdateState.version(application));
        int currentGeneration = ++generation;
        updateChecker.addJavascriptInterface(new UpdateCheckBridge(), "InhouseNative");
        updateChecker.addJavascriptInterface(new UpdateOfferBridge(currentGeneration), "InhouseUpdateHost");
        updateChecker.setWebViewClient(new UpdateAssetClient(application) {
            @Override public boolean onRenderProcessGone(WebView view, RenderProcessGoneDetail detail) {
                if (view == updateChecker) destroyChecker();
                return true;
            }
        });
    }

    public final class UpdateCheckBridge {
        @JavascriptInterface public String getAppVersion() { return UpdateState.version(application); }
        @JavascriptInterface public long getAppVersionCode() { return UpdateState.versionCode(application); }
        @JavascriptInterface public String getUpdateState() { return UpdateState.read(application); }
        @JavascriptInterface public void saveUpdateState(String value) { UpdateState.save(application,value); }
    }

    public final class UpdateOfferBridge {
        private final int checkerGeneration;
        UpdateOfferBridge(int value) { checkerGeneration = value; }
        @JavascriptInterface public void offer(String manifest) {
            main.post(() -> {
                if (checkerGeneration != generation || updateChecker == null || manifest == null) return;
                pendingUpdate = manifest;
                showPendingUpdate();
            });
        }
    }

    private void showPendingUpdate() {
        Activity activity = host.get();
        if (pendingUpdate.isEmpty() || !UpdateState.accepts(application,pendingUpdate) || !resumed || activity == null
                || !activity.hasWindowFocus() || activity.isFinishing() || activity.isDestroyed()) return;
        try {
            String key=UpdateState.offerKey(application,pendingUpdate);
            if(key.equals(offeredKey))return;
            activity.startActivity(new Intent().setClassName(activity.getPackageName(),UpdateState.config(application).optString("activityClass",UpdateActivity.class.getName())).putExtra("manifest", pendingUpdate));
            offeredKey = key;
        } catch (RuntimeException error) {
            Log.w("CalmaUpdates", "Unable to show update popup", error);
        }
    }

    private void detachFromHost() {
        ViewTreeObserver observer = focusObserver.get();
        if (observer != null && observer.isAlive()) observer.removeOnWindowFocusChangeListener(focusListener);
        focusObserver.clear();
        if (updateChecker != null) {
            ViewParent parent = updateChecker.getParent();
            if (parent instanceof ViewGroup) ((ViewGroup) parent).removeView(updateChecker);
        }
        if (checkerContext != null) checkerContext.setBaseContext(application);
        host.clear();
        resumed = false;
    }

    private void destroyChecker() {
        ++generation;
        detachFromHost();
        WebView checker = updateChecker;
        updateChecker = null;
        if (checker != null) {
            checker.removeJavascriptInterface("InhouseNative");
            checker.removeJavascriptInterface("InhouseUpdateHost");
            checker.stopLoading();
            checker.destroy();
        }
        checkerContext = null;
    }
}
