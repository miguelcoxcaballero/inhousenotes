package com.inhouse.services.updater;

import android.content.Context;
import android.content.Intent;
import android.net.Uri;
import android.webkit.*;
import java.io.*;

/** Serve only bundled updater assets in the isolated update WebViews. */
public class UpdateAssetClient extends WebViewClient {
    private final Context context;
    public UpdateAssetClient(Context context){this.context=context;}
    @Override public boolean shouldOverrideUrlLoading(WebView view,WebResourceRequest request){
        Uri uri=request.getUrl();
        // Never navigate the installer WebView to external content. Only user-clicked footer links leave the app.
        if(request.isForMainFrame()&&request.hasGesture()&&"https".equals(uri.getScheme())&&
           ("github.com".equals(uri.getHost())||"inhouse.miguelcoxcaballero.com".equals(uri.getHost()))) {
            try{context.startActivity(new Intent(Intent.ACTION_VIEW,uri).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));}catch(Exception ignored){}
        }
        return true;
    }
    @Override public WebResourceResponse shouldInterceptRequest(WebView view,WebResourceRequest request){
        Uri uri=request.getUrl();
        if(!"https".equals(uri.getScheme()) || !"appassets.androidplatform.net".equals(uri.getHost()))return null;
        String path=uri.getPath();
        if(path==null || !path.startsWith("/updates/") || path.contains(".."))return empty();
        try{
            String type=path.endsWith(".js")?"application/javascript":path.endsWith(".css")?"text/css":"text/html";
            return new WebResourceResponse(type,"UTF-8",context.getAssets().open(path.substring(1)));
        }catch(IOException error){return empty();}
    }
    private WebResourceResponse empty(){return new WebResourceResponse("text/plain","UTF-8",new ByteArrayInputStream(new byte[0]));}
}
