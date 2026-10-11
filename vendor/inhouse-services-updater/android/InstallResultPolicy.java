package com.inhouse.services.updater;

/** Version evidence takes precedence over an installer Activity result. */
public final class InstallResultPolicy {
    private InstallResultPolicy() {}

    public static String reconcile(long expected, long actual, String status, boolean differentProcess) {
        if (expected > 0 && actual >= expected) return "installed";
        if ("installed".equals(status)) return "error";
        if ("installing".equals(status) && differentProcess) return "error";
        return status;
    }

    public static String callback(long expected, long actual, int resultCode) {
        if (expected > 0 && actual >= expected) return "installed";
        // RESULT_OK is not sufficient evidence: verify the actual package.
        return resultCode == 0 ? "cancelled" : "error";
    }

    public static String failureMessage(int packageResult) {
        switch (packageResult) {
            case -4: return "Android needs more free storage to install this update.";
            case -7: return "Android rejected the update signature.";
            case -25: return "Android refused to install an older version.";
            case -111: return "Android blocked this installation. Check device restrictions.";
            case -2: return "Android could not read the update package. Try again.";
            default: return "Android could not install the update. Try again.";
        }
    }
}
