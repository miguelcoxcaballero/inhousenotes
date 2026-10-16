package com.local.inhousenotes;

import android.content.Context;
import android.content.res.AssetManager;
import android.net.Uri;
import android.os.Build;
import android.webkit.WebResourceRequest;
import android.webkit.WebResourceResponse;
import android.webkit.WebView;
import com.getcapacitor.Bridge;
import com.getcapacitor.BridgeWebViewClient;
import java.io.InputStream;
import java.io.ByteArrayOutputStream;
import java.util.HashMap;
import java.util.Map;
import org.json.JSONObject;

/** Immutable public code only. No documents, OAuth or account data cross this path. */
public final class InhouseLocalAssets {
    private final AssetManager assets;
    private final JSONObject manifest;
    private InhouseLocalAssets(Context context) throws Exception {
        assets=context.getApplicationContext().getAssets();
        try(InputStream input=assets.open("inhouse-native/native-assets.json")) {
            ByteArrayOutputStream bytes=new ByteArrayOutputStream();byte[] buffer=new byte[8192];int count;
            while((count=input.read(buffer))!=-1)bytes.write(buffer,0,count);
            manifest=new JSONObject(bytes.toString("UTF-8")).getJSONObject("assets");
        }
    }
    public static void attach(Bridge bridge) {
        final WebView view=bridge.getWebView();
        // Retain the foreground renderer, but let Android reclaim it when hidden.
        if(Build.VERSION.SDK_INT>=26)view.setRendererPriorityPolicy(WebView.RENDERER_PRIORITY_IMPORTANT,true);
        if(Build.VERSION.SDK_INT>=23)view.getSettings().setOffscreenPreRaster(false);
        try {
            final InhouseLocalAssets local=new InhouseLocalAssets(view.getContext());
            bridge.setWebViewClient(new BridgeWebViewClient(bridge) {
                @Override public WebResourceResponse shouldInterceptRequest(WebView webView,WebResourceRequest request) {
                    WebResourceResponse response=local.intercept(request);
                    return response!=null?response:super.shouldInterceptRequest(webView,request);
                }
            });
        }catch(Exception ignored) { /* An unavailable bundle must not prevent normal loading. */ }
    }
    // WebView invokes this on its request thread, not the UI/pen-input thread.
    WebResourceResponse intercept(WebResourceRequest request) {
        Uri uri=request.getUrl();
        if(!"GET".equals(request.getMethod())||request.isForMainFrame()||!"https".equals(uri.getScheme())
            ||!"inhousenotes.com".equals(uri.getHost())||(uri.getPort()!=-1&&uri.getPort()!=443))return null;
        String path=uri.getPath(),digest=uri.getQueryParameter("native");
        if(path==null||path.contains("..")||digest==null)return null;
        JSONObject entry=manifest.optJSONObject(path);
        if(entry==null||!digest.equals(entry.optString("sha256")))return null;
        try {
            InputStream input=assets.open("inhouse-native"+path,AssetManager.ACCESS_STREAMING);
            Map<String,String> headers=new HashMap<>();
            headers.put("Cache-Control","public, max-age=31536000, immutable");
            headers.put("Content-Length",String.valueOf(entry.getLong("size")));
            headers.put("X-Content-Type-Options","nosniff");
            headers.put("X-Inhouse-Asset-Source","apk");
            String mime=entry.getString("mime");
            String encoding=mime.startsWith("text/")||mime.contains("javascript")||mime.contains("json")||mime.contains("svg")?"UTF-8":null;
            return new WebResourceResponse(mime,encoding,200,"OK",headers,input);
        }catch(Exception ignored){return null;}
    }
}
