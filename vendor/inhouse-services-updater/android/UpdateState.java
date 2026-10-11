package com.inhouse.services.updater;

import android.content.Context;
import android.os.Build;
import org.json.JSONObject;
import java.io.ByteArrayOutputStream;
import java.io.InputStream;

/** Shared persistent policy. Edit in inhouse-services-updater and sync every app.
 * A is the default; B requires explicit publication. All prompts are optional. */
public final class UpdateState {
    private static JSONObject config;
    public static synchronized JSONObject config(Context context) {
        if(config==null)try(InputStream input=context.getAssets().open("updates/config.json")) {
            ByteArrayOutputStream bytes=new ByteArrayOutputStream();byte[] buffer=new byte[4096];int n;
            while((n=input.read(buffer))!=-1)bytes.write(buffer,0,n);
            config=new JSONObject(bytes.toString("UTF-8"));
        }catch(Exception error){config=new JSONObject();}
        return config;
    }
    public static String version(Context context){
        String value=config(context).optString("installedVersion","");if(!value.isEmpty())return value;
        try{return context.getPackageManager().getPackageInfo(context.getPackageName(),0).versionName;}catch(Exception ignored){return "0.0.0";}
    }
    public static long versionCode(Context context){
        try{android.content.pm.PackageInfo info=context.getPackageManager().getPackageInfo(context.getPackageName(),0);return Build.VERSION.SDK_INT>=28?info.getLongVersionCode():info.versionCode;}catch(Exception ignored){return 0;}
    }
    public static String read(Context context){return context.getSharedPreferences("inhouse-services-updater",0).getString("state","{\"channel\":\"A\",\"snooze\":{}}");}
    public static void save(Context context,String value){
        try{
            JSONObject state=new JSONObject(value);String channel=state.getString("channel");if(!channel.equals("A")&&!channel.equals("B"))return;
            JSONObject snooze=state.optJSONObject("snooze");if(snooze==null)snooze=new JSONObject();
            JSONObject normalized=new JSONObject().put("channel",channel).put("snooze",new JSONObject().put("A",Math.max(0,snooze.optLong("A",0))).put("B",Math.max(0,snooze.optLong("B",0))));
            if(!context.getSharedPreferences("inhouse-services-updater",0).edit().putString("state",normalized.toString()).commit())throw new IllegalStateException("Could not save update preferences");
        }catch(org.json.JSONException ignored){}
    }
    public static void snooze(Context context){try{JSONObject state=new JSONObject(read(context));JSONObject times=state.optJSONObject("snooze");if(times==null)times=new JSONObject();times.put(state.optString("channel","A"),System.currentTimeMillis()+14L*86400000);state.put("snooze",times);save(context,state.toString());}catch(Exception ignored){}}
    public static boolean accepts(Context context,String catalog){
        try{JSONObject state=new JSONObject(read(context)),data=new JSONObject(catalog);String channel=state.optString("channel","A");JSONObject times=state.optJSONObject("snooze");JSONObject latest=data.optJSONObject("latest");return channel.equals(data.optString("channel"))&&(times==null||times.optLong(channel,0)<=System.currentTimeMillis())&&latest!=null&&latest.optLong("versionCode",0)>versionCode(context);}catch(Exception ignored){return false;}
    }
    public static String offerKey(Context context,String catalog){try{JSONObject state=new JSONObject(read(context)),data=new JSONObject(catalog);return state.optString("channel","A")+":"+data.getJSONObject("latest").optLong("versionCode")+":"+state.optJSONObject("snooze");}catch(Exception ignored){return catalog;}}
}
