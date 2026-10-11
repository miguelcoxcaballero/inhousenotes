package com.inhouse.services.updater;

import android.app.Activity;
import android.os.Bundle;
import android.os.Build;
import android.content.Intent;
import android.graphics.Color;
import android.content.res.Configuration;
import android.net.Uri;
import android.provider.Settings;
import android.webkit.*;
import org.json.JSONObject;

/** Inhouse services updater. Canonical code: github.com/miguelcoxcaballero/inhouse-services-updater.
 * Sync all apps after changes. New releases: A + tiny changelog; B only on explicit user request. */
public class UpdateActivity extends Activity {
    private WebView web;
    private PhotosUpdateInstaller photosInstaller;
    private long lastDownloaded=0,lastTotal=-1;
    @Override public void onCreate(Bundle saved) {
        super.onCreate(saved);
        web=new WebView(this) {
            private boolean closed=false;
            @Override public void evaluateJavascript(String script,ValueCallback<String> callback) {
                if(!closed && !UpdateActivity.this.isFinishing() && !UpdateActivity.this.isDestroyed())super.evaluateJavascript(script,callback);
            }
            @Override public void destroy(){if(!closed){closed=true;super.destroy();}}
        };
        boolean dark=(getResources().getConfiguration().uiMode & Configuration.UI_MODE_NIGHT_MASK)==Configuration.UI_MODE_NIGHT_YES;
        web.setBackgroundColor(getIntent().hasExtra("manifest")?Color.TRANSPARENT:(dark?Color.BLACK:Color.WHITE));
        web.getSettings().setJavaScriptEnabled(true); web.getSettings().setDomStorageEnabled(true);
        web.getSettings().setAllowFileAccess(false); web.getSettings().setAllowContentAccess(false);
        web.getSettings().setMixedContentMode(WebSettings.MIXED_CONTENT_NEVER_ALLOW);
        web.getSettings().setUserAgentString(web.getSettings().getUserAgentString()+" InhouseServicesApp/"+UpdateState.version(this));
        photosInstaller=new PhotosUpdateInstaller(this,new PhotosUpdateInstaller.Listener(){
            public void progress(long downloaded,long total){lastDownloaded=downloaded;lastTotal=total;bridge.notifyAppUpdateProgress(downloaded,total);}
            public void stage(String stage){if("verifying".equals(stage))bridge.notifyAppUpdateResult("downloading","Verificando…",lastDownloaded,lastTotal,100);}
        });
        web.addJavascriptInterface(bridge,"InhouseNative");
        web.setWebViewClient(new UpdateAssetClient(this) {
            @Override public boolean onRenderProcessGone(WebView view,RenderProcessGoneDetail detail) {
                view.destroy();finish();return true;
            }
        });
        setContentView(web);
        web.loadUrl("https://appassets.androidplatform.net/updates/index.html?inhouse_app=1"+(getIntent().getBooleanExtra("settings",false)?"&settings=1":""));
    }
    private final InhouseNativeBridge bridge=new InhouseNativeBridge();
    public class InhouseNativeBridge {
        @JavascriptInterface public String getAppVersion(){return UpdateState.version(UpdateActivity.this);}
        @JavascriptInterface public long getAppVersionCode(){return UpdateState.versionCode(UpdateActivity.this);}
        @JavascriptInterface public String getUpdateState(){return UpdateState.read(UpdateActivity.this);}
        @JavascriptInterface public void saveUpdateState(String value){UpdateState.save(UpdateActivity.this,value);}
        @JavascriptInterface public void closeUpdater(){runOnUiThread(()->finish());}
        @JavascriptInterface public String getPendingUpdateManifest(){String value=getIntent().getStringExtra("manifest");return value==null?"":value;}
        @JavascriptInterface public void installAppUpdate(String url,String hash){
            photosInstaller.installUpdate(url,hash,new PhotosUpdateInstaller.Result(){
                public void success(String status){if("ready".equals(status))notifyAppUpdateResult(status,"",lastDownloaded,lastTotal,100);else notifyAppUpdateResult(status,"");}
                public void error(String code,String message,Object details){notifyAppUpdateResult("error",message);}
            });
        }

        private void notifyAppUpdateResult(String status, String message) {
            notifyAppUpdateResult(status, message, -1, -1, -1);
        }

        private void notifyAppUpdateProgress(long downloadedBytes, long totalBytes) {
            int percent = totalBytes > 0
                ? (int) Math.min(100, (downloadedBytes * 100L) / totalBytes)
                : -1;
            notifyAppUpdateResult("downloading", "Downloading update", downloadedBytes, totalBytes, percent);
        }

        private void notifyAppUpdateResult(String status, String message,
                long downloadedBytes, long totalBytes, int percent) {
            JSONObject payload = new JSONObject();
            try {
                payload.put("status", status);
                payload.put("message", message == null ? "" : message);
                if (downloadedBytes >= 0) payload.put("downloadedBytes", downloadedBytes);
                if (totalBytes > 0) payload.put("totalBytes", totalBytes);
                if (percent >= 0) payload.put("percent", percent);
            } catch (Exception ignored) {}
            WebView webView = web;
            webView.post(() -> webView.evaluateJavascript(
                "window.handleInhouseUpdateResult && window.handleInhouseUpdateResult(" + payload.toString() + ");",
                null
            ));
        }
    }
    @Override public void onBackPressed(){if(getIntent().hasExtra("manifest"))UpdateState.snooze(this);finish();}
    @Override protected void onPause(){if(web!=null)web.onPause();super.onPause();}
    @Override protected void onResume(){super.onResume();if(web!=null)web.onResume();}
    @Override protected void onDestroy(){if(web!=null){web.removeJavascriptInterface("InhouseNative");web.destroy();}super.onDestroy();}
}
