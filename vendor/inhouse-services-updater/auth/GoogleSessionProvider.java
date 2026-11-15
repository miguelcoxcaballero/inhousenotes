package com.inhouse.services.oauth;

import android.content.ContentProvider;
import android.content.ContentValues;
import android.content.Context;
import android.content.pm.PackageManager;
import android.content.pm.Signature;
import android.database.Cursor;
import android.net.Uri;
import android.os.Binder;
import android.os.Bundle;
import android.security.keystore.KeyGenParameterSpec;
import android.security.keystore.KeyProperties;
import android.util.AtomicFile;
import android.util.Base64;
import java.io.File;
import java.io.FileOutputStream;
import java.security.KeyStore;
import java.security.MessageDigest;
import java.util.Arrays;
import java.util.LinkedHashMap;
import java.util.Map;
import javax.crypto.Cipher;
import javax.crypto.KeyGenerator;
import javax.crypto.SecretKey;
import javax.crypto.spec.GCMParameterSpec;
import org.json.JSONObject;

/** Private encrypted grants; only pinned official Inhouse APKs may call this provider. */
public final class GoogleSessionProvider extends ContentProvider {
    static final Map<String, String> TRUSTED = new LinkedHashMap<>();
    static {
        TRUSTED.put("com.local.inhousenotes", "856ad31d017304346de582a5720000a9bb12322701e8a7c56c046ba5e635a957");
        TRUSTED.put("com.inhousesoftware.read", "15a1f89c7a6fe7476bee6eaf48cc78239b5a5cb05f98daa4d3c02a6bc73284bd");
        TRUSTED.put("com.inhouse.paellas", "7133a4b3e4b9f9c4d2fbbd38b7dd2d44c7b8d0d7d16568bfe3da74a09e7887b6");
    }
    static boolean trusted(Context context, String name) {
        try {
            String expected = TRUSTED.get(name);
            if (expected == null) return false;
            Signature[] signatures = context.getPackageManager().getPackageInfo(name, PackageManager.GET_SIGNATURES).signatures;
            if (signatures.length != 1) return false;
            byte[] hash = MessageDigest.getInstance("SHA-256").digest(signatures[0].toByteArray());
            StringBuilder actual = new StringBuilder();
            for (byte value : hash) actual.append(String.format("%02x", value & 255));
            return expected.equals(actual.toString());
        } catch (Exception error) { return false; }
    }
    private void authorize() {
        String[] names = getContext().getPackageManager().getPackagesForUid(Binder.getCallingUid());
        if (names == null || names.length != 1 || !trusted(getContext(), names[0]))
            throw new SecurityException("Untrusted Google session caller");
    }
    @Override public boolean onCreate() { return true; }
    @Override public Bundle call(String method, String arg, Bundle extras) {
        authorize();
        if (!"read".equals(method)) throw new SecurityException("Read-only session provider");
        Bundle result = new Bundle(); result.putString("session", read(getContext())); return result;
    }
    private static SecretKey key() throws Exception {
        KeyStore store = KeyStore.getInstance("AndroidKeyStore"); store.load(null);
        String alias = "inhouse-google-session-v1";
        if (!store.containsAlias(alias)) {
            KeyGenerator generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore");
            generator.init(new KeyGenParameterSpec.Builder(alias, KeyProperties.PURPOSE_ENCRYPT | KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM).setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE).build());
            generator.generateKey();
        }
        return (SecretKey) store.getKey(alias, null);
    }
    private static AtomicFile file(Context context) {
        return new AtomicFile(new File(context.getNoBackupFilesDir(), "inhouse-google-session-v1"));
    }
    static synchronized String read(Context context) {
        try {
            byte[] bytes = file(context).readFully();
            Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding");
            cipher.init(Cipher.DECRYPT_MODE, key(), new GCMParameterSpec(128, Arrays.copyOfRange(bytes, 0, 12)));
            return new String(cipher.doFinal(Arrays.copyOfRange(bytes, 12, bytes.length)), "UTF-8");
        } catch (Exception error) { return "null"; }
    }
    static synchronized void write(Context context, String value) throws Exception {
        if (value == null) { file(context).delete(); return; }
        if (value.length() > 32768) throw new IllegalArgumentException("Session too large");
        JSONObject session = new JSONObject(value);
        if (!session.has("access_token") || !session.has("scope") || !session.has("client_id"))
            throw new IllegalArgumentException("Invalid Google session");
        Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding"); cipher.init(Cipher.ENCRYPT_MODE, key());
        AtomicFile target = file(context); FileOutputStream output = target.startWrite();
        try {
            output.write(cipher.getIV()); output.write(cipher.doFinal(value.getBytes("UTF-8"))); target.finishWrite(output);
        } catch (Exception error) { target.failWrite(output); throw error; }
    }
    static String discover(Context context) {
        JSONObject best = null;
        for (String name : TRUSTED.keySet()) {
            if (!trusted(context, name)) continue;
            try {
                String raw;
                if (name.equals(context.getPackageName())) raw = read(context);
                else {
                    Bundle result = context.getContentResolver().call(Uri.parse("content://" + name + ".inhouse-google-session"), "read", null, null);
                    raw = result == null ? "null" : result.getString("session", "null");
                }
                JSONObject candidate = new JSONObject(raw);
                if (best == null || candidate.optLong("updatedAt") > best.optLong("updatedAt")) best = candidate;
            } catch (Exception ignored) { /* App not installed, not upgraded, or grant unavailable. */ }
        }
        return best == null ? "null" : best.toString();
    }
    @Override public Cursor query(Uri uri,String[] p,String s,String[] a,String o) { throw new SecurityException(); }
    @Override public String getType(Uri uri) { return null; }
    @Override public Uri insert(Uri uri,ContentValues v) { throw new SecurityException(); }
    @Override public int delete(Uri uri,String s,String[] a) { throw new SecurityException(); }
    @Override public int update(Uri uri,ContentValues v,String s,String[] a) { throw new SecurityException(); }
}
