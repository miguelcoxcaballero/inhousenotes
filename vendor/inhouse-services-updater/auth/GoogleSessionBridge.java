package com.inhouse.services.oauth;

import android.content.Context;
import android.webkit.WebView;
import androidx.webkit.WebViewCompat;
import androidx.webkit.WebViewFeature;
import java.util.Set;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import org.json.JSONObject;

/** Origin AND main-frame restricted bridge. Never exposes a JavascriptInterface to child frames. */
public final class GoogleSessionBridge {
    private static final ExecutorService IO = Executors.newSingleThreadExecutor();
    public static void install(WebView webView, Set<String> origins) {
        if (!WebViewFeature.isFeatureSupported(WebViewFeature.WEB_MESSAGE_LISTENER)) return;
        Context context = webView.getContext().getApplicationContext();
        WebViewCompat.addWebMessageListener(webView, "InhouseOAuthNative", origins,
            (view, message, origin, mainFrame, reply) -> {
                if (!mainFrame || message.getData() == null) return;
                if ("com.inhousesoftware.read".equals(context.getPackageName())) {
                    android.net.Uri page = android.net.Uri.parse(view.getUrl() == null ? "" : view.getUrl());
                    if (!"https".equals(page.getScheme()) || !"miguelcoxcaballero.github.io".equals(page.getHost())
                        || page.getPath() == null || !page.getPath().startsWith("/inhouse-read/")) return;
                }
                final String raw = message.getData();
                IO.execute(() -> {
                    JSONObject result = new JSONObject();
                    try {
                        JSONObject request = new JSONObject(raw);
                        result.put("id", request.getString("id"));
                        switch (request.getString("method")) {
                            case "read": String saved = GoogleSessionProvider.discover(context); result.put("session", "null".equals(saved) ? JSONObject.NULL : new JSONObject(saved)); break;
                            case "write": GoogleSessionProvider.write(context, request.getJSONObject("session").toString()); break;
                            case "clear": GoogleSessionProvider.write(context, null); break;
                            default: throw new IllegalArgumentException();
                        }
                    } catch (Exception error) {
                        try { result.put("error", "Google session unavailable"); } catch (Exception ignored) { }
                    }
                    view.post(() -> reply.postMessage(result.toString()));
                });
            });
    }
}
